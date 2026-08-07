//! aimux-replay — RFC-0023 请求回放 CLI(层 2 消费端)。
//!
//! 读录制 jsonl(每行一个 `Recording`),按 `ProviderRecord` 自动重建
//! provider(OpenAI 兼容族),再用录制输入经 `replay_with_model` **重发真实
//! API**。用途:离线重跑线上流量、改 prompt 重发(A/B)、回归对比、CI 集成。
//!
//! 安全:
//! - `--dry-run` 只打印重建后的 provider/prompt/目标 URL,**不发请求、
//!   不输出任何凭据**(api_key 来源按 `api_key_source` 打印)。
//! - 重发会消耗真实 token/费用,文档(§3.6.1)已有警示。
//! - 原生协议 provider(anthropic/google/...)`rebuild_provider` 明确
//!   `Unsupported`——CLI 打印错误并跳过,需自行传 model 实例走库 API。

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::runtime::Runtime;

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
    /// 显式 api key。api_key_source 为 explicit/unknown 时需要。
    #[arg(long)]
    api_key: Option<String>,
    /// 覆盖 prompt(替换所有录制的 prompt;A/B 重发)。
    #[arg(long)]
    prompt: Option<String>,
    /// dry-run:打印重建后的 provider/prompt/目标 URL,不发请求、不输出凭据。
    #[arg(long)]
    dry_run: bool,
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
            run_replay(rec, cli.api_key.as_deref(), cli.prompt.as_deref())
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
        "  provider: {} / model: {}",
        rec.provider.provider, rec.provider.model_id
    );
    println!(
        "  base_url: {}",
        rec.provider.base_url.as_deref().unwrap_or("(default)")
    );
    println!("  api_key_source: {}", rec.provider.api_key_source);
    println!("  prompt: {} message(s)", rec.input.prompt.len());
    if let Some(text) = rec.input.prompt.first().and_then(prompt_text) {
        println!("  first message: {text:?}");
    }
    println!("  (dry-run — no request sent)");
    Ok(())
}

/// 取消息首个文本内容(截断展示用)。
fn prompt_text(
    m: &aimux_core::language_model_message::LanguageModelPromptMessage,
) -> Option<String> {
    match m.content.first() {
        Some(aimux_core::content::ContentPart::Text { text, .. }) => Some(text.clone()),
        _ => None,
    }
}

/// 真实回放:重建 provider → replay_with_model → 打印结果。
fn run_replay(rec: &Recording, api_key: Option<&str>, prompt: Option<&str>) -> Result<()> {
    let model = rebuild_provider(&rec.provider, api_key)
        .with_context(|| format!("rebuild provider for '{}'", rec.provider.provider))?;

    let overrides = prompt.map(|text| ReplayOverrides {
        prompt: Some(vec![
            aimux_core::language_model_message::LanguageModelPromptMessage {
                role: aimux_core::message::Role::User,
                content: vec![aimux_core::content::ContentPart::text(text)],
                provider_options: None,
            },
        ]),
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
