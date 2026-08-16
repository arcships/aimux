//! Built-in tools for the console agent (RFC-0029 §6.3).
//!
//! Tools run server-side and never depend on the LLM. `http_get` is
//! whitelist-gated and **disabled by default** (SSRF protection, §9).

use serde_json::{Value, json};

use crate::wire::WireTool;

/// JSON Schema for the built-in tools — what the model sees.
pub fn tool_schemas() -> Vec<WireTool> {
    vec![
        WireTool {
            name: "calculator".into(),
            description: Some(
                "Evaluate a plain arithmetic expression, e.g. \"1 + 2 * 3\". \
                 Supports + - * / parentheses and decimal numbers."
                    .into(),
            ),
            parameters: json!({
                "type": "object",
                "properties": { "expr": { "type": "string", "description": "expression" } },
                "required": ["expr"]
            }),
        },
        WireTool {
            name: "datetime".into(),
            description: Some("Current date and time (UTC, ISO 8601).".into()),
            parameters: json!({ "type": "object" }),
        },
        WireTool {
            name: "echo".into(),
            description: Some("Return the input text unchanged (useful for loop tests).".into()),
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
        },
    ]
}

/// Execute a built-in tool. Returns the tool result (always a JSON value).
pub fn execute(name: &str, input: &Value) -> Result<Value, String> {
    match name {
        "calculator" => {
            let expr = input
                .get("expr")
                .and_then(Value::as_str)
                .ok_or_else(|| "calculator: missing string field `expr`".to_string())?;
            let value = eval(expr)?;
            Ok(json!({ "value": value, "expr": expr }))
        }
        "datetime" => Ok(json!({
            "unix_ms": now_unix_ms(),
            "iso_utc": now_iso_utc(),
        })),
        "echo" => {
            let text = input.get("text").cloned().unwrap_or(Value::Null);
            Ok(json!({ "text": text }))
        }
        _ => Err(format!("unknown tool '{name}'")),
    }
}

// ── calculator: small recursive-descent arithmetic evaluator ────────────────

fn eval(expr: &str) -> Result<f64, String> {
    let mut p = Parser {
        chars: expr.chars().collect(),
        pos: 0,
    };
    let v = p.parse_expr()?;
    p.skip_ws();
    if p.pos != p.chars.len() {
        return Err(format!("unexpected trailing input: {:?}", p.rest()));
    }
    Ok(v)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn rest(&self) -> String {
        self.chars[self.pos..].iter().collect()
    }

    fn parse_expr(&mut self) -> Result<f64, String> {
        let mut v = self.parse_term()?;
        loop {
            self.skip_ws();
            if self.pos < self.chars.len() && matches!(self.chars[self.pos], '+' | '-') {
                let op = self.chars[self.pos];
                self.pos += 1;
                let r = self.parse_term()?;
                v = if op == '+' { v + r } else { v - r };
            } else {
                break;
            }
        }
        Ok(v)
    }

    fn parse_term(&mut self) -> Result<f64, String> {
        let mut v = self.parse_factor()?;
        loop {
            self.skip_ws();
            if self.pos < self.chars.len() && matches!(self.chars[self.pos], '*' | '/') {
                let op = self.chars[self.pos];
                self.pos += 1;
                let r = self.parse_factor()?;
                v = if op == '*' {
                    v * r
                } else {
                    if r == 0.0 {
                        return Err("division by zero".into());
                    }
                    v / r
                };
            } else {
                break;
            }
        }
        Ok(v)
    }

    fn parse_factor(&mut self) -> Result<f64, String> {
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return Err("unexpected end of expression".into());
        }
        match self.chars[self.pos] {
            '(' => {
                self.pos += 1;
                let v = self.parse_expr()?;
                self.skip_ws();
                if self.pos >= self.chars.len() || self.chars[self.pos] != ')' {
                    return Err("missing ')'".into());
                }
                self.pos += 1;
                Ok(v)
            }
            '-' => {
                self.pos += 1;
                Ok(-self.parse_factor()?)
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = self.pos;
                while self.pos < self.chars.len()
                    && (self.chars[self.pos].is_ascii_digit() || self.chars[self.pos] == '.')
                {
                    self.pos += 1;
                }
                let s: String = self.chars[start..self.pos].iter().collect();
                s.parse::<f64>()
                    .map_err(|_| format!("invalid number '{s}'"))
            }
            c => Err(format!("unexpected character '{c}'")),
        }
    }
}

// ── time helpers (no chrono dependency) ─────────────────────────────────────

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_iso_utc() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs() as i64;
    let millis = d.subsec_millis();
    let (y, mo, day) = civil_from_days(secs.div_euclid(86_400));
    let hms = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{mo:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        hms / 3600,
        (hms % 3600) / 60,
        hms % 60
    )
}

/// Days since 1970-01-01 → civil (year, month, day). `z` must be ≥ 0.
/// Howard Hinnant's `civil_from_days` algorithm (same as `aimux-core::session`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_basic() {
        assert_eq!(eval("1+1").unwrap(), 2.0);
        assert_eq!(eval("1 + 2 * 3").unwrap(), 7.0);
        assert_eq!(eval("(1 + 2) * 3").unwrap(), 9.0);
        assert_eq!(eval("10 / 4").unwrap(), 2.5);
        assert_eq!(eval("-3 + 5").unwrap(), 2.0);
    }

    #[test]
    fn calculator_errors() {
        assert!(eval("1/0").is_err());
        assert!(eval("1 +").is_err());
        assert!(eval("(1+2").is_err());
        assert!(eval("1+foo").is_err());
    }

    #[test]
    fn execute_echo_and_unknown() {
        let out = execute("echo", &json!({"text": "hello"})).unwrap();
        assert_eq!(out["text"], "hello");
        assert!(execute("nope", &json!({})).is_err());
    }
}
