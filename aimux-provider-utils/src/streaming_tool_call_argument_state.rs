//! Incremental structure tracking for streamed tool-call arguments.
//!
//! Rust translation of `@ai-sdk/provider-utils`'s
//! `StreamingToolCallArgumentState`
//! (`packages/provider-utils/src/streaming-tool-call-argument-state.ts`).
//!
//! The state is intentionally *structural* rather than a JSON parse: a
//! currently parsable scalar can still be the prefix of a later value, but a
//! closed top-level `{…}` / `[…]` cannot be extended.

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArgumentStructure {
    Undetermined,
    Other,
    Structured {
        stack: Vec<char>,
        in_string: bool,
        escaped: bool,
        complete: bool,
    },
}

/// Whether `value` (ignoring leading whitespace) starts with `{` or `[`.
#[must_use]
pub fn starts_with_structured_value(value: Option<&str>) -> bool {
    value
        .and_then(|v| v.trim_start().chars().next())
        .is_some_and(|c| c == '{' || c == '[')
}

/// Incrementally tracks whether streamed tool-call arguments contain a
/// complete structured JSON value.
#[derive(Debug, Clone)]
pub struct StreamingToolCallArgumentState {
    structure: ArgumentStructure,
}

impl StreamingToolCallArgumentState {
    /// Create a state seeded with `initial_value`.
    #[must_use]
    pub fn new(initial_value: &str) -> Self {
        let mut state = Self {
            structure: ArgumentStructure::Undetermined,
        };
        state.append(initial_value);
        state
    }

    /// `true` once a top-level `{…}` / `[…]` value has been closed.
    #[must_use]
    pub fn has_complete_structured_value(&self) -> bool {
        matches!(
            self.structure,
            ArgumentStructure::Structured { complete: true, .. }
        )
    }

    /// Feed the next argument fragment.
    pub fn append(&mut self, delta: &str) {
        for character in delta.chars() {
            match &mut self.structure {
                ArgumentStructure::Undetermined => {
                    if character.is_whitespace() {
                        continue;
                    }
                    self.structure = if character == '{' || character == '[' {
                        ArgumentStructure::Structured {
                            stack: vec![character],
                            in_string: false,
                            escaped: false,
                            complete: false,
                        }
                    } else {
                        ArgumentStructure::Other
                    };
                }
                ArgumentStructure::Other | ArgumentStructure::Structured { complete: true, .. } => {
                }
                ArgumentStructure::Structured {
                    stack,
                    in_string,
                    escaped,
                    complete,
                } => {
                    if *in_string {
                        if *escaped {
                            *escaped = false;
                        } else if character == '\\' {
                            *escaped = true;
                        } else if character == '"' {
                            *in_string = false;
                        }
                        continue;
                    }

                    match character {
                        '"' => *in_string = true,
                        '{' | '[' => stack.push(character),
                        '}' | ']' => {
                            let expected = if character == '}' { '{' } else { '[' };
                            if stack.last() != Some(&expected) {
                                self.structure = ArgumentStructure::Other;
                                continue;
                            }
                            stack.pop();
                            if stack.is_empty() {
                                *complete = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
