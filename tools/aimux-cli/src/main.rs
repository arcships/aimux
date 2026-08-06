//! aimux — 基于 aimux 构建的调试 CLI(第一版:缓存探测业务 client)。
//!
//! 三层拆分②(RFC-0015 §1.2 / RFC-0025):探测本身在 core,本二进制只做
//! 探测业务的消费逻辑——读 TraceRecord jsonl 审计、会话级诊断、在线探测
//! 指定 provider 的缓存能力。不实现探测算法,不内置告警决策。

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

mod probe;
mod report;

#[derive(Parser)]
#[command(
    name = "aimux",
    version,
    about = "aimux 调试工具(缓存探测 client, RFC-0025)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 缓存探测:审计/诊断/在线探测 provider 缓存能力(RFC-0015)
    CacheProbe(CacheProbeArgs),
}

#[derive(Args)]
struct CacheProbeArgs {
    #[command(subcommand)]
    sub: ProbeCommand,
}

#[derive(Subcommand)]
enum ProbeCommand {
    /// 离线审计:读 TraceRecord jsonl(export_jsonl 输出)→ 统计报告
    Offline(OfflineArgs),
    /// 会话级诊断:读 jsonl → 指定 session 的链级命中演变
    Session(SessionArgs),
    /// 在线探测:直接调 provider 发测试请求,报告其缓存能力(消耗真实费用)
    Provider(ProviderArgs),
}

#[derive(Args)]
pub(crate) struct OfflineArgs {
    /// TraceRecord jsonl 文件(core export_jsonl 输出,每行一个 TraceRecord)
    #[arg(long)]
    file: PathBuf,
    /// 只统计该 provider
    #[arg(long)]
    provider: Option<String>,
    /// 只统计该 session
    #[arg(long)]
    session: Option<String>,
    /// 输出格式:text 给人,json 给脚本
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Args)]
pub(crate) struct SessionArgs {
    /// TraceRecord jsonl 文件
    #[arg(long)]
    file: PathBuf,
    /// 要诊断的 session_id
    #[arg(long)]
    session: String,
    /// 输出格式
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Args)]
pub(crate) struct ProviderArgs {
    /// provider 注册表名(如 openai / deepseek / groq)
    #[arg(long)]
    provider: String,
    /// 模型 id
    #[arg(long)]
    model: String,
    /// API key 或 env:VAR 引用(如 env:OPENAI_API_KEY)
    #[arg(long)]
    api_key: String,
    /// base-url 覆盖(测试/代理用)
    #[arg(long)]
    base_url: Option<String>,
    /// 测试请求数上限(默认 4;每次追加一轮对话内容验证前缀命中)
    #[arg(long, default_value_t = 4)]
    max_requests: usize,
    /// 覆盖默认测试 system 模板(建议 ≥1024 token 才能触发多数 provider 缓存)
    #[arg(long)]
    prompt: Option<String>,
    /// 输出格式
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(ValueEnum, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Text,
    Json,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::CacheProbe(args) => match args.sub {
            ProbeCommand::Offline(a) => probe::offline::run(&a),
            ProbeCommand::Session(a) => probe::session::run(&a),
            ProbeCommand::Provider(a) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(probe::provider::run(&a))?;
                Ok(())
            }
        },
    }
}
