//! Tool-call accumulator for a chat-completions stream, ported from a TS
//! `TransformStream` (`flush` emits whatever is still open).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One part emitted to the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamPart {
    /// "tool-input-start" | "tool-input-delta" | "tool-call" | "finish"
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// One tool-call delta from the provider. `index: -1` means "the most
/// recently started call".
#[derive(Debug, Clone, Default)]
pub struct Delta {
    pub index: i64,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

struct ToolCall {
    id: String,
    name: String,
    arguments: String,
    has_finished: bool,
    #[allow(dead_code)]
    index: i64,
}

pub struct Accumulator {
    calls: Vec<ToolCall>,
    by_index: HashMap<i64, usize>,
    finished: bool,
    log: Arc<Mutex<Vec<String>>>,
}

fn part(kind: &str) -> StreamPart {
    StreamPart {
        kind: kind.to_string(),
        id: None,
        tool_name: None,
        delta: None,
        input: None,
        finish_reason: None,
    }
}

impl Accumulator {
    /// `log` receives the `type` of every emitted part, in order.
    pub fn new(log: Arc<Mutex<Vec<String>>>) -> Self {
        Accumulator {
            calls: Vec::new(),
            by_index: HashMap::new(),
            finished: false,
            log,
        }
    }

    fn find_call(&self, index: i64) -> i64 {
        if index == -1 {
            return self.calls.len() as i64 - 1;
        }
        match self.by_index.get(&index) {
            Some(i) => *i as i64,
            None => -1,
        }
    }

    fn record(&self, parts: &Vec<StreamPart>) {
        let mut log = self.log.lock().unwrap();
        for p in parts {
            log.push(p.kind.clone());
        }
    }

    /// Feeds one delta. With `emit_deltas` false, only `tool-input-start`
    /// and `tool-call` parts are emitted.
    pub fn push(&mut self, delta: &Delta, emit_deltas: bool) -> Result<Vec<StreamPart>, String> {
        let mut out = Vec::new();
        if self.finished {
            return Ok(out);
        }
        let found = self.find_call(delta.index);
        let i;
        if found == -1 {
            if delta.id.is_none() {
                return Err("Expected 'id' to be a string.".to_string());
            }
            if delta.name.is_none() {
                return Err("Expected 'function.name' to be a string.".to_string());
            }
            let id = delta.id.clone().unwrap();
            let name = delta.name.clone().unwrap();
            self.calls.push(ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: String::new(),
                has_finished: false,
                index: delta.index,
            });
            i = self.calls.len() - 1;
            if delta.index != -1 {
                self.by_index.insert(delta.index, i);
            }
            let mut start = part("tool-input-start");
            start.id = Some(id);
            start.tool_name = Some(name);
            out.push(start);
        } else {
            i = found as usize;
        }

        let call = &mut self.calls[i];
        if call.has_finished {
            self.record(&out);
            return Ok(out);
        }
        if let Some(args) = &delta.arguments {
            if args != "" {
                call.arguments = call.arguments.clone() + args;
                if emit_deltas {
                    let mut d = part("tool-input-delta");
                    d.id = Some(call.id.clone());
                    d.delta = Some(args.clone());
                    out.push(d);
                }
            }
        }
        if serde_json::from_str::<serde_json::Value>(&call.arguments).is_ok() {
            call.has_finished = true;
            let mut c = part("tool-call");
            c.id = Some(call.id.clone());
            c.tool_name = Some(call.name.clone());
            c.input = Some(call.arguments.clone());
            out.push(c);
        }
        self.record(&out);
        Ok(out)
    }

    /// Emits every call that never completed (with whatever arguments it
    /// has, `{}` if none), then a `finish` part. Later calls emit nothing.
    pub fn finish(&mut self, reason: &Option<String>) -> Vec<StreamPart> {
        let mut out = Vec::new();
        if self.finished {
            return out;
        }
        self.finished = true;
        for call in self.calls.iter_mut() {
            if !call.has_finished {
                call.has_finished = true;
                let mut c = part("tool-call");
                c.id = Some(call.id.clone());
                c.tool_name = Some(call.name.clone());
                c.input = Some(if call.arguments == "" {
                    "{}".to_string()
                } else {
                    call.arguments.clone()
                });
                out.push(c);
            }
        }
        let mut f = part("finish");
        f.finish_reason = reason.clone();
        out.push(f);
        self.record(&out);
        out
    }
}
