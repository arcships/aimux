//! Rust translation of `packages/provider-utils/src/is-valid-hostname-part.test.ts`
//! (@ai-sdk/provider-utils). Each `it.each` table is one test that checks every
//! row.

use crate::is_valid_hostname_part;

/// upstream: @ai-sdk/provider-utils src/is-valid-hostname-part.test.ts › isValidHostnamePart › accepts %j
#[test]
fn accepts() {
    let max = "a".repeat(63);
    for value in ["a", "0", "us-east-1", "MY-resource", "global", max.as_str()] {
        assert!(is_valid_hostname_part(value), "should accept {value:?}");
    }
}

/// upstream: @ai-sdk/provider-utils src/is-valid-hostname-part.test.ts › isValidHostnamePart › rejects %j
#[test]
fn rejects() {
    let too_long = "a".repeat(64);
    for value in [
        "",
        too_long.as_str(),
        "-a",
        "a-",
        "a.b",
        "a_b",
        "é",
        "user@internal:8080/#",
        "evil.example.com/#",
        "169.254.169.254:80/x#",
        "us-east-1/../..",
        "us east 1",
        "a\n",
        "a\r",
        "a\t",
        "a\0",
        "%61",
    ] {
        assert!(!is_valid_hostname_part(value), "should reject {value:?}");
    }
}
