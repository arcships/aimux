//! Incremental structure tracking for streamed tool-call arguments.
//!
//! Port of `@ai-sdk/provider-utils`'s `streaming-tool-call-argument-state.ts`.
//! The state is intentionally *structural* rather than a JSON parse: a
//! currently parsable scalar can still be the prefix of a later value, but a
//! closed top-level `{…}` / `[…]` cannot be extended.

/// Whether `value` (ignoring leading whitespace) starts with `{` or `[`.
#[must_use]
pub fn starts_with_structured_value(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim_start().as_bytes().first(), Some(b'{' | b'[')))
}

/// Incrementally tracks whether streamed tool-call arguments contain a
/// complete structured JSON value.
#[derive(Debug, Clone)]
pub struct StreamingToolCallArgumentState {
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
    pub fn new(initial_value: &str) -> Self {
        let mut state = Self {
            structure: Structure::Undetermined,
        };
        state.append(initial_value);
        state
    }

    /// `true` once a top-level `{…}` / `[…]` value has been closed.
    #[must_use]
    pub fn has_complete_structured_value(&self) -> bool {
        matches!(self.structure, Structure::Complete)
    }

    /// Feed the next argument fragment.
    pub fn append(&mut self, delta: &str) {
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
