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
//!   不从录制恢复连接设置。Jev 使用显式 key 或 TYPESAFE_API_KEY。

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::runtime::Runtime;

use aimux_core::language_model_message::{AssistantPart, LanguageModelMessage, UserPart};
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
            if rec.provider.provider_id != "jev" {
                bail!(
                    "decision replay: no configured provider {}",
                    rec.provider.provider_id
                );
            }
            let config = match api_key {
                Some(key) => aimux_providers::JevConfig::new(key),
                None => aimux_providers::JevConfig::from_env()?,
            };
            let providers = std::collections::BTreeMap::from([(
                "jev".into(),
                std::sync::Arc::new(aimux_providers::JevProvider::new(config))
                    as std::sync::Arc<dyn aimux_core::Provider>,
            )]);
            let registry = aimux_core::create_provider_registry(providers, Default::default());
            aimux_providers::rebuild_decision_provider(&rec.provider, &registry)?
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
        let mut providers = aimux_providers::default_providers();
        if let Some(key) = api_key {
            providers.insert(
                rec.provider.provider_id.clone(),
                aimux_providers::create_provider(
                    &rec.provider.provider_id,
                    aimux_providers::PresetSettings {
                        api_key: Some(key.to_string().into()),
                        ..Default::default()
                    },
                )?,
            );
        }
        let registry = aimux_core::create_provider_registry(providers, Default::default());
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
        rec.call_id, rec.provider.provider, rec.provider.model_id, result.finish_reason.unified
    );
    if !result.text.is_empty() {
        println!("  text: {}", truncate(&result.text, 300));
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…(truncated)")
    }
}
