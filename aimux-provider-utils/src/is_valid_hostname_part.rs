//! `isValidHostnamePart` (is-valid-hostname-part.ts).

/// Whether `part` is one ASCII DNS label: 1 to 63 letters, digits or hyphens,
/// not starting or ending with a hyphen.
///
/// Check a resource name, region or location with this before putting it into
/// a generated hostname: values such as `evil.example.com/#` or
/// `user@localhost:8080/#` would change the request destination.
#[must_use]
pub fn is_valid_hostname_part(part: &str) -> bool {
    let bytes = part.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
}
