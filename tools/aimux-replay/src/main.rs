//! aimux-replay — RFC-0023 请求回放 CLI(层 2 消费端)。
//!
//! 读录制 jsonl(每行一个 `Recording`,仅 schema 3),按 `ProviderRecord` 的
//! `provider_id` + `model_id` 重建 model,再用录制输入经 `replay_with_model`
//! **重发真实 API**。用途:离线重跑线上流量、改 prompt 重发(A/B)、回归对比、CI 集成。
//!
//! 安全:
//! - `--dry-run` 只打印录制的 provider/model/prompt,**不发请求、
//!   不输出任何凭据**。
//! - 重发会消耗真实 token/费用,文档(§3.6.1)已有警示。
//! - `--mock` 离线回放语言或 decision 录制；真实回放只使用调用方配置，
//!   不从录制恢复连接设置。语言与 decision 录制都从同一个 registry 重建
//!   ([`replay_registry`]:全部内置 provider,`--api-key` 替换录制的那个);
//!   Jev 不在内置集合里,录制指向它时加入,使用显式 key 或 TYPESAFE_API_KEY。

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::runtime::Runtime;

use aimux_core::language_model_message::{AssistantPart, LanguageModelMessage, UserPart};
use aimux_core::provider_registry::ProviderRegistry;
use aimux_core::recording::Recording;
use aimux_core::replay::{ReplayOverrides, replay_with_model};
use aimux_providers::rebuild_provider;

#[derive(Parser)]
#[command(
    name = "aimux-replay",
    version,
    about = "RFC-0023 请求回放:读录制 jsonl,重建 provider 并重发真实 API"
)]
struct Cli {
    /// 录制 jsonl 文件(每行一个 Recording)。
    file: PathBuf,
    /// 仅回放该 call_id(默认全部)。
    #[arg(long)]
    call_id: Option<String>,
    /// 显式 api key;缺省时读 registry 条目的环境变量。
    #[arg(long)]
    api_key: Option<String>,
    /// 覆盖 prompt(替换所有录制的 prompt;A/B 重发)。
    #[arg(long)]
    prompt: Option<String>,
    /// dry-run:打印录制的 provider/model/prompt,不发请求、不输出凭据。
    #[arg(long)]
    dry_run: bool,
    /// Replay recorded responses offline.
    #[arg(long)]
    mock: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let recordings = load_recordings(&cli.file)?;

    let mut failures = 0usize;
    for rec in &recordings {
        if let Some(want) = &cli.call_id
            && &rec.call_id != want
        {
            continue;
        }
        let result = if cli.dry_run {
            run_dry(rec)
        } else {
            run_replay(rec, cli.api_key.as_deref(), cli.prompt.as_deref(), cli.mock)
        };
        match result {
            Ok(()) => {}
            Err(e) => {
                failures += 1;
                eprintln!("[{}] error: {e:#}", rec.call_id);
            }
        }
    }

    if recordings.is_empty() {
        bail!("no recordings in '{}'", cli.file.display());
    }
    if failures > 0 {
        bail!("{failures}/{} recordings failed", recordings.len());
    }
    Ok(())
}

/// 加载 jsonl(空行跳过);任意一行解析失败 → 报错带行号。
fn load_recordings(path: &PathBuf) -> Result<Vec<Recording>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read recording file '{}'", path.display()))?;
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: Recording = serde_json::from_str(line)
            .with_context(|| format!("parse recording line {}", idx + 1))?;
        out.push(rec);
    }
    Ok(out)
}

/// dry-run:打印将发什么(不含凭据),不重建不请求。
fn run_dry(rec: &Recording) -> Result<()> {
    println!("call_id: {}", rec.call_id);
    println!(
        "  provider_id: {} / provider: {} / model: {}",
        rec.provider.provider_id, rec.provider.provider, rec.provider.model_id
    );
    if rec.input.operation == aimux_core::recording::RecordingOperation::Decision {
        println!(
            "  decision: {} question(s)",
            rec.input.options["questions"]
                .as_array()
                .map_or(0, Vec::len)
        );
    }
    println!("  prompt: {} message(s)", rec.input.prompt.len());
    if let Some(text) = rec.input.prompt.first().and_then(prompt_text) {
        println!("  first message: {text:?}");
    }
    println!("  (dry-run — no request sent)");
    Ok(())
}

/// 取消息首个文本内容(截断展示用)。
fn prompt_text(m: &LanguageModelMessage) -> Option<String> {
    match m {
        LanguageModelMessage::System { content, .. } => Some(content.clone()),
        LanguageModelMessage::User { content, .. } => match content.first() {
            Some(UserPart::Text(part)) => Some(part.text.clone()),
            _ => None,
        },
        LanguageModelMessage::Assistant { content, .. } => match content.first() {
            Some(AssistantPart::Text(part)) => Some(part.text.clone()),
            _ => None,
        },
        LanguageModelMessage::Tool { .. } => None,
    }
}

/// 真实回放:重建 provider → replay_with_model → 打印结果。
fn run_replay(
    rec: &Recording,
    api_key: Option<&str>,
    prompt: Option<&str>,
    mock: bool,
) -> Result<()> {
    if rec.input.operation == aimux_core::recording::RecordingOperation::Decision {
        if prompt.is_some() {
            bail!(
                "--prompt applies to language recordings; decision replay preserves state/questions"
            );
        }
        let model: Box<dyn aimux_core::DecisionModel> = if mock {
            Box::new(aimux_core::replay::MockDecisionReplayModel::new(
                &rec.provider.provider,
                &rec.provider.model_id,
                vec![rec.clone()],
            )?)
        } else {
            let registry = replay_registry(&rec.provider.provider_id, api_key)?;
            aimux_providers::rebuild_decision_provider(&rec.provider, &registry).with_context(
                || {
                    format!(
                        "rebuild decision provider for '{}'",
                        rec.provider.provider_id
                    )
                },
            )?
        };
        let result = Runtime::new()?.block_on(aimux_core::replay::replay_decision_with_model(
            rec,
            model.as_ref(),
        ))?;
        println!(
            "[{}] {} / {} → {}",
            rec.call_id,
            result.provider,
            result.model,
            serde_json::to_string(&result.answers)?
        );
        return Ok(());
    }
    let model: std::sync::Arc<dyn aimux_core::LanguageModel> = if mock {
        std::sync::Arc::new(aimux_core::replay::MockReplayModel::new(
            &rec.provider.provider,
            &rec.provider.model_id,
            vec![rec.clone()],
        ))
    } else {
        let registry = replay_registry(&rec.provider.provider_id, api_key)?;
        rebuild_provider(&rec.provider, &registry)
            .with_context(|| format!("rebuild provider for '{}'", rec.provider.provider_id))?
    };

    let overrides = prompt.map(|text| ReplayOverrides {
        prompt: Some(vec![LanguageModelMessage::user_text(text)]),
        ..Default::default()
    });

    let runtime = Runtime::new().context("create tokio runtime")?;
    let result = runtime
        .block_on(async { replay_with_model(rec, model.as_ref(), overrides.as_ref()).await })?;

    println!(
        "[{}] {} / {} → finish={:?}",
        rec.call_id,
        rec.provider.provider,
        rec.provider.model_id,
        result.finish_reason()
    );
    let text = result.text();
    if !text.is_empty() {
        println!("  text: {}", truncate(&text, 300));
    }
    Ok(())
}

/// The providers live replay rebuilds a recorded model from, for language and
/// decision recordings alike: every built-in provider with default settings
/// (`default_providers()`), the recorded one created again with `api_key`
/// when given. Jev, the decision provider outside the built-in set, is added
/// when the recording names it, with `api_key` or `TYPESAFE_API_KEY`.
fn replay_registry(provider_id: &str, api_key: Option<&str>) -> Result<ProviderRegistry> {
    let mut providers = aimux_providers::default_providers();
    if provider_id == "jev" {
        let config = match api_key {
            Some(key) => aimux_providers::JevConfig::new(key),
            None => aimux_providers::JevConfig::from_env()?,
        };
        providers.insert(
            provider_id.to_string(),
            std::sync::Arc::new(aimux_providers::JevProvider::new(config)),
        );
    } else if let Some(key) = api_key {
        providers.insert(
            provider_id.to_string(),
            aimux_providers::create_provider(
                provider_id,
                aimux_providers::PresetSettings {
                    api_key: Some(key.to_string().into()),
                    ..Default::default()
                },
            )?,
        );
    }
    Ok(aimux_core::create_provider_registry(
        providers,
        Default::default(),
    ))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…(truncated)")
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use aimux_core::Provider;
    use aimux_core::decision_model::{DecisionCallOptions, decide};
    use aimux_core::recording::{self, RecordingOperation, RingRecorder};
    use aimux_providers::{OpenAIProviderSettings, create_openai};
    use serde_json::Value;

    use super::*;

    /// The OpenAI Decisions contract fixture of the provider tests.
    const OPENAI_DECISIONS: &str =
        include_str!("../../../aimux-providers/tests/fixtures/openai_decisions.json");

    /// An HTTP server on a thread that answers every request with `body` and
    /// keeps each raw request.
    fn serve(body: String) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        std::thread::spawn(move || {
            for mut socket in listener.incoming().flatten() {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = socket.read(&mut buf).unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    let complete = text.split_once("\r\n\r\n").is_some_and(|(head, rest)| {
                        let length = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        rest.len() >= length
                    });
                    if n == 0 || complete {
                        break;
                    }
                }
                sink.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&raw).to_string());
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes());
            }
        });
        (url, seen)
    }

    /// Sets an environment variable for the test and restores it on drop.
    struct EnvVar(&'static str, Option<std::ffi::OsString>);

    impl EnvVar {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(name);
            // SAFETY: the only test in this crate that touches the variable.
            unsafe { std::env::set_var(name, value) };
            Self(name, previous)
        }
    }

    impl Drop for EnvVar {
        fn drop(&mut self) {
            // SAFETY: as in `set`.
            unsafe {
                match &self.1 {
                    Some(value) => std::env::set_var(self.0, value),
                    None => std::env::remove_var(self.0),
                }
            }
        }
    }

    /// B17: a decision recording of a provider other than Jev replays live
    /// through the registry the language path uses. The recorded OpenAI
    /// decision is rebuilt from `default_providers()` with `--api-key` and
    /// sent again (`OPENAI_BASE_URL` points the default provider at the
    /// local server).
    #[test]
    fn an_openai_decision_recording_replays_live() {
        let fixture: Value = serde_json::from_str(OPENAI_DECISIONS).unwrap();
        let (base_url, seen) = serve(fixture["response"].to_string());

        let recorder = Arc::new(RingRecorder::new());
        recording::init_recording(Some(recorder.clone()));
        let model = create_openai(OpenAIProviderSettings {
            api_key: Some("record-key".into()),
            base_url: Some(base_url.clone()),
            ..Default::default()
        })
        .unwrap()
        .decision_model("gpt-6-luna")
        .unwrap();
        let options: DecisionCallOptions =
            serde_json::from_value(fixture["options"].clone()).unwrap();
        Runtime::new()
            .unwrap()
            .block_on(decide(model.as_ref(), options))
            .unwrap();
        recording::init_recording(None);
        let rec = recorder.completed().pop().expect("one recording");
        assert_eq!(rec.input.operation, RecordingOperation::Decision);
        assert_eq!(rec.provider.provider_id, "openai");

        let _base_url = EnvVar::set("OPENAI_BASE_URL", &base_url);
        run_replay(&rec, Some("replay-key"), None, false).unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "the recorded call and its replay");
        assert!(seen[1].starts_with("POST /v1/decisions"), "{}", seen[1]);
        assert!(
            seen[1].to_ascii_lowercase().contains("bearer replay-key"),
            "the replay uses the caller's key"
        );
    }

    /// Every decision provider resolves in the replay registry: a built-in
    /// one by its name, Jev when the recording names it; an unknown name is
    /// reported.
    #[test]
    fn the_replay_registry_holds_every_decision_provider() {
        let registry = replay_registry("ollama", None).unwrap();
        assert!(
            registry
                .provider("ollama")
                .unwrap()
                .decision_model("m")
                .is_ok()
        );
        let registry = replay_registry("jev", Some("k")).unwrap();
        assert!(
            registry
                .provider("jev")
                .unwrap()
                .decision_model("m")
                .is_ok()
        );
        let record = aimux_core::recording::ProviderRecord::new("no-such", "no-such", "m");
        let registry = replay_registry("no-such", None).unwrap();
        assert!(aimux_providers::rebuild_decision_provider(&record, &registry).is_err());
    }
}
