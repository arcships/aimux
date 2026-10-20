//! `toCamelCase` (`to-camel-case.ts`).

/// `snake_case` / `kebab-case` provider name to `camelCase` (the AI SDK's
/// `toCamelCase`).
#[must_use]
pub(crate) fn to_camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars().peekable();
    while let Some(ch) = chars.next() {
        match chars.peek() {
            Some(next) if (ch == '_' || ch == '-') && next.is_ascii_lowercase() => {
                out.push(next.to_ascii_uppercase());
                chars.next();
            }
            _ => out.push(ch),
        }
    }
    out
}
