//! Small Google-specific utility functions.
//!
//! Mirrors `get-model-path.ts`, `google-supported-file-url.ts`, and
//! `google-json-accumulator.ts` from the TS SDK.

use aimux_core::error::AiMuxError;
use serde_json::{Map, Value};

/// Resolve a model id into the URL path segment used by the Gemini API.
///
/// Mirrors `getModelPath` in `get-model-path.ts`:
/// - Ids that already contain a `/` (e.g. `models/foo`, `tunedModels/bar`) are
///   passed through unchanged.
/// - Bare ids are prefixed with `models/`.
pub fn get_model_path(model_id: &str) -> String {
    if model_id.contains('/') {
        model_id.to_string()
    } else {
        format!("models/{}", model_id)
    }
}

/// Whether a URL is supported as a file input by the Google Gemini API.
///
/// Mirrors `isSupportedFileUrl` in `google-supported-file-url.ts`:
/// - `https://generativelanguage.googleapis.com/v1beta/files/...` → true
/// - YouTube watch / youtu.be URLs → true
/// - everything else → false
pub fn is_supported_file_url(url: &str) -> bool {
    if url.starts_with("https://generativelanguage.googleapis.com/v1beta/files/") {
        return true;
    }

    // YouTube watch URLs: https://(www.)youtube.com/watch?v=...(&...)?
    if let Some(rest) = url
        .strip_prefix("https://")
        .and_then(|r| r.strip_prefix("www.").or(Some(r)))
        && let Some(query) = rest.strip_prefix("youtube.com/watch")
    {
        // Must be followed by `?v=...` (the query string). A bare
        // `/watch` with no query, or `/watch/...`, is not a video URL.
        if let Some(q) = query.strip_prefix('?') {
            return youtube_query_has_v(q);
        }
        return false;
    }

    // youtu.be short URLs: https://youtu.be/<id>(?...)?
    if let Some(rest) = url.strip_prefix("https://youtu.be/") {
        // The video id is everything up to `?` or end.
        let id = rest.split('?').next().unwrap_or(rest);
        return is_valid_youtube_id(id);
    }

    false
}

/// Check that a YouTube watch query string starts with `v=<id>`.
fn youtube_query_has_v(query: &str) -> bool {
    // The first parameter must be `v=<id>` where id is non-empty and matches
    // `[\w-]+`. Additional params may follow separated by `&`.
    let mut params = query.split('&');
    let Some(first) = params.next() else {
        return false;
    };
    let Some(value) = first.strip_prefix("v=") else {
        return false;
    };
    is_valid_youtube_id(value)
}

fn is_valid_youtube_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

// ── JSON accumulator ─────────────────────────────────────────────────────────

/// A single partial argument streamed by Google's API during tool-call
/// function calling.
#[derive(Debug, Clone, Default)]
pub struct PartialArg {
    /// JSON path like `$.location` or `$.recipe.ingredients[0].name`.
    pub json_path: String,
    pub string_value: Option<String>,
    pub number_value: Option<f64>,
    pub bool_value: Option<bool>,
    /// When present (even `nullValue: {}`), the resolved value is JSON `null`.
    pub null_value: Option<()>,
    /// Whether the string value will continue in a subsequent chunk (the
    /// closing quote is deferred).
    pub will_continue: Option<bool>,
}

impl PartialArg {
    /// Resolve the value carried by this partial arg, plus its JSON
    /// representation. Returns `None` when no value is resolvable.
    fn resolve(&self) -> Option<(Value, String)> {
        if let Some(s) = &self.string_value {
            return Some((Value::String(s.clone()), serde_json::to_string(s).unwrap()));
        }
        if let Some(n) = self.number_value {
            // JavaScript's JSON.stringify omits the fractional part for whole
            // numbers (50, not 50.0). Mirror that by converting whole-number
            // f64s to serde_json integers.
            let v = if n.fract() == 0.0 && n.is_finite() && n.abs() < 9.0072e15 {
                Value::Number(serde_json::Number::from(n as i64))
            } else {
                serde_json::Number::from_f64(n)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            };
            return Some((v.clone(), v.to_string()));
        }
        if let Some(b) = self.bool_value {
            return Some((Value::Bool(b), b.to_string()));
        }
        if self.null_value.is_some() {
            return Some((Value::Null, "null".to_string()));
        }
        None
    }
}

/// Result of processing a batch of partial args.
#[derive(Debug, Clone)]
pub struct ProcessResult {
    /// The accumulated structured object so far.
    pub current_json: Value,
    /// The JSON text fragment emitted by this call.
    pub text_delta: String,
}

/// Result of finalizing the accumulator.
#[derive(Debug, Clone)]
pub struct FinalizeResult {
    /// The complete JSON string.
    pub final_json: String,
    /// The text fragment needed to close all open containers.
    pub closing_delta: String,
}

/// Incrementally builds a JSON object from Google's streaming `partialArgs`
/// chunks.
///
/// Mirrors `GoogleJSONAccumulator` in `google-json-accumulator.ts`. Tracks both
/// the structured object and a running JSON text representation so callers can
/// emit text deltas that, when concatenated, form valid nested JSON matching
/// `serde_json::to_string` output.
#[derive(Debug, Clone, Default)]
pub struct GoogleJsonAccumulator {
    accumulated: Value,
    json_text: String,
    /// Stack of open containers (root is always index 0 once started).
    path_stack: Vec<StackEntry>,
    /// Whether a string value is currently "open" (willContinue was true).
    string_open: bool,
}

#[derive(Debug, Clone)]
struct StackEntry {
    segment: Segment,
    is_array: bool,
    child_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Key(String),
    Index(usize),
    /// The root container placeholder.
    Root,
}

impl GoogleJsonAccumulator {
    pub fn new() -> Self {
        Self {
            accumulated: Value::Object(Map::new()),
            ..Default::default()
        }
    }

    /// Process a batch of partial args, returning the structured object and
    /// the JSON text delta for this call.
    ///
    /// Returns [`AiMuxError::Json`] when a partial-arg path conflicts with the
    /// already-accumulated structure (e.g. `$.a` was a string, then `$.a[0]`
    /// arrives). `partialArgs` is provider-controlled input, so such conflicts
    /// must not panic.
    pub fn process_partial_args(
        &mut self,
        args: &[PartialArg],
    ) -> Result<ProcessResult, AiMuxError> {
        let mut delta = String::new();

        for arg in args {
            let raw_path = arg.json_path.strip_prefix("$.").unwrap_or(&arg.json_path);
            if raw_path.is_empty() {
                continue;
            }

            let segments = parse_path(raw_path);

            // Malformed/empty path (e.g. `$.[]`) parses to no segments; without
            // this guard `emit_navigation_to` would slice `[..len-1]` on an
            // empty slice and panic (audit finding H3 residual panic path).
            if segments.is_empty() {
                continue;
            }

            let existing = get_nested_value(&self.accumulated, &segments);

            if let (Some(s), Some(existing_val)) = (&arg.string_value, existing) {
                // String continuation chunk.
                let escaped = escape_json_string_inner(s);
                let new_val = match existing_val {
                    Value::String(prev) => Value::String(format!("{}{}", prev, s)),
                    _ => Value::String(s.clone()),
                };
                set_nested_value(&mut self.accumulated, &segments, new_val)?;
                delta.push_str(&escaped);
                continue;
            }

            let Some((value, value_json)) = arg.resolve() else {
                continue;
            };

            set_nested_value(&mut self.accumulated, &segments, value)?;
            delta.push_str(&self.emit_navigation_to(&segments, arg, &value_json));
        }

        self.json_text.push_str(&delta);

        Ok(ProcessResult {
            current_json: self.accumulated.clone(),
            text_delta: delta,
        })
    }

    /// Finalize the accumulator, producing the complete JSON string and the
    /// closing delta.
    pub fn finalize(&self) -> FinalizeResult {
        let final_json = serde_json::to_string(&self.accumulated).unwrap_or_default();
        let closing_delta = if final_json.len() >= self.json_text.len() {
            final_json[self.json_text.len()..].to_string()
        } else {
            String::new()
        };
        FinalizeResult {
            final_json,
            closing_delta,
        }
    }

    fn ensure_root(&mut self) -> &'static str {
        if self.path_stack.is_empty() {
            self.path_stack.push(StackEntry {
                segment: Segment::Root,
                is_array: false,
                child_count: 0,
            });
            "{"
        } else {
            ""
        }
    }

    fn emit_navigation_to(
        &mut self,
        target_segments: &[Segment],
        arg: &PartialArg,
        value_json: &str,
    ) -> String {
        let mut fragment = String::new();

        if self.string_open {
            fragment.push('"');
            self.string_open = false;
        }

        fragment.push_str(self.ensure_root());

        let target_container = &target_segments[..target_segments.len() - 1];
        let leaf = &target_segments[target_segments.len() - 1];

        let common_depth = self.find_common_stack_depth(target_container);
        fragment.push_str(&self.close_down_to(common_depth));
        fragment.push_str(&self.open_down_to(target_container, leaf));
        fragment.push_str(&self.emit_leaf(leaf, arg, value_json));

        fragment
    }

    fn find_common_stack_depth(&self, target_container: &[Segment]) -> usize {
        let max_depth = (self.path_stack.len() - 1).min(target_container.len());
        let mut common = 0;
        for (i, target_seg) in target_container.iter().take(max_depth).enumerate() {
            if self.path_stack[i + 1].segment == *target_seg {
                common += 1;
            } else {
                break;
            }
        }
        common + 1
    }

    fn close_down_to(&mut self, target_depth: usize) -> String {
        let mut fragment = String::new();
        while self.path_stack.len() > target_depth {
            let entry = self.path_stack.pop().unwrap();
            fragment.push(if entry.is_array { ']' } else { '}' });
        }
        fragment
    }

    fn open_down_to(&mut self, target_container: &[Segment], leaf: &Segment) -> String {
        let mut fragment = String::new();
        let start_idx = self.path_stack.len() - 1;

        for i in start_idx..target_container.len() {
            let path_segment = &target_container[i];
            let parent_entry = self.path_stack.last_mut().unwrap();

            if parent_entry.child_count > 0 {
                fragment.push(',');
            }
            parent_entry.child_count += 1;

            if let Segment::Key(k) = path_segment {
                fragment.push_str(&serde_json::to_string(k).unwrap());
                fragment.push(':');
            }

            let child_seg = if i + 1 < target_container.len() {
                &target_container[i + 1]
            } else {
                leaf
            };
            let is_array = matches!(child_seg, Segment::Index(_));

            fragment.push(if is_array { '[' } else { '{' });

            self.path_stack.push(StackEntry {
                segment: path_segment.clone(),
                is_array,
                child_count: 0,
            });
        }

        fragment
    }

    fn emit_leaf(&mut self, leaf: &Segment, arg: &PartialArg, value_json: &str) -> String {
        let mut fragment = String::new();
        let container = self.path_stack.last_mut().unwrap();

        if container.child_count > 0 {
            fragment.push(',');
        }
        container.child_count += 1;

        if let Segment::Key(k) = leaf {
            fragment.push_str(&serde_json::to_string(k).unwrap());
            fragment.push(':');
        }

        if arg.string_value.is_some() && arg.will_continue == Some(true) {
            // Omit the closing quote; the value_json includes it.
            fragment.push_str(&value_json[..value_json.len() - 1]);
            self.string_open = true;
        } else {
            fragment.push_str(value_json);
        }

        fragment
    }
}

/// Parse a dotted/bracketed JSON path like `recipe.ingredients[0].name` into
/// segments.
fn parse_path(raw: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    for part in raw.split('.') {
        let bracket_idx = part.find('[');
        match bracket_idx {
            None => segments.push(Segment::Key(part.to_string())),
            Some(idx) => {
                if idx > 0 {
                    segments.push(Segment::Key(part[..idx].to_string()));
                }
                let mut rest = &part[idx..];
                while !rest.is_empty() && rest.starts_with('[') {
                    let close = match rest.find(']') {
                        Some(c) => c,
                        None => break,
                    };
                    let num_str = &rest[1..close];
                    if let Ok(n) = num_str.parse::<usize>() {
                        segments.push(Segment::Index(n));
                    }
                    rest = &rest[close + 1..];
                }
            }
        }
    }
    segments
}

/// Escape a string the way `JSON.stringify(s).slice(1, -1)` does — i.e. the
/// inner content without the surrounding quotes.
fn escape_json_string_inner(s: &str) -> String {
    let quoted = serde_json::to_string(s).unwrap();
    quoted[1..quoted.len() - 1].to_string()
}

/// Traverse a nested value along the given path segments and return the leaf.
fn get_nested_value(obj: &Value, segments: &[Segment]) -> Option<Value> {
    let mut current = obj;
    for seg in segments {
        match seg {
            Segment::Key(k) => {
                current = current.as_object()?.get(k)?;
            }
            Segment::Index(i) => {
                current = current.as_array()?.get(*i)?;
            }
            Segment::Root => {}
        }
    }
    Some(current.clone())
}

/// Maximum array index accepted from a partialArgs path. `partialArgs` is
/// provider-controlled; without a cap, a path like `$.a[1000000000]` makes
/// the array-spreading loop allocate ~1 GiB of `Value::Null`, exhausting
/// memory (audit finding, round 2).
const MAX_PARTIAL_ARG_INDEX: usize = 100_000;

/// Maximum path depth accepted from a partialArgs path (guards against
/// pathologically deep nesting).
const MAX_PARTIAL_ARG_DEPTH: usize = 64;

/// Set a value at a nested path, creating intermediate objects or arrays.
///
/// Returns [`AiMuxError::Json`] instead of panicking when the incoming path
/// conflicts with the already-accumulated type (e.g. `$.a` was set to a
/// string and a later `partialArgs` chunk targets `$.a[0].b`), or when the
/// path exceeds the resource caps ([`MAX_PARTIAL_ARG_INDEX`],
/// [`MAX_PARTIAL_ARG_DEPTH`]). The partial stream is provider-controlled
/// (untrusted), so a conflict must surface as an error, not a process crash.
fn set_nested_value(obj: &mut Value, segments: &[Segment], value: Value) -> Result<(), AiMuxError> {
    if segments.is_empty() {
        return Ok(());
    }
    if segments.len() > MAX_PARTIAL_ARG_DEPTH {
        return Err(AiMuxError::Json(format!(
            "partial args path exceeds maximum depth of {MAX_PARTIAL_ARG_DEPTH}"
        )));
    }
    let mut current = obj;
    for i in 0..segments.len() - 1 {
        let seg = &segments[i];
        let next_is_index = matches!(segments[i + 1], Segment::Index(_));
        let exists = match seg {
            Segment::Key(k) => current
                .as_object_mut()
                .map(|m| m.contains_key(k))
                .unwrap_or(false),
            Segment::Index(i) => current
                .as_array_mut()
                .map(|a| *i < a.len() && !a[*i].is_null())
                .unwrap_or(false),
            Segment::Root => true,
        };

        if !exists {
            match seg {
                Segment::Key(k) => {
                    let parent = current.as_object_mut().ok_or_else(|| parent_must_be(seg))?;
                    let new_val = if next_is_index {
                        Value::Array(Vec::new())
                    } else {
                        Value::Object(Map::new())
                    };
                    parent.insert(k.clone(), new_val);
                }
                Segment::Index(i) => {
                    let arr = current.as_array_mut().ok_or_else(|| parent_must_be(seg))?;
                    while arr.len() <= *i {
                        arr.push(Value::Null);
                    }
                    if arr[*i].is_null() {
                        arr[*i] = if next_is_index {
                            Value::Array(Vec::new())
                        } else {
                            Value::Object(Map::new())
                        };
                    }
                }
                Segment::Root => {}
            }
        }

        current = match seg {
            Segment::Key(k) => current
                .as_object_mut()
                .ok_or_else(|| parent_must_be(seg))?
                .get_mut(k)
                .ok_or_else(|| {
                    AiMuxError::Json(format!("partial args path conflict at {:?}", seg))
                })?,
            Segment::Index(i) => current
                .as_array_mut()
                .ok_or_else(|| parent_must_be(seg))?
                .get_mut(*i)
                .ok_or_else(|| {
                    AiMuxError::Json(format!("partial args path conflict at {:?}", seg))
                })?,
            Segment::Root => current,
        };
    }

    let last = &segments[segments.len() - 1];
    match last {
        Segment::Key(k) => {
            current
                .as_object_mut()
                .ok_or_else(|| parent_must_be(last))?
                .insert(k.clone(), value);
        }
        Segment::Index(i) => {
            if *i > MAX_PARTIAL_ARG_INDEX {
                return Err(AiMuxError::Json(format!(
                    "partial args array index {i} exceeds maximum of {MAX_PARTIAL_ARG_INDEX}"
                )));
            }
            let arr = current.as_array_mut().ok_or_else(|| parent_must_be(last))?;
            while arr.len() <= *i {
                arr.push(Value::Null);
            }
            arr[*i] = value;
        }
        Segment::Root => {
            *current = value;
        }
    }
    Ok(())
}

/// Build the path-type-conflict error for a segment whose actual JSON type
/// does not match the path's expectation (e.g. an index into a string).
fn parent_must_be(seg: &Segment) -> AiMuxError {
    AiMuxError::Json(format!(
        "partial args path type conflict at {:?}: accumulated value does not match the path",
        seg
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc
            .process_partial_args(&[PartialArg {
                json_path: "$.location".to_string(),
                string_value: Some("Boston".to_string()),
                ..Default::default()
            }])
            .unwrap();
        assert_eq!(r.text_delta, "{\"location\":\"Boston\"");
        assert_eq!(r.current_json["location"], "Boston");
    }
}
