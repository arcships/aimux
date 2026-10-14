//! Old-vs-new differential probe (not a regular test; does nothing unless
//! `PROBE_OUT` is set).
//!
//! For every cassette under `tests/cassettes/<vendor>/*.json` whose endpoint is
//! an LLM call (chat completions, responses, anthropic messages, gemini
//! generateContent, bedrock converse, cohere v2 chat) the probe:
//!
//! 1. derives `CallOptions` from the cassette's recorded request body (prompt,
//!    system, tool calls/results, tools, tool choice, sampling params,
//!    response format), falling back to a "Hello" prompt;
//! 2. builds every public model ("route") that speaks that wire format, with a
//!    scripted transport (`fetch`) that records each request and answers with
//!    the cassette's recorded response. Default base URLs are kept, so the
//!    recorded URL is the one the provider config builds;
//! 3. runs `do_generate`, and for streaming cassettes `do_stream` in five body
//!    variants (whole body, with raw chunks, 7-byte chunks, truncated at half,
//!    truncated + transport error);
//! 4. writes one JSON line per (cassette, route, op) to `$PROBE_OUT`: the
//!    requests (method, url, sorted lower-cased headers, JSON body) and the
//!    outcome (result / error / ordered stream parts).
//!
//! Each op runs twice in-process; random tokens that differ between the two
//! runs are masked as `<Rn>` (numbered by first appearance), and UUIDs,
//! SigV4 signatures, today's timestamps and now-ish epoch numbers are masked
//! by pattern. A preset sweep additionally runs every registry preset against
//! a synthetic chat response with rich usage.
//!
//! Optional: `PROBE_FILTER=<substring>` restricts the keys that run.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use futures::StreamExt;
use regex::Regex;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{
    AwsCredentials, Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, Resolvable,
};

const CASSETTE_DIR: &str = "tests/cassettes";
const MAX_BODY: usize = 256 * 1024;
const OP_TIMEOUT: Duration = Duration::from_secs(20);

// ─── transport ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
enum BodyMode {
    Whole,
    Chunked(usize),
    Trunc,
    TruncErr,
}

#[derive(Clone)]
struct Canned {
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
}

struct ProbeFetch {
    canned: Canned,
    mode: BodyMode,
    seen: Mutex<Vec<Value>>,
}

fn body_value(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
        return json!({ "json": v });
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => json!({ "text": s }),
        Err(_) => json!({ "base64": BASE64.encode(bytes) }),
    }
}

#[async_trait]
impl Fetch for ProbeFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, value) in &request.headers {
            headers
                .entry(name.as_str().to_ascii_lowercase())
                .or_default()
                .push(String::from_utf8_lossy(value.as_bytes()).into_owned());
        }
        self.seen.lock().unwrap().push(json!({
            "method": request.method.as_str(),
            "url": request.url.as_str(),
            "headers": headers,
            "body": body_value(&request.body),
            "credentials": format!("{:?}", request.credentials),
        }));
        let mut hm = HeaderMap::new();
        for (name, value) in &self.canned.headers {
            if let (Ok(n), Ok(v)) = (
                HeaderName::try_from(name.as_str()),
                HeaderValue::try_from(value.as_str()),
            ) {
                hm.append(n, v);
            }
        }
        let body = self.canned.body.clone();
        let half = body.len() / 2;
        let stream: futures::stream::BoxStream<'static, Result<Bytes, FetchError>> = match self.mode
        {
            BodyMode::Whole => futures::stream::once(async move { Ok(body) }).boxed(),
            BodyMode::Chunked(n) => {
                let chunks: Vec<Result<Bytes, FetchError>> = body
                    .chunks(n.max(1))
                    .map(|c| Ok(Bytes::copy_from_slice(c)))
                    .collect();
                futures::stream::iter(chunks).boxed()
            }
            BodyMode::Trunc => {
                futures::stream::once(async move { Ok(Bytes::copy_from_slice(&body[..half])) })
                    .boxed()
            }
            BodyMode::TruncErr => futures::stream::iter(vec![
                Ok(Bytes::copy_from_slice(&body[..half])),
                Err(FetchError::Io("probe: connection cut".into())),
            ])
            .boxed(),
        };
        Ok(FetchResponse {
            status: StatusCode::from_u16(self.canned.status).unwrap_or(StatusCode::OK),
            headers: hm,
            url: request.url,
            body: stream,
        })
    }
}

// ─── cassettes ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Chat,
    Responses,
    Messages,
    Gemini,
    Converse,
    Cohere,
}

struct Cas {
    vendor: String,
    file: String,
    method: String,
    path: String,
    req_body: Value,
    canned: Canned,
    is_stream: bool,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hexv(b[i + 1]), hexv(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}
fn hexv(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn load_cassettes() -> Vec<Cas> {
    let mut files: Vec<_> = Vec::new();
    for vd in fs::read_dir(CASSETTE_DIR).unwrap() {
        let vd = vd.unwrap().path();
        if !vd.is_dir() {
            continue;
        }
        for f in fs::read_dir(&vd).unwrap() {
            let f = f.unwrap().path();
            if f.extension().and_then(|e| e.to_str()) == Some("json") {
                files.push(f);
            }
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let v: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
            let vendor = f
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let file = f.file_name().unwrap().to_string_lossy().to_string();
            let req = &v["request"];
            let resp = &v["response"];
            let headers: Vec<(String, String)> = resp["headers"]
                .as_object()
                .map(|h| {
                    h.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            let body: Vec<u8> = match (&resp["body"], &resp["body_base64"]) {
                (Value::String(s), _) => s.as_bytes().to_vec(),
                (_, Value::String(b)) => BASE64.decode(b).unwrap_or_default(),
                _ => Vec::new(),
            };
            let ct = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v.to_ascii_lowercase())
                .unwrap_or_default();
            let head = String::from_utf8_lossy(&body[..body.len().min(64)]).to_string();
            let head = head.trim_start();
            let is_stream = ct.contains("event-stream")
                || ct.contains("eventstream")
                || ct.contains("ndjson")
                || head.starts_with("data:")
                || head.starts_with("event:");
            Cas {
                vendor,
                file,
                method: req["method"].as_str().unwrap_or("POST").to_ascii_uppercase(),
                path: req["path"].as_str().unwrap_or("/").to_string(),
                req_body: match &req["body"] {
                    Value::String(s) => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
                    other => other.clone(),
                },
                canned: Canned {
                    status: resp["status"].as_u64().unwrap_or(200) as u16,
                    headers,
                    body: Bytes::from(body),
                },
                is_stream,
            }
        })
        .collect()
}

fn classify(c: &Cas) -> Result<Kind, String> {
    let p = c.path.split('?').next().unwrap_or("");
    if c.method != "POST" {
        return Err(format!("{} request (not an LLM call)", c.method));
    }
    if p.ends_with("/chat/completions") {
        Ok(Kind::Chat)
    } else if p.ends_with("/responses") {
        Ok(Kind::Responses)
    } else if p == "/v1/messages" {
        Ok(Kind::Messages)
    } else if p.ends_with(":generateContent") || p.ends_with(":streamGenerateContent") {
        Ok(Kind::Gemini)
    } else if p.ends_with("/converse") || p.ends_with("/converse-stream") {
        Ok(Kind::Converse)
    } else if p == "/v2/chat" {
        Ok(Kind::Cohere)
    } else {
        let short = Regex::new(r"/[A-Za-z0-9_.:%-]{20,}")
            .unwrap()
            .replace_all(p, "/{id}")
            .to_string();
        Err(format!("endpoint {short} has no LanguageModel route"))
    }
}

fn model_id(c: &Cas, kind: Kind) -> String {
    let p = percent_decode(c.path.split('?').next().unwrap_or(""));
    match kind {
        Kind::Gemini => p
            .split("/models/")
            .nth(1)
            .and_then(|r| r.split(':').next())
            .unwrap_or("gemini-2.5-flash")
            .to_string(),
        Kind::Converse => p
            .split("/model/")
            .nth(1)
            .and_then(|r| r.rsplit_once('/').map(|(m, _)| m))
            .unwrap_or("us.anthropic.claude-sonnet-4-20250514-v1:0")
            .to_string(),
        _ => c.req_body["model"]
            .as_str()
            .unwrap_or("probe-model")
            .to_string(),
    }
}

// ─── CallOptions derivation ─────────────────────────────────────────────────

struct Prompt {
    msgs: Vec<Value>,
    names: HashMap<String, String>,
    next_id: usize,
}

impl Prompt {
    fn new() -> Self {
        Self {
            msgs: Vec::new(),
            names: HashMap::new(),
            next_id: 0,
        }
    }
    fn system(&mut self, text: String) {
        if !text.is_empty() {
            self.msgs.push(json!({"role": "system", "content": text}));
        }
    }
    fn push(&mut self, role: &str, part: Value) {
        if let Some(last) = self.msgs.last_mut()
            && last["role"] == role
            && role != "system"
        {
            last["content"].as_array_mut().unwrap().push(part);
            return;
        }
        self.msgs.push(json!({"role": role, "content": [part]}));
    }
    fn user_text(&mut self, t: &str) {
        self.push("user", json!({"type": "text", "text": t}));
    }
    fn assistant_text(&mut self, t: &str) {
        self.push("assistant", json!({"type": "text", "text": t}));
    }
    fn tool_call(&mut self, id: Option<&str>, name: &str, input: Value) -> String {
        let id = id.map(str::to_string).unwrap_or_else(|| {
            self.next_id += 1;
            format!("probe_call_{}", self.next_id)
        });
        self.names.insert(id.clone(), name.to_string());
        self.push(
            "assistant",
            json!({"type": "tool-call", "toolCallId": id, "toolName": name, "input": input}),
        );
        id
    }
    fn tool_result(&mut self, id: &str, name: Option<&str>, output: Value) {
        let name = name
            .map(str::to_string)
            .or_else(|| self.names.get(id).cloned())
            .unwrap_or_else(|| "unknown_tool".into());
        let output = match output {
            Value::String(s) => json!({"type": "text", "value": s}),
            other => json!({"type": "json", "value": other}),
        };
        self.push(
            "tool",
            json!({"type": "tool-result", "toolCallId": id, "toolName": name, "output": output}),
        );
    }
    fn finish(self) -> Vec<Value> {
        let mut m = self.msgs;
        if !m.iter().any(|m| m["role"] != "system") {
            m.push(json!({"role": "user", "content": [{"type": "text", "text": "Hello"}]}));
        }
        m
    }
}

fn texts_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_args(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
        Value::Null => json!({}),
        other => other.clone(),
    }
}

fn openai_messages(p: &mut Prompt, msgs: &[Value]) {
    for m in msgs {
        let role = m["role"].as_str().unwrap_or("user");
        match role {
            "system" | "developer" => p.system(texts_of(&m["content"])),
            "user" => match &m["content"] {
                Value::String(s) => p.user_text(s),
                Value::Array(parts) => {
                    for part in parts {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            p.user_text(t);
                        } else {
                            p.user_text("[non-text part]");
                        }
                    }
                }
                _ => p.user_text(""),
            },
            "assistant" => {
                let t = texts_of(&m["content"]);
                if !t.is_empty() {
                    p.assistant_text(&t);
                }
                if let Some(calls) = m["tool_calls"].as_array() {
                    for c in calls {
                        let f = &c["function"];
                        p.tool_call(
                            c["id"].as_str(),
                            f["name"].as_str().unwrap_or("tool"),
                            parse_args(&f["arguments"]),
                        );
                    }
                }
            }
            "tool" => {
                let id = m["tool_call_id"].as_str().unwrap_or("call_0").to_string();
                let out = texts_of(&m["content"]);
                p.tool_result(&id, None, Value::String(out));
            }
            _ => {}
        }
    }
}

fn anthropic_messages(p: &mut Prompt, body: &Value) {
    p.system(texts_of(&body["system"]));
    for m in body["messages"].as_array().into_iter().flatten() {
        let role = m["role"].as_str().unwrap_or("user");
        match &m["content"] {
            Value::String(s) => {
                if role == "assistant" {
                    p.assistant_text(s)
                } else {
                    p.user_text(s)
                }
            }
            Value::Array(blocks) => {
                for b in blocks {
                    match b["type"].as_str().unwrap_or("") {
                        "text" => {
                            let t = b["text"].as_str().unwrap_or("");
                            if role == "assistant" {
                                p.assistant_text(t)
                            } else {
                                p.user_text(t)
                            }
                        }
                        "tool_use" => {
                            p.tool_call(
                                b["id"].as_str(),
                                b["name"].as_str().unwrap_or("tool"),
                                b["input"].clone(),
                            );
                        }
                        "tool_result" => {
                            let id = b["tool_use_id"].as_str().unwrap_or("toolu_0").to_string();
                            p.tool_result(&id, None, Value::String(texts_of(&b["content"])));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn gemini_messages(p: &mut Prompt, body: &Value) {
    p.system(texts_of(&body["systemInstruction"]["parts"]));
    let mut pending: Vec<(String, String)> = Vec::new();
    for c in body["contents"].as_array().into_iter().flatten() {
        let role = c["role"].as_str().unwrap_or("user");
        for part in c["parts"].as_array().into_iter().flatten() {
            if part["thought"] == Value::Bool(true) {
                continue;
            }
            if let Some(t) = part["text"].as_str() {
                if role == "model" {
                    p.assistant_text(t)
                } else {
                    p.user_text(t)
                }
            } else if part["functionCall"].is_object() {
                let fc = &part["functionCall"];
                let name = fc["name"].as_str().unwrap_or("tool").to_string();
                let id = p.tool_call(fc["id"].as_str(), &name, fc["args"].clone());
                pending.push((name, id));
            } else if part["functionResponse"].is_object() {
                let fr = &part["functionResponse"];
                let name = fr["name"].as_str().unwrap_or("tool").to_string();
                let id = fr["id"].as_str().map(str::to_string).unwrap_or_else(|| {
                    pending
                        .iter()
                        .position(|(n, _)| *n == name)
                        .map(|i| pending.remove(i).1)
                        .unwrap_or_else(|| "probe_call_x".into())
                });
                p.tool_result(&id, Some(&name), fr["response"].clone());
            } else if role != "model" {
                p.user_text("[non-text part]");
            }
        }
    }
}

fn converse_messages(p: &mut Prompt, body: &Value) {
    p.system(texts_of(&body["system"]));
    for m in body["messages"].as_array().into_iter().flatten() {
        let role = m["role"].as_str().unwrap_or("user");
        for b in m["content"].as_array().into_iter().flatten() {
            if let Some(t) = b["text"].as_str() {
                if role == "assistant" {
                    p.assistant_text(t)
                } else {
                    p.user_text(t)
                }
            } else if b["toolUse"].is_object() {
                let tu = &b["toolUse"];
                p.tool_call(
                    tu["toolUseId"].as_str(),
                    tu["name"].as_str().unwrap_or("tool"),
                    tu["input"].clone(),
                );
            } else if b["toolResult"].is_object() {
                let tr = &b["toolResult"];
                let id = tr["toolUseId"].as_str().unwrap_or("tooluse_0").to_string();
                let content = &tr["content"];
                let out = content
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|c| c.get("json").cloned())
                    .unwrap_or_else(|| Value::String(texts_of(content)));
                p.tool_result(&id, None, out);
            }
        }
    }
}

fn responses_messages(p: &mut Prompt, body: &Value) {
    if let Some(s) = body["instructions"].as_str() {
        p.system(s.to_string());
    }
    match &body["input"] {
        Value::String(s) => p.user_text(s),
        Value::Array(items) => {
            for it in items {
                let ty = it["type"].as_str().unwrap_or("message");
                match ty {
                    "message" => {
                        let role = it["role"].as_str().unwrap_or("user");
                        let t = texts_of(&it["content"]);
                        match role {
                            "system" | "developer" => p.system(t),
                            "assistant" => p.assistant_text(&t),
                            _ => p.user_text(&t),
                        }
                    }
                    "function_call" => {
                        p.tool_call(
                            it["call_id"].as_str(),
                            it["name"].as_str().unwrap_or("tool"),
                            parse_args(&it["arguments"]),
                        );
                    }
                    "function_call_output" => {
                        let id = it["call_id"].as_str().unwrap_or("call_0").to_string();
                        let out = match &it["output"] {
                            Value::String(s) => Value::String(s.clone()),
                            other => Value::String(texts_of(other)),
                        };
                        p.tool_result(&id, None, out);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn schema(v: &Value) -> Value {
    if v.is_object() {
        v.clone()
    } else {
        json!({"type": "object", "properties": {}})
    }
}

fn fn_tool(name: &Value, desc: &Value, params: &Value) -> Option<Value> {
    let name = name.as_str()?;
    let mut t = json!({"type": "function", "name": name, "inputSchema": schema(params)});
    if let Some(d) = desc.as_str() {
        t["description"] = json!(d);
    }
    Some(t)
}

fn derive_tools(kind: Kind, b: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    match kind {
        Kind::Chat | Kind::Cohere => {
            for t in b["tools"].as_array().into_iter().flatten() {
                let f = &t["function"];
                out.extend(fn_tool(&f["name"], &f["description"], &f["parameters"]));
            }
        }
        Kind::Responses => {
            for t in b["tools"].as_array().into_iter().flatten() {
                if t["type"] == "function" {
                    out.extend(fn_tool(&t["name"], &t["description"], &t["parameters"]));
                }
            }
        }
        Kind::Messages => {
            for t in b["tools"].as_array().into_iter().flatten() {
                if t.get("input_schema").is_some() {
                    out.extend(fn_tool(&t["name"], &t["description"], &t["input_schema"]));
                }
            }
        }
        Kind::Gemini => {
            for t in b["tools"].as_array().into_iter().flatten() {
                for d in t["functionDeclarations"].as_array().into_iter().flatten() {
                    let params = if d["parameters"].is_object() {
                        &d["parameters"]
                    } else {
                        &d["parametersJsonSchema"]
                    };
                    out.extend(fn_tool(&d["name"], &d["description"], params));
                }
            }
        }
        Kind::Converse => {
            for t in b["toolConfig"]["tools"].as_array().into_iter().flatten() {
                let s = &t["toolSpec"];
                out.extend(fn_tool(&s["name"], &s["description"], &s["inputSchema"]["json"]));
            }
        }
    }
    out
}

fn derive_tool_choice(kind: Kind, b: &Value) -> Option<Value> {
    let named = |n: &Value| n.as_str().map(|n| json!({"type": "tool", "toolName": n}));
    match kind {
        Kind::Chat | Kind::Cohere | Kind::Responses => match &b["tool_choice"] {
            Value::String(s) if ["auto", "none", "required"].contains(&s.as_str()) => {
                Some(json!({"type": s}))
            }
            Value::String(s) if s == "REQUIRED" => Some(json!({"type": "required"})),
            Value::String(s) if s == "NONE" => Some(json!({"type": "none"})),
            Value::Object(o) => named(&o.get("function").map_or(
                o.get("name").cloned().unwrap_or(Value::Null),
                |f| f["name"].clone(),
            )),
            _ => None,
        },
        Kind::Messages => match b["tool_choice"]["type"].as_str() {
            Some("auto") => Some(json!({"type": "auto"})),
            Some("any") => Some(json!({"type": "required"})),
            Some("none") => Some(json!({"type": "none"})),
            Some("tool") => named(&b["tool_choice"]["name"]),
            _ => None,
        },
        Kind::Gemini => {
            let c = &b["toolConfig"]["functionCallingConfig"];
            match c["mode"].as_str() {
                Some("AUTO") => Some(json!({"type": "auto"})),
                Some("NONE") => Some(json!({"type": "none"})),
                Some("ANY") => match c["allowedFunctionNames"].as_array() {
                    Some(a) if a.len() == 1 => named(&a[0]),
                    _ => Some(json!({"type": "required"})),
                },
                _ => None,
            }
        }
        Kind::Converse => {
            let c = &b["toolConfig"]["toolChoice"];
            if c["auto"].is_object() {
                Some(json!({"type": "auto"}))
            } else if c["any"].is_object() {
                Some(json!({"type": "required"}))
            } else if c["tool"].is_object() {
                named(&c["tool"]["name"])
            } else {
                None
            }
        }
    }
}

fn first_num<'a>(cands: &[&'a Value]) -> Option<&'a Value> {
    cands.iter().copied().find(|v| v.is_number())
}

fn derive_options(kind: Kind, b: &Value) -> Value {
    let mut p = Prompt::new();
    match kind {
        Kind::Chat | Kind::Cohere => {
            openai_messages(&mut p, b["messages"].as_array().map_or(&[][..], |v| v))
        }
        Kind::Messages => anthropic_messages(&mut p, b),
        Kind::Gemini => gemini_messages(&mut p, b),
        Kind::Converse => converse_messages(&mut p, b),
        Kind::Responses => responses_messages(&mut p, b),
    }
    let gc = &b["generationConfig"];
    let ic = &b["inferenceConfig"];
    let mut o = Map::new();
    o.insert("prompt".into(), Value::Array(p.finish()));
    if let Some(v) = first_num(&[
        &b["max_tokens"],
        &b["max_completion_tokens"],
        &b["max_output_tokens"],
        &gc["maxOutputTokens"],
        &ic["maxTokens"],
    ]) {
        o.insert("maxOutputTokens".into(), v.clone());
    }
    if let Some(v) = first_num(&[&b["temperature"], &gc["temperature"], &ic["temperature"]]) {
        o.insert("temperature".into(), v.clone());
    }
    if let Some(v) = first_num(&[&b["top_p"], &b["p"], &gc["topP"], &ic["topP"]]) {
        o.insert("topP".into(), v.clone());
    }
    if let Some(v) = first_num(&[&b["top_k"], &b["k"], &gc["topK"]]) {
        o.insert("topK".into(), v.clone());
    }
    if let Some(v) = first_num(&[&b["seed"], &gc["seed"]]) {
        o.insert("seed".into(), v.clone());
    }
    if let Some(v) = first_num(&[&b["presence_penalty"], &gc["presencePenalty"]]) {
        o.insert("presencePenalty".into(), v.clone());
    }
    if let Some(v) = first_num(&[&b["frequency_penalty"], &gc["frequencyPenalty"]]) {
        o.insert("frequencyPenalty".into(), v.clone());
    }
    let stops = [
        &b["stop"],
        &b["stop_sequences"],
        &gc["stopSequences"],
        &ic["stopSequences"],
    ];
    for s in stops {
        match s {
            Value::String(x) => {
                o.insert("stopSequences".into(), json!([x]));
                break;
            }
            Value::Array(a) if !a.is_empty() => {
                o.insert("stopSequences".into(), Value::Array(a.clone()));
                break;
            }
            _ => {}
        }
    }
    let tools = derive_tools(kind, b);
    if !tools.is_empty() {
        o.insert("tools".into(), Value::Array(tools));
        if let Some(tc) = derive_tool_choice(kind, b) {
            o.insert("toolChoice".into(), tc);
        }
    }
    // response format
    let rf = &b["response_format"];
    let tf = &b["text"]["format"];
    if rf["type"] == "json_schema" {
        let js = &rf["json_schema"];
        o.insert(
            "responseFormat".into(),
            json!({"type": "json", "schema": schema(&js["schema"]), "name": js["name"].as_str().unwrap_or("response")}),
        );
    } else if rf["type"] == "json_object" {
        o.insert("responseFormat".into(), json!({"type": "json"}));
    } else if tf["type"] == "json_schema" {
        o.insert(
            "responseFormat".into(),
            json!({"type": "json", "schema": schema(&tf["schema"]), "name": tf["name"].as_str().unwrap_or("response")}),
        );
    } else if gc["responseMimeType"] == "application/json" {
        let s = if gc["responseSchema"].is_object() {
            &gc["responseSchema"]
        } else {
            &gc["responseJsonSchema"]
        };
        if s.is_object() {
            o.insert("responseFormat".into(), json!({"type": "json", "schema": s}));
        } else {
            o.insert("responseFormat".into(), json!({"type": "json"}));
        }
    }
    Value::Object(o)
}

fn call_options(kind: Kind, body: &Value) -> (CallOptions, Option<String>) {
    let v = derive_options(kind, body);
    match serde_json::from_value::<CallOptions>(v) {
        Ok(o) => (o, None),
        Err(e) => {
            let o = serde_json::from_value::<CallOptions>(
                json!({"prompt": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}]}),
            )
            .unwrap();
            (o, Some(format!("derive fallback: {e}")))
        }
    }
}

// ─── routes ─────────────────────────────────────────────────────────────────

const KEY: &str = "probe-key";

fn creds() -> AwsCredentials {
    AwsCredentials {
        access_key_id: "AKIDPROBE".into(),
        secret_access_key: "probe-secret".into(),
        session_token: None,
        region: "us-east-1".into(),
    }
}

fn routes_for(kind: Kind, vendor: &str) -> Vec<String> {
    let mut r: Vec<String> = match kind {
        Kind::Chat => vec![
            "compat", "openai.chat", "deepseek", "groq", "mistral",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        Kind::Responses => vec![
            "openai.responses",
            "xai.responses",
            "hf.responses",
            "open_responses",
            "codex.chatgpt",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        Kind::Messages => vec![
            "anthropic",
            "vertex.anthropic",
            "anthropic_aws.key",
            "anthropic_aws.sigv4",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        Kind::Gemini => vec!["google", "vertex", "vertex.express"]
            .into_iter()
            .map(String::from)
            .collect(),
        Kind::Converse => vec!["bedrock.sigv4", "bedrock.key"]
            .into_iter()
            .map(String::from)
            .collect(),
        Kind::Cohere => vec!["cohere".to_string()],
    };
    if kind == Kind::Chat
        && !["openai", "deepseek", "groq", "mistral", "gemini", "huggingface", "chatgpt"]
            .contains(&vendor)
        && aimux_providers::preset::lookup(vendor).is_some()
    {
        r.push(format!("preset.{vendor}"));
    }
    r
}

fn build(
    route: &str,
    model: &str,
    chat_prefix: &str,
    fetch: FetchFunction,
) -> Result<Arc<dyn LanguageModel>, String> {
    use aimux_providers::*;
    let e = |e: aimux_core::AiMuxError| format!("factory error: {e}");
    let f = Some(fetch);
    Ok(match route {
        "compat" => Arc::new(
            openai_compatible::create_openai_compatible(
                openai_compatible::OpenAICompatibleProviderSettings {
                    name: "compat".into(),
                    base_url: format!("https://compat.probe.test{chat_prefix}"),
                    api_key: Some(KEY.into()),
                    fetch: f,
                    ..Default::default()
                },
            )
            .map_err(e)?
            .chat(model),
        ),
        "openai.chat" | "openai.responses" => {
            let p = create_openai(OpenAIProviderSettings {
                api_key: Some(KEY.into()),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?;
            if route == "openai.chat" {
                Arc::new(p.chat(model))
            } else {
                Arc::new(p.responses(model))
            }
        }
        "deepseek" => Arc::new(
            deepseek::create_deepseek(deepseek::DeepSeekProviderSettings {
                api_key: Some(KEY.into()),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .chat(model),
        ),
        "groq" => Arc::new(
            groq::create_groq(groq::GroqProviderSettings {
                api_key: Some(KEY.into()),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .chat(model),
        ),
        "mistral" => Arc::new(
            create_mistral(MistralProviderSettings {
                api_key: Some(KEY.into()),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .chat(model),
        ),
        "azure.chat" | "azure.responses" => {
            let p = create_azure(AzureOpenAIProviderSettings {
                resource_name: Some("probe-res".into()),
                api_key: Some(Resolvable::Value(KEY.into())),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?;
            if route == "azure.chat" {
                Arc::new(p.chat(model))
            } else {
                Arc::new(p.responses(model))
            }
        }
        "xai.responses" => Arc::new(
            create_xai(XAIProviderSettings {
                api_key: Some(KEY.into()),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .responses(model),
        ),
        "hf.responses" => Arc::new(
            create_huggingface(HuggingFaceProviderSettings {
                api_key: Some(Resolvable::Value(KEY.into())),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .responses(model),
        ),
        "open_responses" => Arc::new(
            create_open_responses(OpenResponsesProviderSettings {
                name: "probe-or".into(),
                base_url: "https://or.probe.test/v1".into(),
                api_key: Some(Resolvable::Value(KEY.into())),
                fetch: f,
                headers: None,
                user_agent_suffix: None,
                transform_request_body: None,
            })
            .map_err(e)?
            .responses(model),
        ),
        "codex.apikey" | "codex.chatgpt" => {
            let mode = if route == "codex.apikey" {
                CodexMode::ApiKey(Some(Resolvable::Value(KEY.into())))
            } else {
                CodexMode::ChatGptAccount {
                    token: Resolvable::Value("probe-token".into()),
                    account_id: Some("probe-account".into()),
                }
            };
            Arc::new(
                create_codex(CodexProviderSettings {
                    mode,
                    fetch: f,
                    ..Default::default()
                })
                .map_err(e)?
                .responses(model),
            )
        }
        "anthropic" => Arc::new(
            create_anthropic(AnthropicProviderSettings {
                api_key: Some(KEY.into()),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .messages(model),
        ),
        "vertex.anthropic" => create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project: Some("probe-proj".into()),
            location: Some("us-east5".into()),
            fetch: f,
            generate_auth_token: Some(Resolvable::Value(Some("probe-gtoken".into()))),
            ..Default::default()
        })
        .map_err(e)?
        .call(model),
        "anthropic_aws.key" | "anthropic_aws.sigv4" => {
            let auth = if route == "anthropic_aws.key" {
                AnthropicAwsAuth::ApiKey(Resolvable::Value(KEY.into()))
            } else {
                AnthropicAwsAuth::SigV4(Resolvable::Value(creds()))
            };
            Arc::new(
                create_anthropic_aws(AnthropicAwsProviderSettings {
                    region: Some("us-east-1".into()),
                    auth: Some(auth),
                    workspace_id: Some("probe-ws".into()),
                    fetch: f,
                    ..Default::default()
                })
                .map_err(e)?
                .messages(model),
            )
        }
        "google" => Arc::new(
            create_google(GoogleProviderSettings {
                api_key: Some(KEY.into()),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .generative_ai(model),
        ),
        "vertex" => Arc::new(
            create_google_vertex(VertexProviderSettings {
                project: Some("probe-proj".into()),
                location: Some("us-central1".into()),
                google_auth_options: Some(GoogleAuthOptions {
                    auth_client: Some(Resolvable::Value(Some("probe-gtoken".into()))),
                    ..Default::default()
                }),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .chat(model),
        ),
        "vertex.express" => Arc::new(
            create_google_vertex(VertexProviderSettings {
                api_key: Some(Resolvable::Value(KEY.into())),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .chat(model),
        ),
        "bedrock.sigv4" | "bedrock.key" => {
            let mut s = AmazonBedrockProviderSettings {
                region: Some("us-east-1".into()),
                fetch: f,
                ..Default::default()
            };
            if route == "bedrock.key" {
                s.api_key = Some(Resolvable::Value(KEY.into()));
            } else {
                s.access_key_id = Some("AKIDPROBE".into());
                s.secret_access_key = Some("probe-secret".into());
            }
            Arc::new(create_amazon_bedrock(s).map_err(e)?.chat(model))
        }
        "cohere" => Arc::new(
            create_cohere(CohereProviderSettings {
                api_key: Some(KEY.into()),
                fetch: f,
                ..Default::default()
            })
            .map_err(e)?
            .chat(model),
        ),
        r if r.starts_with("preset.") => {
            let name = &r["preset.".len()..];
            let mut params = HashMap::new();
            if let Some(entry) = preset::lookup(name) {
                for p in entry.descriptor.params.iter().filter(|p| p.derive.is_none()) {
                    params.insert(p.name.to_string(), "probe".to_string());
                }
            }
            let mk = |params: HashMap<String, String>, fetch: Option<FetchFunction>| {
                create_provider(
                    name,
                    PresetSettings {
                        api_key: Some(Resolvable::Value(KEY.into())),
                        fetch,
                        params,
                        ..Default::default()
                    },
                )
            };
            // Derived/undeclared params are refused: retry without params.
            let p = match mk(params, f.clone()) {
                Ok(p) => p,
                Err(_) => mk(HashMap::new(), f).map_err(e)?,
            };
            p.language_model(model).map_err(e)?
        }
        other => return Err(format!("unknown route {other}")),
    })
}

// ─── masking ────────────────────────────────────────────────────────────────

struct Masker {
    patterns: Vec<(Regex, &'static str)>,
    now_secs: i64,
    token: Regex,
    uuid: Regex,
}

impl Masker {
    fn new() -> Self {
        let now = chrono::Utc::now();
        let days: Vec<chrono::DateTime<chrono::Utc>> = [-1i64, 0, 1]
            .iter()
            .map(|d| now + chrono::Duration::days(*d))
            .collect();
        let iso = days
            .iter()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .collect::<Vec<_>>()
            .join("|");
        let compact = days
            .iter()
            .map(|d| d.format("%Y%m%d").to_string())
            .collect::<Vec<_>>()
            .join("|");
        let http_date = days
            .iter()
            .map(|d| d.format("%d %b %Y").to_string())
            .collect::<Vec<_>>()
            .join("|");
        Self {
            patterns: vec![
                (
                    Regex::new(&format!(
                        r"(?:{iso})[T ]\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:?\d\d)?"
                    ))
                    .unwrap(),
                    "<NOW-ISO>",
                ),
                (
                    Regex::new(&format!(r"(?:{compact})T\d{{6}}Z")).unwrap(),
                    "<NOW-AMZ>",
                ),
                (
                    Regex::new(&format!(r"\b(?:{compact})\b")).unwrap(),
                    "<NOW-DAY>",
                ),
                (
                    Regex::new(&format!(r"\w{{3}}, (?:{http_date}) \d\d:\d\d:\d\d GMT")).unwrap(),
                    "<NOW-HTTP>",
                ),
                (
                    Regex::new(r"Signature=[0-9a-f]{64}").unwrap(),
                    "Signature=<SIG>",
                ),
            ],
            now_secs: now.timestamp(),
            token: Regex::new(r"[A-Za-z0-9]{8,}").unwrap(),
            uuid: Regex::new(
                r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
            )
            .unwrap(),
        }
    }

    fn pattern_mask(&self, v: &mut Value) {
        match v {
            Value::String(s) => {
                for (re, rep) in &self.patterns {
                    if re.is_match(s) {
                        *s = re.replace_all(s, *rep).into_owned();
                    }
                }
            }
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    let d = 3 * 86_400;
                    if (i - self.now_secs).abs() < d {
                        *v = json!("<NOW-EPOCH-S>");
                    } else if (i - self.now_secs * 1000).abs() < d * 1000 {
                        *v = json!("<NOW-EPOCH-MS>");
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| self.pattern_mask(x)),
            Value::Object(o) => o.values_mut().for_each(|x| self.pattern_mask(x)),
            _ => {}
        }
    }

    fn tokens(&self, v: &Value, out: &mut HashSet<String>) {
        match v {
            Value::String(s) => {
                for m in self.token.find_iter(s) {
                    out.insert(m.as_str().to_string());
                }
            }
            Value::Array(a) => a.iter().for_each(|x| self.tokens(x, out)),
            Value::Object(o) => o.iter().for_each(|(k, x)| {
                for m in self.token.find_iter(k) {
                    out.insert(m.as_str().to_string());
                }
                self.tokens(x, out)
            }),
            _ => {}
        }
    }

    fn replace(
        &self,
        s: &str,
        nondet: &HashSet<String>,
        uuid: &Regex,
        map: &mut HashMap<String, String>,
    ) -> String {
        let s = uuid
            .replace_all(s, |c: &regex::Captures| {
                let k = c[0].to_ascii_lowercase();
                let n = map.len() + 1;
                map.entry(k).or_insert_with(|| format!("<U{n}>")).clone()
            })
            .into_owned();
        self.token
            .replace_all(&s, |c: &regex::Captures| {
                let k = c[0].to_string();
                if nondet.contains(&k) {
                    let n = map.len() + 1;
                    map.entry(k).or_insert_with(|| format!("<R{n}>")).clone()
                } else {
                    k
                }
            })
            .into_owned()
    }

    fn apply(
        &self,
        v: &Value,
        nondet: &HashSet<String>,
        uuid: &Regex,
        map: &mut HashMap<String, String>,
    ) -> Value {
        match v {
            Value::String(s) => Value::String(self.replace(s, nondet, uuid, map)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.apply(x, nondet, uuid, map)).collect()),
            Value::Object(o) => {
                let mut out = Map::new();
                for (k, x) in o {
                    let k2 = self.replace(k, nondet, uuid, map);
                    out.insert(k2, self.apply(x, nondet, uuid, map));
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }

    /// Mask `a` and `b` (two runs of the same op); returns (masked a, masked b).
    fn mask_pair(&self, mut a: Value, mut b: Value) -> (Value, Value) {
        self.pattern_mask(&mut a);
        self.pattern_mask(&mut b);
        let (mut ta, mut tb) = (HashSet::new(), HashSet::new());
        self.tokens(&a, &mut ta);
        self.tokens(&b, &mut tb);
        let nondet: HashSet<String> = ta.symmetric_difference(&tb).cloned().collect();
        let uuid = &self.uuid;
        let ma = self.apply(&a, &nondet, uuid, &mut HashMap::new());
        let mb = self.apply(&b, &nondet, uuid, &mut HashMap::new());
        (ma, mb)
    }
}

// ─── running one op ─────────────────────────────────────────────────────────

#[derive(Clone)]
struct OpSpec {
    key: String,
    route: String,
    model: String,
    chat_prefix: String,
    canned: Canned,
    stream: bool,
    mode: BodyMode,
    raw: bool,
    options: CallOptions,
    note: Option<String>,
}

/// Replace a large echoed body string (`response.body` / `request.body` in a
/// result) by its length and a deterministic hash: it is the cassette bytes or
/// the request already recorded by the transport, and masking it is costly.
fn digest_bodies(v: &mut Value) {
    use std::hash::{Hash, Hasher};
    for key in ["request", "response"] {
        if let Some(b) = v.get_mut(key).and_then(|x| x.get_mut("body"))
            && !b.is_null()
        {
            let s = b.to_string();
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            *b = json!(format!("<body len={} hash={:016x}>", s.len(), h.finish()));
        }
    }
}

fn err_json(e: &aimux_core::AiMuxError) -> Value {
    json!({
        "display": e.to_string(),
        "wire": serde_json::to_value(e).unwrap_or_else(|x| json!(format!("unserializable: {x}"))),
    })
}

async fn run_once(spec: OpSpec) -> Value {
    let fetch = Arc::new(ProbeFetch {
        canned: spec.canned.clone(),
        mode: spec.mode,
        seen: Mutex::new(Vec::new()),
    });
    let ff: FetchFunction = fetch.clone();
    let outcome = match build(&spec.route, &spec.model, &spec.chat_prefix, ff) {
        Err(e) => json!({ "build_error": e }),
        Ok(model) => {
            let mut opts = spec.options.clone();
            if spec.raw {
                opts.include_raw_chunks = Some(true);
            }
            let stream = spec.stream;
            let task = tokio::spawn(async move {
                let meta = json!({"provider": model.provider(), "modelId": model.model_id()});
                if !stream {
                    match model.do_generate(&opts).await {
                        Ok(r) => {
                            let mut v = serde_json::to_value(&r).unwrap();
                            digest_bodies(&mut v);
                            json!({"meta": meta, "ok": v})
                        }
                        Err(e) => json!({"meta": meta, "err": err_json(&e)}),
                    }
                } else {
                    match model.do_stream(&opts).await {
                        Err(e) => json!({"meta": meta, "err": err_json(&e)}),
                        Ok(sr) => {
                            let request = serde_json::to_value(&sr.request).unwrap();
                            let response = serde_json::to_value(&sr.response).unwrap();
                            let mut parts = Vec::new();
                            let mut s = sr.stream;
                            while let Some(p) = s.next().await {
                                parts.push(match p {
                                    Ok(p) => json!({"part": serde_json::to_value(&p).unwrap()}),
                                    Err(e) => json!({"error": err_json(&e)}),
                                });
                            }
                            let mut rr = json!({"request": request, "response": response});
                            digest_bodies(&mut rr);
                            json!({"meta": meta, "request": rr["request"], "response": rr["response"], "parts": parts})
                        }
                    }
                }
            });
            match tokio::time::timeout(OP_TIMEOUT, task).await {
                Err(_) => json!({"timeout": true}),
                Ok(Err(join)) => json!({"panic": join.to_string()}),
                Ok(Ok(v)) => v,
            }
        }
    };
    let seen = fetch.seen.lock().unwrap().clone();
    json!({ "requests": seen, "outcome": outcome })
}

async fn run_op(spec: OpSpec, masker: Arc<Masker>) -> (String, Value, bool) {
    let t0 = std::time::Instant::now();
    let a = run_once(spec.clone()).await;
    if std::env::var("PROBE_TRACE").is_ok() {
        eprintln!("probe-trace {:>6}ms {}", t0.elapsed().as_millis(), spec.key);
    }
    let b = run_once(spec.clone()).await;
    let (ma, mb) = masker.mask_pair(a, b);
    let stable = ma == mb;
    let mut line = json!({
        "key": spec.key,
        "route": spec.route,
        "model": spec.model,
        "stream": spec.stream,
        "mode": format!("{:?}", spec.mode),
        "raw": spec.raw,
    });
    if let Some(n) = &spec.note {
        line["note"] = json!(n);
    }
    line["requests"] = ma["requests"].clone();
    line["outcome"] = ma["outcome"].clone();
    if !stable {
        line["unstable_b"] = mb;
    }
    (spec.key, line, stable)
}

// ─── preset sweep fixtures ──────────────────────────────────────────────────

fn sweep_json() -> Canned {
    let body = json!({
        "id": "chatcmpl-probe1", "object": "chat.completion", "created": 1_700_000_000,
        "model": "probe-model", "system_fingerprint": "fp_probe",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": "Hi there.", "reasoning_content": "thinking...",
            "tool_calls": [{"id": "call_p1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}}]
        }, "logprobs": null}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 40, "total_tokens": 140, "cached_tokens": 7,
            "prompt_tokens_details": {"cached_tokens": 30, "cache_creation_input_tokens": 11, "cache_write_tokens": 13, "audio_tokens": 2},
            "completion_tokens_details": {"reasoning_tokens": 9, "accepted_prediction_tokens": 3, "rejected_prediction_tokens": 1, "audio_tokens": 0},
            "prompt_cache_hit_tokens": 21, "prompt_cache_miss_tokens": 79},
        "citations": ["https://example.com/a"]
    });
    Canned {
        status: 200,
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("x-request-id".into(), "req_probe".into()),
        ],
        body: Bytes::from(serde_json::to_vec(&body).unwrap()),
    }
}

fn sweep_sse() -> Canned {
    let chunks = [
        json!({"id":"chatcmpl-probe2","object":"chat.completion.chunk","created":1_700_000_000,"model":"probe-model","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}),
        json!({"id":"chatcmpl-probe2","object":"chat.completion.chunk","created":1_700_000_000,"model":"probe-model","choices":[{"index":0,"delta":{"reasoning_content":"thinking"},"finish_reason":null}]}),
        json!({"id":"chatcmpl-probe2","object":"chat.completion.chunk","created":1_700_000_000,"model":"probe-model","choices":[{"index":0,"delta":{"content":"Hi "},"finish_reason":null}]}),
        json!({"id":"chatcmpl-probe2","object":"chat.completion.chunk","created":1_700_000_000,"model":"probe-model","choices":[{"index":0,"delta":{"content":"there."},"finish_reason":null}]}),
        json!({"id":"chatcmpl-probe2","object":"chat.completion.chunk","created":1_700_000_000,"model":"probe-model","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_p2","type":"function","function":{"name":"get_weather","arguments":"{\"ci"}}]},"finish_reason":null}]}),
        json!({"id":"chatcmpl-probe2","object":"chat.completion.chunk","created":1_700_000_000,"model":"probe-model","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ty\":\"Paris\"}"}}]},"finish_reason":null}]}),
        json!({"id":"chatcmpl-probe2","object":"chat.completion.chunk","created":1_700_000_000,"model":"probe-model","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        json!({"id":"chatcmpl-probe2","object":"chat.completion.chunk","created":1_700_000_000,"model":"probe-model","choices":[],"usage":{"prompt_tokens":100,"completion_tokens":40,"total_tokens":140,"cached_tokens":7,"prompt_tokens_details":{"cached_tokens":30,"cache_creation_input_tokens":11,"cache_write_tokens":13},"completion_tokens_details":{"reasoning_tokens":9,"accepted_prediction_tokens":3,"rejected_prediction_tokens":1}}}),
    ];
    let mut s = String::new();
    for c in chunks {
        s.push_str(&format!("data: {c}\n\n"));
    }
    s.push_str("data: [DONE]\n\n");
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: Bytes::from(s.into_bytes()),
    }
}

fn sweep_options() -> CallOptions {
    serde_json::from_value(json!({
        "prompt": [
            {"role": "system", "content": "Be brief."},
            {"role": "user", "content": [{"type": "text", "text": "Weather in Paris?"}]}
        ],
        "maxOutputTokens": 123, "temperature": 0.3, "topP": 0.9, "seed": 7,
        "stopSequences": ["STOP"],
        "tools": [{"type": "function", "name": "get_weather", "description": "Get weather",
                   "inputSchema": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}}],
        "toolChoice": {"type": "auto"}
    }))
    .unwrap()
}

// ─── main ───────────────────────────────────────────────────────────────────

fn stream_variants() -> Vec<(&'static str, BodyMode, bool)> {
    vec![
        ("stream", BodyMode::Whole, false),
        ("stream.raw", BodyMode::Whole, true),
        ("stream.chunk7", BodyMode::Chunked(7), false),
        ("stream.trunc_err", BodyMode::TruncErr, false),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zz_diff_probe() {
    let Ok(out_path) = std::env::var("PROBE_OUT") else {
        eprintln!("PROBE_OUT not set; probe skipped");
        return;
    };
    let filter = std::env::var("PROBE_FILTER").ok();
    let masker = Arc::new(Masker::new());
    let cassettes = load_cassettes();
    let total = cassettes.len();

    let mut specs: Vec<OpSpec> = Vec::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut covered_cassettes = 0usize;
    for c in &cassettes {
        let kind = match classify(c).and_then(|k| {
            if c.canned.body.len() > MAX_BODY {
                Err(format!("response body > {} KiB (time budget)", MAX_BODY / 1024))
            } else {
                Ok(k)
            }
        }) {
            Ok(k) => k,
            Err(reason) => {
                *skipped.entry(format!("{}: {reason}", c.vendor)).or_default() += 1;
                continue;
            }
        };
        covered_cassettes += 1;
        let model = model_id(c, kind);
        let (options, note) = call_options(kind, &c.req_body);
        let p = c.path.split('?').next().unwrap_or("");
        let chat_prefix = p
            .strip_suffix("/chat/completions")
            .unwrap_or("")
            .to_string();
        for route in routes_for(kind, &c.vendor) {
            let base = format!("{}/{}|{}", c.vendor, c.file, route);
            let mut ops: Vec<(&str, BodyMode, bool, bool)> =
                vec![("generate", BodyMode::Whole, false, false)];
            if c.is_stream {
                for (n, m, raw) in stream_variants() {
                    ops.push((n, m, raw, true));
                }
            }
            for (op, mode, raw, stream) in ops {
                if filter.as_deref().is_some_and(|f| !format!("{base}|{op}").contains(f)) {
                    continue;
                }
                specs.push(OpSpec {
                    key: format!("{base}|{op}"),
                    route: route.clone(),
                    model: model.clone(),
                    chat_prefix: chat_prefix.clone(),
                    canned: c.canned.clone(),
                    stream,
                    mode,
                    raw,
                    options: options.clone(),
                    note: note.clone(),
                });
            }
        }
    }
    // Preset sweep: every registry preset against a synthetic rich-usage chat.
    let mut preset_names: Vec<&str> = aimux_providers::preset::names().collect();
    preset_names.sort_unstable();
    for name in &preset_names {
        let route = format!("preset.{name}");
        specs.push(OpSpec {
            key: format!("~sweep/{name}|{route}|generate"),
            route: route.clone(),
            model: "probe-model".into(),
            chat_prefix: String::new(),
            canned: sweep_json(),
            stream: false,
            mode: BodyMode::Whole,
            raw: false,
            options: sweep_options(),
            note: None,
        });
        for (op, mode, raw) in stream_variants() {
            specs.push(OpSpec {
                key: format!("~sweep/{name}|{route}|{op}"),
                route: route.clone(),
                model: "probe-model".into(),
                chat_prefix: String::new(),
                canned: sweep_sse(),
                stream: true,
                mode,
                raw,
                options: sweep_options(),
                note: None,
            });
        }
    }
    if let Some(f) = &filter {
        specs.retain(|s| s.key.contains(f.as_str()));
    }
    let n_specs = specs.len();
    eprintln!("probe: {total} cassettes, {covered_cassettes} LLM cassettes, {n_specs} ops");

    let results: Vec<(String, Value, bool)> = futures::stream::iter(
        specs
            .into_iter()
            .map(|s| {
                let m = masker.clone();
                async move { tokio::spawn(run_op(s, m)).await.unwrap() }
            }),
    )
    .buffer_unordered(32)
    .collect()
    .await;

    let mut lines: Vec<(String, String)> = Vec::new();
    let mut unstable = 0usize;
    for (key, line, stable) in results {
        if !stable {
            unstable += 1;
        }
        lines.push((key, serde_json::to_string(&line).unwrap()));
    }
    lines.sort();
    let mut text = String::new();
    for (_, l) in &lines {
        text.push_str(l);
        text.push('\n');
    }
    fs::write(&out_path, text).unwrap();
    let summary = json!({
        "total_cassettes": total,
        "llm_cassettes": covered_cassettes,
        "ops": n_specs,
        "presets": preset_names.len(),
        "unstable_in_process": unstable,
        "skipped": skipped,
    });
    fs::write(
        Path::new(&format!("{out_path}.summary.json")),
        serde_json::to_string_pretty(&summary).unwrap(),
    )
    .unwrap();
    eprintln!("probe: wrote {} lines to {out_path}; unstable_in_process={unstable}", lines.len());
}
