//! SSRF guard for downloads of provider-supplied URLs.
//!
//! Providers return asset URLs, polling URLs, and result URLs in response
//! bodies; fetching them blindly lets a compromised or spoofed response steer
//! authenticated requests at internal services (cloud metadata, loopback,
//! RFC1918 space). This module validates such URLs, resolves and validates
//! every DNS answer, and hands the validated addresses back so the transport
//! can pin the actual connection to them (defeating TTL-0 DNS rebinding).
//! The address policy mirrors AI SDK's `validateUrl` blocklists.

use std::net::IpAddr;

use aimux_core::error::AiMuxError;

/// Compare scheme, host, and effective port. Unparseable inputs are never
/// same-origin. Public so providers can build credential allowlists on top
/// of it (AI SDK's `isSameOrigin`).
#[must_use]
pub fn same_origin(url: &str, origin: &str) -> bool {
    let (Ok(url), Ok(origin)) = (url::Url::parse(url), url::Url::parse(origin)) else {
        return false;
    };
    url.scheme() == origin.scheme()
        && url.host_str() == origin.host_str()
        && url.port_or_known_default() == origin.port_or_known_default()
}

/// Trust flows only within the trusted origin: the exemption applies to a
/// redirect target only when the redirecting URL is itself on the trusted
/// origin, so a foreign hop cannot launder a request into it.
pub(crate) fn hop_trusted_origin<'a>(
    trusted_origin: Option<&'a str>,
    current_url: &str,
) -> Option<&'a str> {
    trusted_origin.filter(|origin| same_origin(current_url, origin))
}

fn without_query(parsed: &url::Url) -> String {
    let mut redacted = parsed.clone();
    redacted.set_query(None);
    redacted.to_string()
}

/// Parse a host as `url::Url::host_str` yields it into an IP literal. The url
/// crate keeps IPv6 hosts bracketed (and may render mapped addresses in hex
/// form, e.g. "[::ffff:7f00:1]"), so brackets are stripped before parsing.
fn host_ip_literal(host: &str) -> Option<IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

/// Syntactic checks: scheme, disallowed hostnames, and literal-IP publicness.
///
/// Callers that resolve DNS afterwards rely on this having rejected every
/// non-public literal, so keep this the single literal-check site.
pub(crate) fn validate_download_url(url: &str) -> Result<url::Url, AiMuxError> {
    let parsed = url::Url::parse(url)
        .map_err(|error| AiMuxError::InvalidArgument(format!("invalid download URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AiMuxError::InvalidArgument(format!(
            "download URL must use HTTP or HTTPS: {}",
            without_query(&parsed)
        )));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AiMuxError::InvalidArgument("download URL has no host".into()))?;
    let normalized_host = host.to_ascii_lowercase();
    let normalized_host = normalized_host.trim_end_matches('.');
    if normalized_host == "localhost"
        || normalized_host.ends_with(".local")
        || normalized_host.ends_with(".localhost")
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "download URL targets a disallowed hostname: {normalized_host}"
        )));
    }
    if let Some(address) = host_ip_literal(normalized_host)
        && !is_public_download_address(address)
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "download URL targets a non-public address: {address}"
        )));
    }
    Ok(parsed)
}

/// Validate a download URL and resolve its host, returning the DNS answers
/// that passed the guard so the connection can be pinned to them.
///
/// A URL same-origin with `trusted_origin` (normally the configured
/// `base_url`) is exempt and returns no addresses; self-hosted deployments
/// legitimately serve assets from private space on their own origin.
pub(crate) async fn validate_download_target(
    url: &str,
    trusted_origin: Option<&str>,
) -> Result<Vec<IpAddr>, AiMuxError> {
    if trusted_origin.is_some_and(|origin| same_origin(url, origin)) {
        return Ok(Vec::new());
    }
    let parsed = validate_download_url(url)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| AiMuxError::InvalidArgument("download URL has no host".into()))?;
    if let Some(address) = host_ip_literal(host) {
        // validate_download_url above already rejected non-public literals.
        return Ok(vec![address]);
    }
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| AiMuxError::InvalidArgument("download URL has no known port".into()))?;
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| {
            AiMuxError::InvalidArgument(format!("download URL host could not be resolved: {error}"))
        })?
        .map(|address| address.ip())
        .collect();
    validate_resolved_download_addresses(host, addresses)
}

fn validate_resolved_download_addresses(
    host: &str,
    addresses: Vec<IpAddr>,
) -> Result<Vec<IpAddr>, AiMuxError> {
    if addresses.is_empty() {
        return Err(AiMuxError::InvalidArgument(format!(
            "download URL host did not resolve to an address: {host}"
        )));
    }
    let mut validated = Vec::with_capacity(addresses.len());
    for address in addresses {
        if !is_public_download_address(address) {
            return Err(AiMuxError::InvalidArgument(format!(
                "download URL resolves to a non-public address: {address}"
            )));
        }
        if !validated.contains(&address) {
            validated.push(address);
        }
    }
    Ok(validated)
}

fn is_public_download_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [a, b, c, _] = address.octets();
            !(a == 0
                || a == 10
                || (a == 100 && (64..=127).contains(&b))
                || a == 127
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && matches!(c, 0 | 2))
                || (a == 192 && b == 168)
                || (a == 198 && matches!(b, 18 | 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || a >= 224)
        }
        IpAddr::V6(address) => {
            let groups = address.segments();
            let top_zero = |count: usize| groups[..count].iter().all(|group| *group == 0);
            if (top_zero(7) && matches!(groups[7], 0 | 1))
                || (groups[0] & 0xfe00) == 0xfc00
                || (groups[0] & 0xffc0) == 0xfe80
                || (groups[0] & 0xffc0) == 0xfec0
                || (groups[0] & 0xff00) == 0xff00
                || (groups[0] == 0x2001 && groups[1] == 0x0db8)
                || (groups[0] == 0x3fff && (groups[1] & 0xf000) == 0)
            {
                return false;
            }

            // Transition prefixes that embed an IPv4 address are judged by
            // the embedded IPv4 bits: IPv4-compatible (::a.b.c.d), mapped
            // (::ffff:a.b.c.d), SIIT (::ffff:0:a.b.c.d), and NAT64
            // (64:ff9b::/96, 64:ff9b:1::/48). 6to4/Teredo are deliberately
            // omitted for parity with AI SDK's isPrivateIPv6.
            let embeds_ipv4 = top_zero(6)
                || (top_zero(5) && groups[5] == 0xffff)
                || (top_zero(4) && groups[4] == 0xffff && groups[5] == 0)
                || (groups[0] == 0x0064
                    && groups[1] == 0xff9b
                    && groups[2..6].iter().all(|group| *group == 0))
                || (groups[0] == 0x0064 && groups[1] == 0xff9b && groups[2] == 1);
            if !embeds_ipv4 {
                return true;
            }

            let embedded = std::net::Ipv4Addr::new(
                (groups[6] >> 8) as u8,
                groups[6] as u8,
                (groups[7] >> 8) as u8,
                groups[7] as u8,
            );
            is_public_download_address(IpAddr::V4(embedded))
        }
    }
}

/// Drop hop-by-hop, forwarding, and metadata-service headers from a download
/// request; they leak deployment topology or unlock metadata endpoints when
/// forwarded to a provider-supplied host. Mirrors AI SDK's download header
/// policy (auth headers survive; the redirect loop clears them cross-origin).
pub(crate) fn sanitize_download_headers(headers: &mut Vec<(String, String)>) {
    const BLOCKED: &[&str] = &[
        "connection",
        "keep-alive",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "host",
        "forwarded",
        "proxy-authorization",
        "via",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-real-ip",
        "metadata",
        "metadata-flavor",
        "x-aws-ec2-metadata-token",
        "x-metadata-token",
        "cookie",
        "set-cookie",
    ];
    headers.retain(|(name, _)| {
        !BLOCKED
            .iter()
            .any(|blocked| name.eq_ignore_ascii_case(blocked))
    });
}

/// Drop caller headers except `User-Agent` when a redirect crosses origin.
pub(crate) fn retain_user_agent(headers: &mut Vec<(String, String)>) {
    let user_agent = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        .cloned();
    headers.clear();
    if let Some(header) = user_agent {
        headers.push(header);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(address: &str) -> bool {
        is_public_download_address(address.parse().unwrap())
    }

    #[test]
    fn address_policy_matches_ai_sdk_ranges() {
        for address in [
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "127.255.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "3fff::1",
            "3fff:fff::1",
            "::7f00:1",
            "::ffff:7f00:1",
            "::ffff:0:7f00:1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::a9fe:a9fe",
        ] {
            assert!(!public(address), "{address} must be blocked");
        }

        for address in [
            "8.8.8.8",
            "100.63.0.1",
            "100.128.0.1",
            "172.15.0.1",
            "172.32.0.1",
            "192.0.3.1",
            "198.51.101.1",
            "203.0.114.1",
            "2606:4700::1",
            "3fff:1000::1",
            "::ffff:808:808",
            "64:ff9b::808:808",
        ] {
            assert!(public(address), "{address} must be allowed");
        }
    }

    #[test]
    fn url_policy_matches_ai_sdk_hostname_and_scheme_rules() {
        for url in [
            "http://localhost/file",
            "http://localhost./file",
            "http://myhost.local/file",
            "http://myhost.local./file",
            "http://app.localhost/file",
            "http://app.localhost./file",
            "http://2130706433/file",
            "http://0x7f000001/file",
            "http://0177.0.0.1/file",
            "http://[::127.0.0.1]/file",
            "http://[::ffff:0:127.0.0.1]/file",
            "http://[64:ff9b::169.254.169.254]/file",
            "http://[64:ff9b:1::169.254.169.254]/file",
            "file:///etc/passwd",
            "ftp://example.com/file",
            "data:text/plain;base64,aGVsbG8=",
            "javascript:alert(1)",
        ] {
            assert!(validate_download_url(url).is_err(), "{url} must be blocked");
        }

        for url in [
            "https://example.com/file",
            "https://example.com./file",
            "http://8.8.8.8/file",
        ] {
            validate_download_url(url).unwrap_or_else(|error| {
                panic!("{url} must be allowed: {error:?}");
            });
        }
    }

    #[test]
    fn resolved_address_validation_fails_closed_and_preserves_all_answers() {
        let empty = validate_resolved_download_addresses("empty.example", Vec::new())
            .expect_err("an empty DNS answer must fail closed");
        assert!(
            matches!(empty, AiMuxError::InvalidArgument(ref message) if message.contains("did not resolve"))
        );

        let mixed = validate_resolved_download_addresses(
            "mixed.example",
            vec!["8.8.8.8".parse().unwrap(), "127.0.0.1".parse().unwrap()],
        )
        .expect_err("one private answer must reject the entire DNS result");
        assert!(
            matches!(mixed, AiMuxError::InvalidArgument(ref message) if message.contains("non-public"))
        );

        let all = validate_resolved_download_addresses(
            "public.example",
            vec![
                "8.8.8.8".parse().unwrap(),
                "1.1.1.1".parse().unwrap(),
                "8.8.8.8".parse().unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(
            all,
            vec![
                "8.8.8.8".parse::<IpAddr>().unwrap(),
                "1.1.1.1".parse::<IpAddr>().unwrap()
            ]
        );
    }
}
