//! HTTP header utilities.
//!
//! Provider headers are layered — provider settings, then per-call headers,
//! then authentication — and later layers win. A layer entry of `None` means
//! "remove this header" (the AI SDK's `undefined` header value).

use std::collections::HashMap;

use crate::resolvable::Resolvable;

/// A header layer: `None` removes the header from earlier layers.
pub type HeaderMapOpt = HashMap<String, Option<String>>;

/// Headers supplied directly or computed per request (token refresh, …).
pub type HeadersFn = Resolvable<HeaderMapOpt>;

/// Append a suffix to the case-insensitive `user-agent` header.
pub fn with_user_agent_suffix(headers: &mut HashMap<String, String>, suffix: &str) {
    let layer = headers
        .drain()
        .map(|(name, value)| (name, Some(value)))
        .collect();
    *headers = normalize_headers(layer).into_iter().collect();
    let value = user_agent_value(headers.get("user-agent").map_or("", String::as_str), suffix);
    headers.insert("user-agent".to_string(), value);
}

pub(crate) fn user_agent_value(current: &str, suffix: &str) -> String {
    match (current.is_empty(), suffix.is_empty()) {
        (true, true) => String::new(),
        (true, false) => suffix.to_owned(),
        (false, true) => current.to_owned(),
        (false, false) => {
            let mut out = String::with_capacity(current.len() + 1 + suffix.len());
            out.push_str(current);
            out.push(' ');
            out.push_str(suffix);
            out
        }
    }
}

/// Merge header layers, earliest first. Names are lowercased; a later layer
/// overrides an earlier one (never appends), including with `None`, which
/// removes the header. Entries of one layer whose names differ only by case
/// are applied in byte order of the original name, so the result does not
/// depend on hash-map iteration order.
#[must_use]
pub fn combine_headers(layers: &[&HeaderMapOpt]) -> HeaderMapOpt {
    let mut combined = HeaderMapOpt::new();
    for layer in layers {
        let mut entries: Vec<_> = layer.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (name, value) in entries {
            combined.insert(name.to_ascii_lowercase(), value.clone());
        }
    }
    combined
}

/// Turn a header layer into the list that goes on the wire: names lowercased,
/// `None` entries dropped, one entry per name (when names differ only by case
/// the one that sorts last wins), sorted by name.
#[must_use]
pub fn normalize_headers(headers: HeaderMapOpt) -> Vec<(String, String)> {
    let mut entries: Vec<_> = headers.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut unique: std::collections::BTreeMap<String, Option<String>> =
        std::collections::BTreeMap::new();
    for (name, value) in entries {
        unique.insert(name.to_ascii_lowercase(), value);
    }
    unique
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name, value)))
        .collect()
}

/// Merge layers without allocating normalized names or copying header values
/// before inserting them into the transport map. Validate only the winners:
/// an overridden or deleted invalid value never reached the wire before.
pub(crate) fn merge_header_layers(
    layers: &[HeaderMapOpt],
    allow_name: impl Fn(&str) -> bool,
) -> Result<http::HeaderMap, aimux_core::AiMuxError> {
    let mut entries = Vec::with_capacity(layers.iter().map(HeaderMapOpt::len).sum());
    for (layer, headers) in layers.iter().enumerate() {
        entries.extend(headers.iter().filter_map(|(name, value)| {
            allow_name(name).then_some((name.as_str(), layer, value.as_deref()))
        }));
    }
    entries.sort_unstable_by(|(a, al, _), (b, bl, _)| {
        a.bytes()
            .map(|byte| byte.to_ascii_lowercase())
            .cmp(b.bytes().map(|byte| byte.to_ascii_lowercase()))
            .then_with(|| al.cmp(bl))
            .then_with(|| a.cmp(b))
    });
    let mut headers = http::HeaderMap::new();
    let mut entries = entries.into_iter().peekable();
    while let Some((name, _, value)) = entries.next() {
        if entries
            .peek()
            .is_some_and(|next| name.eq_ignore_ascii_case(next.0))
        {
            continue;
        }
        if let Some(value) = value {
            insert_header(&mut headers, name, value).map_err(|error| match error {
                aimux_core::AiMuxError::InvalidArgument { message, cause } => {
                    aimux_core::AiMuxError::InvalidArgument {
                        message: message.to_ascii_lowercase(),
                        cause,
                    }
                }
                error => error,
            })?;
        }
    }
    Ok(headers)
}

pub(crate) fn insert_header(
    headers: &mut http::HeaderMap,
    name: &str,
    value: &str,
) -> Result<(), aimux_core::AiMuxError> {
    use aimux_core::{AiMuxError, ErrorCause};

    let header_name =
        http::HeaderName::try_from(name).map_err(|error| AiMuxError::InvalidArgument {
            message: format!("invalid header name: {name}"),
            cause: Some(ErrorCause::source(&error)),
        })?;
    // Values are commonly credentials and must not be echoed on failure.
    let header_value =
        http::HeaderValue::try_from(value).map_err(|error| AiMuxError::InvalidArgument {
            message: format!("invalid header value for {name}"),
            cause: Some(ErrorCause::source(&error)),
        })?;
    headers.insert(header_name, header_value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_layers_match_normalization_with_collisions_deletions_and_overrides() {
        let first = [
            ("z-last", Some("last")),
            ("AUTHORIZATION", Some("malformed\nvalue")),
            ("X-Case", Some("upper")),
            ("x-case", Some("lower")),
            ("User-Agent", Some("base")),
        ];
        let second = [
            ("authorization", None),
            ("x-CASE", Some("call")),
            ("User-Agent", Some("ignored")),
            ("user-agent", None),
            ("a-first", Some("first")),
        ];
        let to_layer = |items: &[(&str, Option<&str>)]| {
            items
                .iter()
                .map(|(k, v)| (k.to_string(), v.map(str::to_owned)))
                .collect()
        };
        let layers = [to_layer(&first), to_layer(&second)];
        let reversed = [
            to_layer(&first.into_iter().rev().collect::<Vec<_>>()),
            to_layer(&second.into_iter().rev().collect::<Vec<_>>()),
        ];
        let expected = normalize_headers(combine_headers(&[&layers[0], &layers[1]]));
        for layers in [&layers, &reversed] {
            let wire = merge_header_layers(layers, |_| true).unwrap();
            let actual: Vec<_> = wire
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_owned()))
                .collect();
            assert_eq!(actual, expected);
        }
        assert_eq!(
            expected,
            [
                ("a-first".to_owned(), "first".to_owned()),
                ("x-case".to_owned(), "call".to_owned()),
                ("z-last".to_owned(), "last".to_owned()),
            ]
        );
    }
}
