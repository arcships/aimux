//! JSON repair and partial-JSON parsing helpers.
//!
//! Ports of `fix-json.ts` and `parse-partial-json.ts` from the Vercel AI SDK
//! `util` package.

use serde_json::Value;

// ===========================================================================
// fixJson  <- packages/ai/src/util/fix-json.ts
// ===========================================================================

/// Scanner state for [`fix_json`]. Mirrors the `State` union in `fix-json.ts`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FixState {
    Root,
    Finish,
    InsideString,
    InsideStringEscape,
    InsideStringUnicodeEscape,
    InsideLiteral,
    InsideNumber,
    InsideObjectStart,
    InsideObjectKey,
    InsideObjectAfterKey,
    InsideObjectBeforeValue,
    InsideObjectAfterValue,
    InsideObjectAfterComma,
    InsideArrayStart,
    InsideArrayAfterValue,
    InsideArrayAfterComma,
}

/// Single-pass scanner/repairer for partial JSON, ported line-for-line from
/// `fix-json.ts`.
///
/// Given a (possibly truncated) piece of JSON, returns the longest prefix that
/// can be closed into valid JSON by appending the minimal set of closing
/// tokens (`"`, `}`, `]`, or the remainder of an in-progress literal).
struct Fixer<'a> {
    chars: &'a [char],
    stack: Vec<FixState>,
    last_valid_index: isize, // -1 == nothing valid yet
    literal_start: Option<usize>,
    unicode_escape_digits: usize,
}

impl<'a> Fixer<'a> {
    fn new(chars: &'a [char]) -> Self {
        Self {
            chars,
            stack: vec![FixState::Root],
            last_valid_index: -1,
            literal_start: None,
            unicode_escape_digits: 0,
        }
    }

    fn is_hex_digit(c: char) -> bool {
        c.is_ascii_hexdigit()
    }

    /// Port of `processValueStart`.
    fn process_value_start(&mut self, c: char, i: usize, swap_state: FixState) {
        match c {
            '"' => {
                self.last_valid_index = i as isize;
                self.stack.pop();
                self.stack.push(swap_state);
                self.stack.push(FixState::InsideString);
            }
            'f' | 't' | 'n' => {
                self.last_valid_index = i as isize;
                self.literal_start = Some(i);
                self.stack.pop();
                self.stack.push(swap_state);
                self.stack.push(FixState::InsideLiteral);
            }
            '-' => {
                self.stack.pop();
                self.stack.push(swap_state);
                self.stack.push(FixState::InsideNumber);
            }
            '0'..='9' => {
                self.last_valid_index = i as isize;
                self.stack.pop();
                self.stack.push(swap_state);
                self.stack.push(FixState::InsideNumber);
            }
            '{' => {
                self.last_valid_index = i as isize;
                self.stack.pop();
                self.stack.push(swap_state);
                self.stack.push(FixState::InsideObjectStart);
            }
            '[' => {
                self.last_valid_index = i as isize;
                self.stack.pop();
                self.stack.push(swap_state);
                self.stack.push(FixState::InsideArrayStart);
            }
            // Unmatched characters do nothing (matches the TS switch which has
            // no default arm).
            _ => {}
        }
    }

    /// Port of `processAfterObjectValue`.
    fn process_after_object_value(&mut self, c: char, i: usize) {
        match c {
            ',' => {
                self.stack.pop();
                self.stack.push(FixState::InsideObjectAfterComma);
            }
            '}' => {
                self.last_valid_index = i as isize;
                self.stack.pop();
            }
            _ => {}
        }
    }

    /// Port of `processAfterArrayValue`.
    fn process_after_array_value(&mut self, c: char, i: usize) {
        match c {
            ',' => {
                self.stack.pop();
                self.stack.push(FixState::InsideArrayAfterComma);
            }
            ']' => {
                self.last_valid_index = i as isize;
                self.stack.pop();
            }
            _ => {}
        }
    }

    fn run(&mut self) {
        for (i, &c) in self.chars.iter().enumerate() {
            let current = *self
                .stack
                .last()
                .expect("fix-json stack is never empty (seeded with Root)");

            match current {
                FixState::Root => self.process_value_start(c, i, FixState::Finish),

                FixState::InsideObjectStart => match c {
                    '"' => {
                        self.stack.pop();
                        self.stack.push(FixState::InsideObjectKey);
                    }
                    '}' => {
                        self.last_valid_index = i as isize;
                        self.stack.pop();
                    }
                    _ => {}
                },

                FixState::InsideObjectAfterComma => {
                    if c == '"' {
                        self.stack.pop();
                        self.stack.push(FixState::InsideObjectKey);
                    }
                }

                FixState::InsideObjectKey => {
                    if c == '"' {
                        self.stack.pop();
                        self.stack.push(FixState::InsideObjectAfterKey);
                    }
                }

                FixState::InsideObjectAfterKey => {
                    if c == ':' {
                        self.stack.pop();
                        self.stack.push(FixState::InsideObjectBeforeValue);
                    }
                }

                FixState::InsideObjectBeforeValue => {
                    self.process_value_start(c, i, FixState::InsideObjectAfterValue);
                }

                FixState::InsideObjectAfterValue => {
                    self.process_after_object_value(c, i);
                }

                FixState::InsideString => match c {
                    '"' => {
                        self.stack.pop();
                        self.last_valid_index = i as isize;
                    }
                    '\\' => {
                        self.stack.push(FixState::InsideStringEscape);
                    }
                    _ => {
                        self.last_valid_index = i as isize;
                    }
                },

                FixState::InsideArrayStart => match c {
                    ']' => {
                        self.last_valid_index = i as isize;
                        self.stack.pop();
                    }
                    _ => {
                        self.last_valid_index = i as isize;
                        self.process_value_start(c, i, FixState::InsideArrayAfterValue);
                    }
                },

                FixState::InsideArrayAfterValue => match c {
                    ',' => {
                        self.stack.pop();
                        self.stack.push(FixState::InsideArrayAfterComma);
                    }
                    ']' => {
                        self.last_valid_index = i as isize;
                        self.stack.pop();
                    }
                    _ => {
                        self.last_valid_index = i as isize;
                    }
                },

                FixState::InsideArrayAfterComma => {
                    self.process_value_start(c, i, FixState::InsideArrayAfterValue);
                }

                FixState::InsideStringEscape => {
                    self.stack.pop();
                    if c == 'u' {
                        self.unicode_escape_digits = 0;
                        self.stack.push(FixState::InsideStringUnicodeEscape);
                    } else {
                        self.last_valid_index = i as isize;
                    }
                }

                FixState::InsideStringUnicodeEscape => {
                    if Self::is_hex_digit(c) {
                        self.unicode_escape_digits += 1;
                        if self.unicode_escape_digits == 4 {
                            self.stack.pop();
                            self.last_valid_index = i as isize;
                        }
                    }
                }

                FixState::InsideNumber => match c {
                    '0'..='9' => {
                        self.last_valid_index = i as isize;
                    }
                    'e' | 'E' | '-' | '.' => {}
                    ',' => {
                        self.stack.pop();
                        if self.stack.last() == Some(&FixState::InsideArrayAfterValue) {
                            self.process_after_array_value(c, i);
                        }
                        if self.stack.last() == Some(&FixState::InsideObjectAfterValue) {
                            self.process_after_object_value(c, i);
                        }
                    }
                    '}' => {
                        self.stack.pop();
                        if self.stack.last() == Some(&FixState::InsideObjectAfterValue) {
                            self.process_after_object_value(c, i);
                        }
                    }
                    ']' => {
                        self.stack.pop();
                        if self.stack.last() == Some(&FixState::InsideArrayAfterValue) {
                            self.process_after_array_value(c, i);
                        }
                    }
                    _ => {
                        self.stack.pop();
                    }
                },

                FixState::InsideLiteral => {
                    let start = self
                        .literal_start
                        .expect("literal_start is set whenever INSIDE_LITERAL is pushed");
                    let partial: String = self.chars[start..=i].iter().collect();
                    if !"false".starts_with(&partial)
                        && !"true".starts_with(&partial)
                        && !"null".starts_with(&partial)
                    {
                        self.stack.pop();
                        match self.stack.last() {
                            Some(FixState::InsideObjectAfterValue) => {
                                self.process_after_object_value(c, i);
                            }
                            Some(FixState::InsideArrayAfterValue) => {
                                self.process_after_array_value(c, i);
                            }
                            _ => {}
                        }
                    } else {
                        self.last_valid_index = i as isize;
                    }
                }

                // FINISH (and any unhandled state): the TS switch has no case
                // for FINISH, so trailing characters after the top-level value
                // completes are ignored.
                FixState::Finish => {}
            }
        }
    }

    fn finish(self) -> String {
        let mut result: String = if self.last_valid_index >= 0 {
            self.chars[..=(self.last_valid_index as usize)]
                .iter()
                .collect()
        } else {
            String::new()
        };

        // Walk the stack top-to-bottom, appending the minimal closing tokens.
        // Mirrors the second `for` loop in `fix-json.ts`.
        for state in self.stack.iter().rev() {
            match state {
                FixState::InsideString => result.push('"'),
                FixState::InsideObjectKey
                | FixState::InsideObjectAfterKey
                | FixState::InsideObjectAfterComma
                | FixState::InsideObjectStart
                | FixState::InsideObjectBeforeValue
                | FixState::InsideObjectAfterValue => result.push('}'),
                FixState::InsideArrayStart
                | FixState::InsideArrayAfterComma
                | FixState::InsideArrayAfterValue => result.push(']'),
                FixState::InsideLiteral => {
                    let start = self
                        .literal_start
                        .expect("literal_start is set whenever INSIDE_LITERAL is pushed");
                    let partial: String = self.chars[start..].iter().collect();
                    let plen = partial.chars().count();
                    if "true".starts_with(&partial) {
                        result.push_str(&"true"[plen..]);
                    } else if "false".starts_with(&partial) {
                        result.push_str(&"false"[plen..]);
                    } else if "null".starts_with(&partial) {
                        result.push_str(&"null"[plen..]);
                    }
                }
                // ROOT / FINISH / INSIDE_NUMBER / *_ESCAPE / *_UNICODE_ESCAPE
                // have no closing token in the TS source.
                _ => {}
            }
        }

        result
    }
}

/// Repairs a (possibly partial) JSON string into valid JSON.
///
/// Port of `fixJson` in `packages/ai/src/util/fix-json.ts`.
pub fn fix_json(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut fixer = Fixer::new(&chars);
    fixer.run();
    fixer.finish()
}

// ===========================================================================
// parsePartialJson  <- packages/ai/src/util/parse-partial-json.ts
// ===========================================================================

/// Outcome category returned by [`parse_partial_json`]. Mirrors the `state`
/// union in `parse-partial-json.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsePartialJsonState {
    /// `undefined` was passed as input.
    UndefinedInput,
    /// The input parsed as-is, without repair.
    SuccessfulParse,
    /// The input only parsed after running [`fix_json`].
    RepairedParse,
    /// The input could not be parsed even after repair.
    FailedParse,
}

/// Result of [`parse_partial_json`].
#[derive(Debug, Clone, PartialEq)]
pub struct ParsePartialJsonResult {
    pub value: Option<Value>,
    pub state: ParsePartialJsonState,
}

/// Best-effort parse of a (possibly partial) JSON string.
///
/// Port of `parsePartialJson` in `packages/ai/src/util/parse-partial-json.ts`.
///
/// The TS original is `async` only because its `safeParseJSON` helper is
/// async; the Rust port is synchronous (it uses `serde_json::from_str` as the
/// equivalent of `safeParseJSON`).
pub fn parse_partial_json(json_text: Option<&str>) -> ParsePartialJsonResult {
    let Some(text) = json_text else {
        return ParsePartialJsonResult {
            value: None,
            state: ParsePartialJsonState::UndefinedInput,
        };
    };

    // First try: parse the raw text (== `safeParseJSON({ text })`).
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return ParsePartialJsonResult {
            value: Some(value),
            state: ParsePartialJsonState::SuccessfulParse,
        };
    }

    // Second try: repair then parse (== `safeParseJSON({ text: fixJson(text) })`).
    let fixed = fix_json(text);
    if let Ok(value) = serde_json::from_str::<Value>(&fixed) {
        return ParsePartialJsonResult {
            value: Some(value),
            state: ParsePartialJsonState::RepairedParse,
        };
    }

    ParsePartialJsonResult {
        value: None,
        state: ParsePartialJsonState::FailedParse,
    }
}
