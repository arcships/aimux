//! JSON repair and partial-JSON parsing.
//!
//! Port of `packages/ai/src/util/fix-json.ts` and
//! `packages/ai/src/util/parse-partial-json.ts` from the Vercel AI SDK. The
//! scanner states keep their upstream names in the variant docs so upstream
//! changes can be located.
//!
//! Core's own paths do not call it: a final tool-call input is parsed as
//! given, never repaired (RFC-0042 §3.2). It stays public because `ai`
//! exports `parsePartialJson`, for callers that show streamed partial JSON
//! (tool-input deltas, partial objects) before it is complete.
//!
//! Positions are counted in Unicode scalar values, where upstream counts
//! UTF-16 code units. The two only disagree when a character outside the
//! Basic Multilingual Plane (an emoji, say) directly ends a number or a
//! literal inside an array: upstream keeps the character (`"[1😀"` becomes
//! `"[1😀]"`), this port drops it (`"[1]"`).

use std::ops::ControlFlow;

use serde_json::Value;

/// Repairs a possibly truncated JSON text into JSON a parser accepts.
///
/// Keeps the longest prefix that ends on a complete token and appends the
/// closing tokens (`"`, `}`, `]`, or the rest of `true`/`false`/`null`) for
/// every value still open. Text after the top-level value is dropped. The
/// result is not validated: input that is invalid for other reasons stays
/// invalid.
///
/// ```
/// use aimux_core::json_repair::fix_json;
///
/// assert_eq!(fix_json(r#"{"items": [1, 2, {"done": tr"#), r#"{"items": [1, 2, {"done": true}]}"#);
/// assert_eq!(fix_json(r#"{"key": "val"#), r#"{"key": "val"}"#);
/// assert_eq!(fix_json("[1, "), "[1]");
/// ```
#[doc(alias = "fixJson")]
#[must_use]
pub fn fix_json(input: &str) -> String {
    let mut chars = input.char_indices();
    let mut repair = Repair::default();

    // Upstream `ROOT`: characters before the first value start are skipped.
    if let Some((i, c, frame)) = chars
        .by_ref()
        .find_map(|(i, c)| Frame::opened_by(c).map(|frame| (i, c, frame)))
    {
        repair.open_value(i, c, frame);
    }
    for (i, c) in chars {
        if repair.step(i, c).is_break() {
            break;
        }
    }
    repair.finish(input)
}

/// A value that is open at the current position, with where the scanner is
/// inside it.
#[derive(Clone, Copy)]
enum Frame {
    Object(ObjectPos),
    Array(ArrayPos),
    String(StringPos),
    /// `INSIDE_NUMBER`.
    Number,
    /// `INSIDE_LITERAL`; holds the part of `true`/`false`/`null` not seen yet.
    Literal(&'static str),
}

#[derive(Clone, Copy)]
enum ObjectPos {
    /// `INSIDE_OBJECT_START`: after `{`.
    Start,
    /// `INSIDE_OBJECT_KEY`: inside a key.
    Key,
    /// `INSIDE_OBJECT_AFTER_KEY`: after a key, before `:`.
    AfterKey,
    /// `INSIDE_OBJECT_BEFORE_VALUE`: after `:`.
    BeforeValue,
    /// `INSIDE_OBJECT_AFTER_VALUE`.
    AfterValue,
    /// `INSIDE_OBJECT_AFTER_COMMA`.
    AfterComma,
}

#[derive(Clone, Copy)]
enum ArrayPos {
    /// `INSIDE_ARRAY_START`: after `[`.
    Start,
    /// `INSIDE_ARRAY_AFTER_VALUE`.
    AfterValue,
    /// `INSIDE_ARRAY_AFTER_COMMA`.
    AfterComma,
}

#[derive(Clone, Copy)]
enum StringPos {
    /// `INSIDE_STRING`.
    Plain,
    /// `INSIDE_STRING_ESCAPE`: after `\`.
    Escape,
    /// `INSIDE_STRING_UNICODE_ESCAPE`: after `\u` and `digits` hex digits.
    UnicodeEscape { digits: u8 },
}

impl Frame {
    /// The frame a value starting with `c` opens (upstream `processValueStart`).
    fn opened_by(c: char) -> Option<Self> {
        match c {
            '"' => Some(Self::String(StringPos::Plain)),
            'f' => Some(Self::Literal("alse")),
            't' => Some(Self::Literal("rue")),
            'n' => Some(Self::Literal("ull")),
            '-' | '0'..='9' => Some(Self::Number),
            '{' => Some(Self::Object(ObjectPos::Start)),
            '[' => Some(Self::Array(ArrayPos::Start)),
            _ => None,
        }
    }

    /// What closes this frame when the input ends inside it.
    fn closing(self) -> &'static str {
        match self {
            Self::Object(_) => "}",
            Self::Array(_) => "]",
            Self::String(_) => "\"",
            Self::Number => "",
            Self::Literal(rest) => rest,
        }
    }
}

#[derive(Default)]
struct Repair {
    /// Open values, outermost first. Empty before the top-level value starts
    /// (upstream `ROOT`) and after it closes (upstream `FINISH`).
    open: Vec<Frame>,
    /// Byte length of the longest prefix that ends on a complete token
    /// (upstream `lastValidIndex + 1`, in bytes instead of code units).
    valid_len: usize,
}

impl Repair {
    /// Marks the input up to and including `c` (at byte `i`) as keepable.
    fn mark(&mut self, i: usize, c: char) {
        self.valid_len = i + c.len_utf8();
    }

    /// Marks `c` and closes the innermost value.
    fn close(&mut self, i: usize, c: char) {
        self.mark(i, c);
        self.open.pop();
    }

    /// Pushes the value `c` starts. A lone `-` is not a number yet, so it is
    /// not marked.
    fn open_value(&mut self, i: usize, c: char, frame: Frame) {
        if c != '-' {
            self.mark(i, c);
        }
        self.open.push(frame);
    }

    /// Upstream `processAfterObjectValue` / `processAfterArrayValue`.
    fn after_value(&mut self, i: usize, c: char) {
        let Some(top) = self.open.last_mut() else {
            return;
        };
        match (*top, c) {
            (Frame::Object(ObjectPos::AfterValue), ',') => {
                *top = Frame::Object(ObjectPos::AfterComma);
            }
            (Frame::Array(ArrayPos::AfterValue), ',') => *top = Frame::Array(ArrayPos::AfterComma),
            (Frame::Object(ObjectPos::AfterValue), '}')
            | (Frame::Array(ArrayPos::AfterValue), ']') => {
                self.close(i, c);
            }
            _ => {}
        }
    }

    /// Closes a number or literal at `c`, which then goes to the enclosing
    /// container. `c` is not reprocessed in any other state.
    fn end_scalar(&mut self, i: usize, c: char) {
        self.open.pop();
        self.after_value(i, c);
    }

    /// Feeds the character `c` at byte `i`. Breaks once the top-level value
    /// has closed (upstream `FINISH`): the rest of the input is ignored.
    fn step(&mut self, i: usize, c: char) -> ControlFlow<()> {
        let Some(top) = self.open.last_mut() else {
            return ControlFlow::Break(());
        };
        match *top {
            Frame::Object(pos) => match (pos, c) {
                (ObjectPos::Start | ObjectPos::AfterComma, '"') => {
                    *top = Frame::Object(ObjectPos::Key);
                }
                (ObjectPos::Start, '}') => self.close(i, c),
                (ObjectPos::Key, '"') => *top = Frame::Object(ObjectPos::AfterKey),
                (ObjectPos::AfterKey, ':') => *top = Frame::Object(ObjectPos::BeforeValue),
                (ObjectPos::BeforeValue, _) => {
                    if let Some(value) = Frame::opened_by(c) {
                        *top = Frame::Object(ObjectPos::AfterValue);
                        self.open_value(i, c, value);
                    }
                }
                (ObjectPos::AfterValue, _) => self.after_value(i, c),
                _ => {}
            },
            Frame::Array(pos) => match (pos, c) {
                (ArrayPos::Start | ArrayPos::AfterValue, ']') => self.close(i, c),
                (ArrayPos::Start, _) => {
                    if let Some(value) = Frame::opened_by(c) {
                        *top = Frame::Array(ArrayPos::AfterValue);
                        self.open_value(i, c, value);
                    }
                    // Upstream marks every character here, even one that
                    // starts no value.
                    self.mark(i, c);
                }
                (ArrayPos::AfterValue, ',') => *top = Frame::Array(ArrayPos::AfterComma),
                (ArrayPos::AfterValue, _) => self.mark(i, c),
                (ArrayPos::AfterComma, _) => {
                    if let Some(value) = Frame::opened_by(c) {
                        *top = Frame::Array(ArrayPos::AfterValue);
                        self.open_value(i, c, value);
                    }
                }
            },
            Frame::String(pos) => match (pos, c) {
                (StringPos::Plain, '"') => self.close(i, c),
                (StringPos::Plain, '\\') => *top = Frame::String(StringPos::Escape),
                (StringPos::Plain, _) => self.mark(i, c),
                (StringPos::Escape, 'u') => {
                    *top = Frame::String(StringPos::UnicodeEscape { digits: 0 });
                }
                (StringPos::Escape, _) => {
                    *top = Frame::String(StringPos::Plain);
                    self.mark(i, c);
                }
                (StringPos::UnicodeEscape { digits }, _) if c.is_ascii_hexdigit() => {
                    if digits == 3 {
                        *top = Frame::String(StringPos::Plain);
                        self.mark(i, c);
                    } else {
                        *top = Frame::String(StringPos::UnicodeEscape { digits: digits + 1 });
                    }
                }
                (StringPos::UnicodeEscape { .. }, _) => {}
            },
            Frame::Number => match c {
                '0'..='9' => self.mark(i, c),
                'e' | 'E' | '-' | '.' => {}
                _ => self.end_scalar(i, c),
            },
            Frame::Literal(rest) => match rest.strip_prefix(c) {
                Some(rest) => {
                    *top = Frame::Literal(rest);
                    self.mark(i, c);
                }
                None => self.end_scalar(i, c),
            },
        }
        ControlFlow::Continue(())
    }

    fn finish(self, input: &str) -> String {
        let closing: usize = self.open.iter().map(|frame| frame.closing().len()).sum();
        let mut out = String::with_capacity(self.valid_len + closing);
        // `valid_len` only ever ends a character from `char_indices`, so it
        // is a char boundary.
        out.push_str(&input[..self.valid_len]);
        for frame in self.open.iter().rev() {
            out.push_str(frame.closing());
        }
        out
    }
}

/// Outcome of [`PartialJson::parse`].
///
/// ```
/// use aimux_core::json_repair::PartialJson;
/// use serde_json::json;
///
/// // A streamed tool-call argument cut off mid-way.
/// let args = PartialJson::parse(r#"{"city": "Par"#);
/// assert_eq!(args, PartialJson::Repaired(json!({"city": "Par"})));
/// assert_eq!(args.value(), Some(&json!({"city": "Par"})));
///
/// assert_eq!(PartialJson::parse("[1, 2]").into_value(), Some(json!([1, 2])));
/// assert_eq!(PartialJson::parse("not json"), PartialJson::Failed);
///
/// // Upstream's `undefined-input` state is the caller's `Option`:
/// let text: Option<&str> = None;
/// assert_eq!(text.map(PartialJson::parse), None);
/// ```
#[doc(alias = "parsePartialJson")]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PartialJson {
    /// The text parsed as-is (upstream `successful-parse`).
    Parsed(Value),
    /// The text parsed only after [`fix_json`] (upstream `repaired-parse`).
    Repaired(Value),
    /// The text did not parse even after repair (upstream `failed-parse`).
    Failed,
}

impl PartialJson {
    /// Parses `text`, retrying with [`fix_json`] if it does not parse as-is.
    ///
    /// Upstream's `parsePartialJson` is `async` only because its JSON parser
    /// is; this is synchronous.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        if let Ok(value) = serde_json::from_str(text) {
            return Self::Parsed(value);
        }
        match serde_json::from_str(&fix_json(text)) {
            Ok(value) => Self::Repaired(value),
            Err(_) => Self::Failed,
        }
    }

    /// The parsed value, repaired or not.
    #[must_use]
    pub fn value(&self) -> Option<&Value> {
        match self {
            Self::Parsed(value) | Self::Repaired(value) => Some(value),
            Self::Failed => None,
        }
    }

    /// Takes the parsed value, repaired or not.
    #[must_use]
    pub fn into_value(self) -> Option<Value> {
        match self {
            Self::Parsed(value) | Self::Repaired(value) => Some(value),
            Self::Failed => None,
        }
    }
}
