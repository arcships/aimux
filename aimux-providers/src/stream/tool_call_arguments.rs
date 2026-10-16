//! Incremental structure tracking for streamed tool-call arguments.
//!
//! Port of `@ai-sdk/provider-utils`'s `streaming-tool-call-argument-state.ts`.
//! The state is intentionally *structural* rather than a JSON parse: a
//! currently parsable scalar can still be the prefix of a later value, but a
//! closed top-level `{…}` / `[…]` cannot be extended.

/// Whether `value` (ignoring leading whitespace) starts with `{` or `[`.
#[must_use]
pub(super) fn starts_with_structured_value(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim_start().as_bytes().first(), Some(b'{' | b'[')))
}

/// Incrementally tracks whether streamed tool-call arguments contain a
/// complete structured JSON value.
#[derive(Debug, Clone)]
pub(super) struct StreamingToolCallArgumentState {
    structure: Structure,
}

#[derive(Debug, Clone)]
enum Structure {
    /// Only whitespace so far.
    Undetermined,
    /// Not a structured value (a scalar, or a mismatched bracket).
    Other,
    /// Inside a top-level `{…}` / `[…]`; `stack` holds the open brackets.
    Open {
        stack: Vec<u8>,
        in_string: bool,
        escaped: bool,
    },
    /// The top-level value has been closed.
    Complete,
}

impl StreamingToolCallArgumentState {
    /// Create a state seeded with `initial_value`.
    #[must_use]
    pub(super) fn new(initial_value: &str) -> Self {
        let mut state = Self {
            structure: Structure::Undetermined,
        };
        state.append(initial_value);
        state
    }

    /// `true` once a top-level `{…}` / `[…]` value has been closed.
    #[must_use]
    pub(super) fn has_complete_structured_value(&self) -> bool {
        matches!(self.structure, Structure::Complete)
    }

    /// Feed the next argument fragment.
    pub(super) fn append(&mut self, delta: &str) {
        let mut rest = delta;
        if matches!(self.structure, Structure::Undetermined) {
            rest = rest.trim_start();
            match rest.as_bytes().first() {
                None => return,
                Some(b'{' | b'[') => {
                    self.structure = Structure::Open {
                        stack: Vec::new(),
                        in_string: false,
                        escaped: false,
                    };
                }
                Some(_) => {
                    self.structure = Structure::Other;
                    return;
                }
            }
        }

        let Structure::Open {
            stack,
            in_string,
            escaped,
        } = &mut self.structure
        else {
            return;
        };

        // Every structural character is ASCII and UTF-8 continuation bytes
        // never collide with ASCII, so scanning bytes is exact.
        for &byte in rest.as_bytes() {
            if *in_string {
                if *escaped {
                    *escaped = false;
                } else if byte == b'\\' {
                    *escaped = true;
                } else if byte == b'"' {
                    *in_string = false;
                }
                continue;
            }
            match byte {
                b'"' => *in_string = true,
                b'{' | b'[' => stack.push(byte),
                b'}' | b']' => {
                    let opening = if byte == b'}' { b'{' } else { b'[' };
                    if stack.pop() != Some(opening) {
                        self.structure = Structure::Other;
                        return;
                    }
                    if stack.is_empty() {
                        self.structure = Structure::Complete;
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Port of `streaming-tool-call-argument-state.test.ts` from
/// `@ai-sdk/provider-utils`.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_structured_value_true() {
        for value in ["{}", "  {", "[]", "\n["] {
            assert!(starts_with_structured_value(Some(value)), "{value:?}");
        }
    }

    #[test]
    fn starts_with_structured_value_false() {
        // TS covers `undefined` and `null` separately; Rust has one `None`.
        assert!(!starts_with_structured_value(None));
        for value in ["", "   ", "1", "\"value\""] {
            assert!(!starts_with_structured_value(Some(value)), "{value:?}");
        }
    }

    #[test]
    fn tracks_a_structured_value_across_deltas() {
        let mut state = StreamingToolCallArgumentState::new("  {\"value\":");
        assert!(!state.has_complete_structured_value());

        state.append("1}");
        assert!(state.has_complete_structured_value());
    }

    #[test]
    fn tracks_nested_objects_and_arrays() {
        let state = StreamingToolCallArgumentState::new("[{\"value\":{\"items\":[1,2]}}]");
        assert!(state.has_complete_structured_value());
    }

    #[test]
    fn ignores_structural_characters_inside_strings() {
        let state = StreamingToolCallArgumentState::new("{\"value\":\"braces: } ] { [\"}");
        assert!(state.has_complete_structured_value());
    }

    #[test]
    fn handles_escaped_quotes_across_deltas() {
        let mut state = StreamingToolCallArgumentState::new("{\"value\":\"escaped quote: \\\"");
        assert!(!state.has_complete_structured_value());

        state.append(" still in string\"}");
        assert!(state.has_complete_structured_value());
    }

    #[test]
    fn can_begin_after_an_empty_or_whitespace_only_delta() {
        let mut state = StreamingToolCallArgumentState::new("  ");

        state.append("[");
        assert!(!state.has_complete_structured_value());

        state.append("]");
        assert!(state.has_complete_structured_value());
    }

    #[test]
    fn does_not_treat_scalar_arguments_as_a_complete_structured_value() {
        let state = StreamingToolCallArgumentState::new("12");
        assert!(!state.has_complete_structured_value());
    }

    #[test]
    fn does_not_recover_mismatched_structures_as_complete() {
        let mut state = StreamingToolCallArgumentState::new("{\"value\":]");
        state.append("}");
        assert!(!state.has_complete_structured_value());
    }
}
