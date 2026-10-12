//! Differential test: the current `json_repair` against the line-for-line
//! port of `fix-json.ts` / `parse-partial-json.ts` it replaced (kept verbatim
//! in `legacy` below, minus its module doc).
//!
//! `fix_json` must return the same string; `PartialJson::parse` must give the
//! legacy `parse_partial_json` outcome and value, with legacy
//! `UndefinedInput` being `None.map(PartialJson::parse)`.
//!
//! The default run compares 20 000 generated inputs; set
//! `JSON_REPAIR_DIFF_N` to compare more (the rewrite was checked on
//! 1 000 000).

use aimux_core::json_repair::{PartialJson, fix_json};

#[allow(clippy::all)]
mod legacy {

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
    #[must_use]
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
    #[must_use]
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
}

/// xorshift64* — deterministic, no dependency.
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// Raw fragments: every structural character, every escape form, partial and
/// full literals, number pieces, whitespace, multi-byte and astral characters
/// (the UTF-16 trap), prototype keys and out-of-range numbers (the JSON.parse
/// traps), and junk.
const FRAGMENTS: &[&str] = &[
    "{",
    "}",
    "[",
    "]",
    ",",
    ":",
    "\"",
    "\\",
    "\\\"",
    "\\\\",
    "\\n",
    "\\u",
    "\\u00",
    "\\u00e9",
    "\\ud83d",
    "\\ude00",
    "\\uD83D\\uDE00",
    "u",
    "0",
    "1",
    "42",
    "9",
    "-",
    ".",
    "e",
    "E",
    "+",
    "t",
    "tr",
    "true",
    "f",
    "fals",
    "false",
    "n",
    "nul",
    "null",
    "x",
    "a",
    "key",
    " ",
    "  ",
    "\n",
    "\t",
    "\r",
    "é",
    "中",
    "😀",
    "𝄞",
    "\u{2028}",
    "\"__proto__\":",
    "\"constructor\":",
    "{\"prototype\":1}",
    "1e400",
    "123456789012345678901234567890",
    "/",
    "'",
    "NaN",
    "\u{0}",
];

fn soup(rng: &mut Rng, out: &mut String, max: usize) {
    for _ in 0..rng.below(max) + 1 {
        out.push_str(rng.pick(FRAGMENTS));
    }
}

fn ws(rng: &mut Rng, out: &mut String) {
    if rng.below(3) == 0 {
        out.push_str(rng.pick(&[" ", "\n", "  ", "\t", "\r\n"]));
    }
}

fn string(rng: &mut Rng, out: &mut String) {
    const PARTS: &[&str] = &[
        "a",
        "b",
        "key",
        "value",
        " ",
        "é",
        "中",
        "😀",
        "𝄞",
        "\\\"",
        "\\\\",
        "\\/",
        "\\n",
        "\\t",
        "\\u0041",
        "\\u00e9",
        "\\uD83D\\uDE00",
        "\\ud800",
        "{",
        "}",
        "[",
        "]",
        ",",
        ":",
        "true",
        "null",
        "1",
        "__proto__",
        "constructor",
    ];
    out.push('"');
    for _ in 0..rng.below(5) {
        out.push_str(rng.pick(PARTS));
    }
    out.push('"');
}

fn number(rng: &mut Rng, out: &mut String) {
    if rng.below(3) == 0 {
        out.push('-');
    }
    out.push_str(rng.pick(&[
        "0",
        "1",
        "7",
        "42",
        "100",
        "12345678901234567890",
        "9007199254740993",
    ]));
    if rng.below(3) == 0 {
        out.push('.');
        out.push_str(rng.pick(&["0", "5", "25", "000001"]));
    }
    if rng.below(4) == 0 {
        out.push_str(rng.pick(&["e", "E"]));
        out.push_str(rng.pick(&["", "-", "+"]));
        out.push_str(rng.pick(&["1", "3", "21", "400"]));
    }
}

fn value(rng: &mut Rng, out: &mut String, depth: usize) {
    let kind = if depth > 4 {
        rng.below(4)
    } else {
        rng.below(7)
    };
    match kind {
        0 => string(rng, out),
        1 => number(rng, out),
        2 => out.push_str(rng.pick(&["true", "false", "null"])),
        3 => string(rng, out),
        4 | 5 => {
            out.push('{');
            for k in 0..rng.below(4) {
                if k > 0 {
                    out.push(',');
                }
                ws(rng, out);
                if rng.below(8) == 0 {
                    out.push_str(rng.pick(&["\"__proto__\"", "\"constructor\"", "\"a\""]));
                } else {
                    string(rng, out);
                }
                ws(rng, out);
                out.push(':');
                ws(rng, out);
                value(rng, out, depth + 1);
                ws(rng, out);
            }
            out.push('}');
        }
        _ => {
            out.push('[');
            for k in 0..rng.below(4) {
                if k > 0 {
                    out.push(',');
                }
                ws(rng, out);
                value(rng, out, depth + 1);
                ws(rng, out);
            }
            out.push(']');
        }
    }
}

fn char_cut(s: &str, rng: &mut Rng) -> usize {
    let bounds: Vec<usize> = s.char_indices().map(|(i, _)| i).chain([s.len()]).collect();
    bounds[rng.below(bounds.len())]
}

/// One generated input: empty, raw fragment soup, deep nesting, or a valid
/// document that is kept, truncated, extended with junk, has a range
/// repeated, or has two ranges swapped.
pub fn input(rng: &mut Rng) -> String {
    let mut s = String::new();
    match rng.below(100) {
        0 => {}
        1..=25 => soup(rng, &mut s, 12),
        26..=28 => {
            let depth = rng.below(200) + 1;
            for _ in 0..depth {
                s.push_str(rng.pick(&["[", "{\"k\":"]));
            }
            if rng.below(2) == 0 {
                s.push('1');
            }
        }
        _ => {
            ws(rng, &mut s);
            value(rng, &mut s, 0);
            ws(rng, &mut s);
            match rng.below(6) {
                0 => {}
                1 | 2 => s.truncate(char_cut(&s, rng)),
                3 => {
                    s.truncate(char_cut(&s, rng));
                    soup(rng, &mut s, 3);
                }
                4 => {
                    let (a, b) = (char_cut(&s, rng), char_cut(&s, rng));
                    let (a, b) = (a.min(b), a.max(b));
                    let dup = s[a..b].to_owned();
                    s.insert_str(b, &dup);
                    s.truncate(char_cut(&s, rng));
                }
                _ => {
                    let (a, b) = (char_cut(&s, rng), char_cut(&s, rng));
                    let (a, b) = (a.min(b), a.max(b));
                    let m = a + (b - a) / 2;
                    let m = (m..=b).find(|&i| s.is_char_boundary(i)).unwrap_or(b);
                    s = format!("{}{}{}{}", &s[..a], &s[m..b], &s[a..m], &s[b..]);
                }
            }
        }
    }
    s
}

/// The legacy result as the new API's outcome; `None` is upstream's
/// `undefined-input`. A value that does not fit its state is reported as a
/// difference by the caller.
fn legacy_outcome(
    result: legacy::ParsePartialJsonResult,
) -> Result<Option<PartialJson>, legacy::ParsePartialJsonResult> {
    use legacy::ParsePartialJsonState as State;
    match (result.state, result.value) {
        (State::UndefinedInput, None) => Ok(None),
        (State::SuccessfulParse, Some(value)) => Ok(Some(PartialJson::Parsed(value))),
        (State::RepairedParse, Some(value)) => Ok(Some(PartialJson::Repaired(value))),
        (State::FailedParse, None) => Ok(Some(PartialJson::Failed)),
        (state, value) => Err(legacy::ParsePartialJsonResult { value, state }),
    }
}

#[test]
fn matches_the_line_for_line_port() {
    let n: usize = std::env::var("JSON_REPAIR_DIFF_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let mut rng = Rng::new(0x9e37_79b9_7f4a_7c15);
    let mut differences = Vec::new();
    let mut states = [0usize; 3];

    let none: Option<&str> = None;
    assert_eq!(
        legacy_outcome(legacy::parse_partial_json(None)),
        Ok(none.map(PartialJson::parse))
    );

    for _ in 0..n {
        let text = input(&mut rng);

        let (old, new) = (legacy::fix_json(&text), fix_json(&text));
        if old != new {
            differences.push(format!("fix_json({text:?}): old {old:?}, new {new:?}"));
        }

        let old = legacy_outcome(legacy::parse_partial_json(Some(&text)));
        let new = PartialJson::parse(&text);
        states[match new {
            PartialJson::Parsed(_) => 0,
            PartialJson::Repaired(_) => 1,
            _ => 2,
        }] += 1;
        if old.as_ref() != Ok(&Some(new.clone())) {
            differences.push(format!(
                "PartialJson::parse({text:?}): old {old:?}, new {new:?}"
            ));
        }
    }

    eprintln!(
        "compared {n} inputs (parsed {}, repaired {}, failed {}), {} differences",
        states[0],
        states[1],
        states[2],
        differences.len()
    );
    assert!(
        differences.is_empty(),
        "{}",
        differences[..differences.len().min(10)].join("\n")
    );
}
