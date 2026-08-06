//! RFC-0023: 调用上下文录制(P1 — 数据模型 + Recorder trait + 门控 + JsonlRecorder)。
//!
//! 录制一次 `generate_text`/`stream_text` 调用的三层完整上下文:
//! ① 输入侧(prompt + options)、② 配置侧(provider 身份)、③ HTTP 侧(wire 交换)。
//!
//! 关键性质(按 2026-08-06 定稿):
//! - **call_id 关联**:一次逻辑调用一个 `call_id`(与 RFC-0015/24/25 语义一致,
//!   区别于 HTTP 请求级 ID 与跨服务 trace)。
//! - **默认关闭**:不调 `init_recording`,热路径 = 1 读锁 + clone(次 ns 级)。
//! - **隐私受控**:api_key / Authorization 恒脱敏(contains 式,含 `x-goog-api-key`);
//!   `InputRecord.options` 序列化前递归脱敏。
//! - **completion barrier**:outcome 与全部 exchange(流式含终结)齐才写行。
//! - **专用 writer thread + oneshot flush**:同步 `flush()` 阻塞至落盘,不依赖运行时。
//! - **recorder 快照绑定**:层 A 入口取一次 `Arc<dyn Recorder>` 随调用,禁止各点重读。

use crate::language_model_message::LanguageModelPrompt;
use crate::options::CallOptions;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ── 数据模型(三层 + call_id 关联;schema 版本)────────────────────────────

/// 录制格式版本(用于未来字段迁移与绑定层兼容)。
pub const RECORDING_SCHEMA: u32 = 1;

/// 一次完整调用的录制记录(三层 + call_id 关联)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// 格式版本。
    pub schema: u32,
    /// 暴露逻辑调用 ID,关联三层。
    pub call_id: String,
    /// ISO 8601 时间戳。
    pub recorded_at: String,

    /// ① 输入侧:调用参数(prompt + options)。
    pub input: InputRecord,

    /// ② 配置侧:provider 身份与配置。
    pub provider: ProviderRecord,

    /// ③ HTTP 侧:实际 wire 交换(per-attempt 一条)。
    pub exchanges: Vec<HttpExchange>,

    /// 最终结果摘要(状态 + finish_reason + usage)。
    pub outcome: OutcomeRecord,

    /// wire 是否完整(流式 body 未补全时为 false;由 writer 标记)。
    #[serde(default)]
    pub complete: bool,
    /// 传输层封闭信号:层 B 声明"不再有 exchange"(P1:层 A 收尾自动发;
    /// P2:由层 B 发送)。false = 仍可能来 exchange(记录暂不可定稿)。
    #[serde(default)]
    pub transport_closed: bool,
}

impl Recording {
    /// 以 call_id 开一条新录制(字段由事件流填充)。
    pub fn new(call_id: &str, input: InputRecord, provider: ProviderRecord) -> Self {
        Self {
            schema: RECORDING_SCHEMA,
            call_id: call_id.to_string(),
            recorded_at: iso8601_now(),
            input,
            provider,
            exchanges: Vec::new(),
            outcome: OutcomeRecord::default(),
            complete: false,
            transport_closed: false,
        }
    }

    /// completion barrier:outcome 非 Pending && 传输层已封闭 &&
    /// 全部 exchange 已终结。缺封闭信号时(层 B 未发)不提前写出,
    /// 防止 outcome 先到时误以为"无 exchange"而早写。
    fn ready(&self) -> bool {
        self.transport_closed
            && self.outcome.status != OutcomeStatus::Pending
            && self.exchanges.iter().all(|e| e.finalized)
    }
}

/// ① 输入侧:完整调用参数,足以重建 generate_text 调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputRecord {
    /// 完整 prompt(消息数组,含 ContentPart::Image 等多模态)。
    pub prompt: LanguageModelPrompt,
    /// 序列化的 CallOptions(abort_signal/call_id 已 serde skip);
    /// headers/provider_options/body_overrides 已递归脱敏。
    pub options: serde_json::Value,
}

impl InputRecord {
    /// 从 CallOptions 提取输入侧快照(options 序列化后递归脱敏)。
    pub fn from_call_options(options: &CallOptions) -> Self {
        let value = serde_json::to_value(options).unwrap_or(serde_json::Value::Null);
        Self {
            prompt: options.prompt.clone(),
            options: redact_json(value),
        }
    }
}

/// ② 配置侧:provider 身份(请求回放重建用)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    /// `model.provider()`,如 "openai"。
    pub provider: String,
    /// `model.model_id()`,如 "gpt-4o"。
    pub model_id: String,
    /// provider 的 base_url。
    pub base_url: Option<String>,
    /// api_key 来源(不存明文):"env:OPENAI_API_KEY" / "explicit" / "none" / "unknown"。
    pub api_key_source: String,
    /// OpenAICompatProfile(能力差异)。
    pub profile: Option<serde_json::Value>,
    /// ProviderOptions(headers/org/project/...;已脱敏)。
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderRecord {
    /// 最小快照(provider/model_id)——`config_snapshot` 默认实现;cover 的部分放结构方法。
    pub fn minimal(provider: &str, model_id: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            base_url: None,
            api_key_source: "unknown".to_string(),
            profile: None,
            provider_options: None,
        }
    }
}

/// ③ HTTP 侧:单次 attempt 的 wire 交换。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchange {
    /// 第几次重试(0=首次);per-attempt 递增。
    pub attempt: u32,
    pub request: HttpRecord,
    /// None = 请求失败未获响应。
    pub response: Option<ResponseRecord>,
    pub timing: TimingRecord,
    pub error: Option<String>,
    /// 流式:该 exchange 是否已终结(收到 response 补全)。非流式恒 true。
    #[serde(default = "default_finalized")]
    pub finalized: bool,
}

fn default_finalized() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRecord {
    pub method: String,
    pub url: String,
    /// 敏感头(authorization/cookie/含 api-key 等)已脱敏为 "[REDACTED]"。
    pub headers: Vec<(String, String)>,
    /// 明文(脱敏后);None = 无 body。
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseRecord {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// 非流式:完整 JSON;流式:原始 SSE 拼接文本(上限截断)。
    pub body: Option<String>,
    pub stream_chunks: Option<usize>,
    /// 首字节延迟(流式)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttfb_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingRecord {
    pub latency_ms: u64,
    pub ttfb_ms: Option<u64>,
}

/// 调用终结状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    /// 未到(内部哨兵,不作为最终输出)。
    #[default]
    Pending,
    Success,
    /// 调用失败(非流式 err / 流式 Error part / item Err)。
    Error,
    /// 流式:EOF 前未见 Finish(协议不完整)。
    Incomplete,
    /// 流式:消费方提前 drop。
    Cancelled,
}

/// 最终结果摘要。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutcomeRecord {
    pub status: OutcomeStatus,
    pub finish_reason: Option<String>,
    pub error: Option<String>,
    /// 序列化的 Usage。
    pub usage: Option<serde_json::Value>,
}

impl OutcomeRecord {
    /// 非流式成功。
    pub fn from_generate_result(r: &crate::result::GenerateResult) -> Self {
        Self {
            status: OutcomeStatus::Success,
            finish_reason: serde_json::to_value(r.finish_reason.unified)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string())),
            error: None,
            usage: serde_json::to_value(&r.usage).ok(),
        }
    }

    /// 失败(非流式 / 流式立即失败)。
    pub fn from_error(e: &crate::error::AiMuxError) -> Self {
        Self {
            status: OutcomeStatus::Error,
            finish_reason: None,
            error: Some(e.to_string()),
            usage: None,
        }
    }
}

// ── Recorder trait(快照:一次调用绑定一个实例)──────────────────────────

/// 录制器 trait。
pub trait Recorder: Send + Sync {
    /// 录制输入侧 + 配置侧最小信息(层 A 入口调用)。
    fn record_input(&self, call_id: &str, options: &CallOptions, provider: &str, model_id: &str);
    /// 录制配置侧完整快照。
    fn record_provider(&self, call_id: &str, snapshot: &ProviderRecord);
    /// 录制单次 HTTP 交换(层 B http.rs 调用,per-attempt 一条)。
    fn record_exchange(&self, call_id: &str, exchange: &HttpExchange);
    /// 流式响应终结时归档该次 exchange(补 body/status/ttfb + 可选 error)。
    /// `error` 非空时合并到该 exchange 而非新增一条(保持 attempt 唯一)。
    fn record_exchange_update(
        &self,
        call_id: &str,
        attempt: u32,
        response: &ResponseRecord,
        error: Option<String>,
    );
    /// 录制最终结果(层 A 入口调用)。
    fn record_outcome(&self, call_id: &str, outcome: &OutcomeRecord);
    /// 声明传输层封闭:该 call 不再会有 exchange(层 B;P1 层 A 收尾调用)。
    fn record_transport_closed(&self, call_id: &str) {
        let _ = call_id;
    }
    /// 同步 flush:阻塞到全部已写行落盘(oneshot 回执),关闭前调用确保不丢。
    fn flush(&self);
}

// ── 全局门控(关闭时热路径 ≈1 读锁;支持替换/关闭/测试隔离)────────────

static RECORDER: RwLock<Option<Arc<dyn Recorder>>> = RwLock::new(None);

/// 一次调用的录制上下文(RFC R7 快照绑定):层 A 入口构造一次,
/// 随 `CallOptions`/`HttpRequest` 传到底层,禁止各录制点重读全局 recorder。
#[derive(Clone)]
pub struct RecordingContext {
    pub call_id: String,
    pub recorder: Arc<dyn Recorder>,
}

impl std::fmt::Debug for RecordingContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingContext")
            .field("call_id", &self.call_id)
            .field("recorder", &"..")
            .finish()
    }
}

/// 注册/替换/关闭全局录制器。传 None 关闭。
pub fn init_recording(recorder: Option<Arc<dyn Recorder>>) {
    if let Ok(mut g) = RECORDER.write() {
        *g = recorder;
    }
}

/// 从环境变量初始化:`AIMUX_RECORD=1` 开启,`AIMUX_RECORD_DIR` 指定目录(默认 `./recordings`)。
/// 未开启返回 false(静默)。
pub fn init_recording_from_env() -> bool {
    if std::env::var("AIMUX_RECORD").as_deref() != Ok("1") {
        return false;
    }
    let dir = std::env::var("AIMUX_RECORD_DIR").unwrap_or_else(|_| "./recordings".to_string());
    init_recording(Some(Arc::new(JsonlRecorder::new(dir))));
    true
}

/// 从当前全局 recorder 生成一次调用的 context(关闭时 None)。
/// 层 A 入口:先读一次,再用 `context && ctx.start()` 记录 ①+②。
pub fn context(call_id: impl Into<String>) -> Option<RecordingContext> {
    recorder().map(|recorder| RecordingContext {
        call_id: call_id.into(),
        recorder,
    })
}

/// 热路径检查(关闭时 None,≈1 读锁 + 1 次 Arc clone)。
///
/// **快照语义**:层 A 入口调用一次并随 `call_id` 传到底层;
/// 禁止各录制点重读全局 recorder(中途替换会拆散同一条 Recording)。
pub fn recorder() -> Option<Arc<dyn Recorder>> {
    RECORDER.read().ok()?.clone()
}

// ── call_id 生成 ──────────────────────────────────────────────────────────

static CALL_SEQ: AtomicU64 = AtomicU64::new(0);

/// 生成进程级唯一 call_id(`call-{ns}-{seq}`;与 session.rs 同构)。
pub fn new_call_id() -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("call-{ns}-{}", CALL_SEQ.fetch_add(1, Ordering::Relaxed))
}

// ── 脱敏(contains 式;与 logging.rs 同规则)──────────────────────────────

/// 敏感键判断:受保护头/参数名(值将恒脱敏)。
pub fn is_sensitive_key(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "authorization"
        || n == "proxy-authorization"
        || n == "cookie"
        || n == "set-cookie"
        || n.contains("api-key")
        || n.contains("api_key")
}

/// 递归脱敏(JSON 中含敏感键的项值替换为 `[REDACTED]`)。
/// 覆盖 `CallOptions.headers`/`provider_options`/`body_overrides` 任意层级。
pub(crate) fn redact_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if is_sensitive_key(&k) {
                    out.insert(k, serde_json::Value::String("[REDACTED]".into()));
                } else {
                    out.insert(k, redact_json(v));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(redact_json).collect())
        }
        other => other,
    }
}

// ── JsonlRecorder(专用 writer thread;completion barrier;oneshot flush)──

/// writer 事件(单消费者线程合并).
enum RecordEvent {
    Input {
        call_id: String,
        input: InputRecord,
        provider: ProviderRecord,
    },
    Provider {
        call_id: String,
        provider: ProviderRecord,
    },
    Exchange {
        call_id: String,
        exchange: HttpExchange,
    },
    ExchangeUpdate {
        call_id: String,
        attempt: u32,
        response: ResponseRecord,
        error: Option<String>,
    },
    Outcome {
        call_id: String,
        outcome: OutcomeRecord,
    },
    /// 传输层封闭:该调用不再会有 exchange(层 B 发;P1 由层 A 收尾发)。
    TransportClosed { call_id: String },
    /// 刷盘命令;完成后经 ack 回执(SyncSender,容量 0 即 rendezvous)。
    Flush { ack: SyncSender<()> },
}

/// 每条完整 `Recording` 一行 jsonl;分片按 call_id 在专用线程合并。
/// 热路径仅 mpsc send(非阻塞),I/O 全部在专用线程。
pub struct JsonlRecorder {
    tx: Option<Sender<RecordEvent>>,
    dir: PathBuf,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl JsonlRecorder {
    /// 在 `dir` 下创建录制器并启动 writer 线程。目录不存在自动创建。
    /// 文件:`{dir}/recordings.jsonl`。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let (tx, rx) = std::sync::mpsc::channel::<RecordEvent>();
        std::fs::create_dir_all(&dir).ok();
        let writer_dir = dir.clone();
        let handle = std::thread::Builder::new()
            .name("aimux-recording".into())
            .spawn(move || writer_loop(rx, writer_dir))
            .expect("failed to spawn recording writer thread");
        Self {
            tx: Some(tx),
            dir,
            thread: Some(handle),
        }
    }

    /// 事件入队(线程退出后静默丢弃)。
    fn send_ev(&self, ev: RecordEvent) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(ev);
        }
    }

    /// 录制文件路径。
    pub fn path(&self) -> PathBuf {
        self.dir.join("recordings.jsonl")
    }
}

impl Drop for JsonlRecorder {
    fn drop(&mut self) {
        // 先主动断开发送端:writer 消费完残留事件后由 Disconnected 触发
        // 兜底写 incomplete 并退出;随后 join 保证落盘完成。
        if let Some(tx) = self.tx.take() {
            drop(tx);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Recorder for JsonlRecorder {
    fn record_input(&self, call_id: &str, options: &CallOptions, provider: &str, model_id: &str) {
        self.send_ev(RecordEvent::Input {
            call_id: call_id.to_string(),
            input: InputRecord::from_call_options(options),
            provider: ProviderRecord::minimal(provider, model_id),
        });
    }

    fn record_provider(&self, call_id: &str, snapshot: &ProviderRecord) {
        // 核心边界统一强制脱敏 provider_options/profile(B6 评审),
        // 不依赖 provider 端自觉。
        let mut snap = snapshot.clone();
        snap.provider_options = snap.provider_options.take().map(redact_json);
        snap.profile = snap.profile.take().map(redact_json);
        self.send_ev(RecordEvent::Provider {
            call_id: call_id.to_string(),
            provider: snap,
        });
    }

    fn record_exchange(&self, call_id: &str, exchange: &HttpExchange) {
        self.send_ev(RecordEvent::Exchange {
            call_id: call_id.to_string(),
            exchange: exchange.clone(),
        });
    }

    fn record_exchange_update(
        &self,
        call_id: &str,
        attempt: u32,
        response: &ResponseRecord,
        error: Option<String>,
    ) {
        self.send_ev(RecordEvent::ExchangeUpdate {
            call_id: call_id.to_string(),
            attempt,
            response: response.clone(),
            error,
        });
    }

    fn record_outcome(&self, call_id: &str, outcome: &OutcomeRecord) {
        self.send_ev(RecordEvent::Outcome {
            call_id: call_id.to_string(),
            outcome: outcome.clone(),
        });
    }

    fn record_transport_closed(&self, call_id: &str) {
        self.send_ev(RecordEvent::TransportClosed {
            call_id: call_id.to_string(),
        });
    }

    fn flush(&self) {
        let (ack_tx, ack_rx) = sync_channel::<()>(0);
        // 写入 flush 事件;若 writer 已退出(通道断)则放弃等待。
        if self.tx.is_none() {
            return;
        }
        self.send_ev(RecordEvent::Flush { ack: ack_tx });
        // 阻塞等 writer 回执(专用写线程,不占调用方热路径锁)。
        let _ = ack_rx.recv_timeout(Duration::from_secs(30));
    }
}

/// 兜底建条目:事件先于 Input 到达(纯层 B/乱序)时,以空 input 占位。
fn entry_or_init<'a>(
    pending: &'a mut HashMap<String, Recording>,
    call_id: &str,
) -> &'a mut Recording {
    pending.entry(call_id.to_string()).or_insert_with(|| {
        Recording::new(
            call_id,
            InputRecord {
                prompt: Vec::new(),
                options: serde_json::Value::Null,
            },
            ProviderRecord::minimal("", ""),
        )
    })
}

/// writer 线程:按 call_id 合并分片;completion barrier 后写一行。
fn writer_loop(rx: Receiver<RecordEvent>, dir: PathBuf) {
    let path = dir.join("recordings.jsonl");
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let mut w = BufWriter::new(file);
    let mut pending: HashMap<String, Recording> = HashMap::new();

    while let Ok(ev) = rx.recv() {
        match ev {
            RecordEvent::Input {
                call_id,
                input,
                provider,
            } => {
                let rec = pending
                    .entry(call_id.clone())
                    .or_insert_with(|| Recording::new(&call_id, input.clone(), provider.clone()));
                rec.input = input;
                rec.provider = provider;
            }
            RecordEvent::Provider { call_id, provider } => {
                if let Some(rec) = pending.get_mut(&call_id) {
                    rec.provider = provider;
                }
            }
            RecordEvent::Exchange { call_id, exchange } => {
                entry_or_init(&mut pending, &call_id)
                    .exchanges
                    .push(exchange);
                try_finalize(&mut w, &mut pending, &call_id);
            }
            RecordEvent::ExchangeUpdate {
                call_id,
                attempt,
                response,
                error,
            } => {
                if let Some(rec) = pending.get_mut(&call_id)
                    && let Some(ex) = rec.exchanges.iter_mut().find(|ex| ex.attempt == attempt)
                {
                    ex.response = Some(response);
                    if let Some(err) = error {
                        ex.error = Some(err);
                    }
                    ex.finalized = true;
                }
                try_finalize(&mut w, &mut pending, &call_id);
            }
            RecordEvent::TransportClosed { call_id } => {
                entry_or_init(&mut pending, &call_id).transport_closed = true;
                try_finalize(&mut w, &mut pending, &call_id);
            }
            RecordEvent::Outcome { call_id, outcome } => {
                entry_or_init(&mut pending, &call_id).outcome = outcome;
                try_finalize(&mut w, &mut pending, &call_id);
            }
            RecordEvent::Flush { ack } => {
                write_ready_all(&mut w, &mut pending);
                let _ = w.flush();
                // 阻塞 send:rendezvous 语义,确保 ack 一定被调用方收到。
                let _ = ack.send(());
            }
        }
    }

    // 所有 sender 已 drop:兜底把残余 pending 作为 incomplete 写出。
    for (_, rec) in pending.drain() {
        let mut rec = rec;
        rec.complete = false;
        if rec.outcome.status == OutcomeStatus::Pending {
            rec.outcome.status = OutcomeStatus::Incomplete;
        }
        write_line(&mut w, rec);
    }
    let _ = w.flush();
}

/// 若该 call 的所有分片到齐(barrier 满足)则写出,并标记 complete=true。
fn try_finalize(w: &mut impl Write, pending: &mut HashMap<String, Recording>, call_id: &str) {
    if pending.get(call_id).map(|r| r.ready()).unwrap_or(false)
        && let Some(mut rec) = pending.remove(call_id)
    {
        if !rec.exchanges.is_empty() || rec.transport_closed {
            rec.complete = true;
        }
        write_line(w, rec);
    }
}

fn write_ready_all(w: &mut impl Write, pending: &mut HashMap<String, Recording>) {
    let ids: Vec<String> = pending
        .iter()
        .filter(|(_, r)| r.ready())
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        if let Some(mut rec) = pending.remove(&id) {
            rec.complete = true;
            write_line(w, rec);
        }
    }
}

fn write_line(w: &mut impl Write, rec: Recording) {
    if let Ok(line) = serde_json::to_string(&rec) {
        let _ = writeln!(w, "{line}");
        let _ = w.flush(); // 每行落盘,保证崩溃前已写行可见。
    }
}

// ── 工具 ───────────────────────────────────────────────────────────────────

fn iso8601_now() -> String {
    // MVP:秒精度 + 时区标记 Z(毫秒精度留待列 `rfc3339` 公共 util 后统一)。
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format!("{:?}", d))
        .unwrap_or_default()
        .replace("Duration", "t")
}

// ── 流式 outcome 观测(B4:流结束时终结,含 drop/EOF 兜底)────────────────

/// 流式观测包装器:终结时记录 `OutcomeRecord`。
///
/// 终结语义:
/// - `StreamPart::Finish` → `Success`(finish_reason/usage 写入)
/// - `StreamPart::Error` → `Error`
/// - 流 item `Err` → `Error`
/// - EOF 前未见 Finish → `Incomplete`
/// - 消费方提前 drop(未终结)→ `Cancelled`
///
/// 只做观测,不改流内容。
pub struct RecordingOutcomeStream<S> {
    inner: S,
    recorder: Option<Arc<dyn Recorder>>,
    call_id: String,
    /// 已记录终结。
    recorded: bool,
}

impl<S> RecordingOutcomeStream<S> {
    /// 用层 A 入口取的 recorder 快照绑定流(全局替换不影响本次调用)。
    pub fn new(inner: S, recorder: Option<Arc<dyn Recorder>>, call_id: impl Into<String>) -> Self {
        Self {
            inner,
            recorder,
            call_id: call_id.into(),
            recorded: false,
        }
    }

    fn record(&mut self, outcome: &OutcomeRecord) {
        if !self.recorded {
            self.recorded = true;
            if let Some(rec) = &self.recorder {
                rec.record_outcome(&self.call_id, outcome);
                // P1: 流终结即传输层封闭(barrier 前提;P2 改由层 B 发)。
                rec.record_transport_closed(&self.call_id);
            }
        }
    }
}

impl<S, E> futures::Stream for RecordingOutcomeStream<S>
where
    S: futures::Stream<Item = Result<crate::stream_part::StreamPart, E>> + Unpin,
    E: std::fmt::Display,
{
    type Item = Result<crate::stream_part::StreamPart, E>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        let inner = std::pin::Pin::new(&mut this.inner);
        match inner.poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(part))) => {
                match &part {
                    crate::stream_part::StreamPart::Finish {
                        finish_reason,
                        usage,
                        ..
                    } => {
                        let outcome = OutcomeRecord {
                            status: OutcomeStatus::Success,
                            finish_reason: serde_json::to_value(finish_reason.unified)
                                .ok()
                                .and_then(|v| v.as_str().map(|s| s.to_string())),
                            error: None,
                            usage: serde_json::to_value(usage).ok(),
                        };
                        self.as_mut().get_mut().record(&outcome);
                    }
                    crate::stream_part::StreamPart::Error { error } => {
                        let outcome = OutcomeRecord {
                            status: OutcomeStatus::Error,
                            finish_reason: None,
                            error: Some(error.to_string()),
                            usage: None,
                        };
                        self.as_mut().get_mut().record(&outcome);
                    }
                    _ => {}
                }
                std::task::Poll::Ready(Some(Ok(part)))
            }
            std::task::Poll::Ready(Some(Err(e))) => {
                let outcome = OutcomeRecord {
                    status: OutcomeStatus::Error,
                    finish_reason: None,
                    error: Some(e.to_string()),
                    usage: None,
                };
                this.record(&outcome);
                std::task::Poll::Ready(Some(Err(e)))
            }
            std::task::Poll::Ready(None) => {
                let outcome = OutcomeRecord {
                    status: OutcomeStatus::Incomplete,
                    finish_reason: None,
                    error: None,
                    usage: None,
                };
                this.record(&outcome);
                std::task::Poll::Ready(None)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl<S> Drop for RecordingOutcomeStream<S> {
    fn drop(&mut self) {
        if !self.recorded {
            let outcome = OutcomeRecord {
                status: OutcomeStatus::Cancelled,
                finish_reason: None,
                error: None,
                usage: None,
            };
            self.record(&outcome);
        }
    }
}

// ── 测试 ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::GenerateTextOptions;

    fn sample_options() -> CallOptions {
        GenerateTextOptions {
            temperature: Some(0.7),
            ..Default::default()
        }
        .into_call_options(
            crate::language_model_message::convert_to_language_model_prompt(
                &[crate::message::ModelMessage::user("ping")],
                None,
            ),
        )
    }

    #[test]
    fn gate_is_off_by_default() {
        assert!(recorder().is_none());
    }

    #[test]
    fn call_id_is_unique() {
        let a = new_call_id();
        let b = new_call_id();
        assert_ne!(a, b);
        assert!(a.starts_with("call-"));
    }

    #[test]
    fn input_record_serializes_options_without_internal_fields() {
        let mut opts = sample_options();
        opts.call_id = Some("call-test".into());
        opts.abort_signal = None;
        let rec = InputRecord::from_call_options(&opts);
        let v = &rec.options;
        assert!(v.get("call_id").is_none());
        assert!(v.get("abort_signal").is_none());
    }

    #[test]
    fn redact_json_hides_sensitive_values_everywhere() {
        let v = serde_json::json!({
            "headers": { "Authorization": "Bearer sk-abc", "X-Key": "ok", "x-goog-api-key": "gkey" },
            "provider_options": { "headers": { "Cookie": "s=1" } },
            "body_overrides": { "api_key": "sk-secret" },
            "temperature": 0.5,
        });
        let r = redact_json(v);
        assert_eq!(r["headers"]["Authorization"], "[REDACTED]");
        assert_eq!(r["headers"]["x-goog-api-key"], "[REDACTED]");
        assert_eq!(r["headers"]["X-Key"], "ok");
        assert_eq!(r["provider_options"]["headers"]["Cookie"], "[REDACTED]");
        assert_eq!(r["body_overrides"]["api_key"], "[REDACTED]");
        assert_eq!(r["temperature"], 0.5);
    }

    #[test]
    fn recording_round_trips() {
        let rec = Recording::new(
            "call-1",
            InputRecord::from_call_options(&sample_options()),
            ProviderRecord::minimal("openai", "gpt-4o"),
        );
        let json = serde_json::to_string(&rec).unwrap();
        let back: Recording = serde_json::from_str(&json).unwrap();
        assert_eq!(back.call_id, "call-1");
        assert_eq!(back.schema, RECORDING_SCHEMA);
        assert_eq!(back.provider.provider, "openai");
    }

    #[test]
    fn jsonl_flush_writes_complete_line_blocking() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);

        rec.record_input("call-flush-1", &sample_options(), "openai", "gpt-4o");
        rec.record_outcome(
            "call-flush-1",
            &OutcomeRecord {
                status: OutcomeStatus::Success,
                finish_reason: Some("stop".into()),
                error: None,
                usage: None,
            },
        );
        // 层 B 缺失的 P1 语义:收尾声明传输封闭,barrier 才满足。
        rec.record_transport_closed("call-flush-1");
        // flush 阻塞直到落盘:无需轮询。
        rec.flush();
        let content = std::fs::read_to_string(rec.path()).unwrap();
        let parsed: Recording = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.call_id, "call-flush-1");
        assert_eq!(parsed.outcome.status, OutcomeStatus::Success);
        assert!(parsed.exchanges.is_empty()); // 无 exchange 也算 ready(非流式仅层 A)

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exchange_update_is_attached_to_correct_attempt() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-test2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);

        rec.record_input("call-x2", &sample_options(), "openai", "gpt-4o");
        // attempt 0 骨架 + attempt 1 补全。
        rec.record_exchange(
            "call-x2",
            &HttpExchange {
                attempt: 0,
                request: HttpRecord {
                    method: "post".into(),
                    url: "u".into(),
                    headers: vec![("authorization".into(), "[REDACTED]".into())],
                    body: Some("{}".into()),
                },
                response: None,
                timing: TimingRecord {
                    latency_ms: 1,
                    ttfb_ms: None,
                },
                error: None,
                finalized: false,
            },
        );
        rec.record_exchange_update(
            "call-x2",
            0,
            &ResponseRecord {
                status: 200,
                headers: vec![],
                body: Some("{\"ok\":true}".into()),
                stream_chunks: Some(1),
                ttfb_ms: None,
            },
            None,
        );
        rec.record_outcome(
            "call-x2",
            &OutcomeRecord {
                status: OutcomeStatus::Success,
                finish_reason: None,
                error: None,
                usage: None,
            },
        );
        rec.record_transport_closed("call-x2");
        rec.flush();

        let content = std::fs::read_to_string(rec.path()).unwrap();
        let parsed: Recording = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.exchanges.len(), 1);
        assert_eq!(parsed.exchanges[0].response.as_ref().unwrap().status, 200);
        assert!(parsed.exchanges[0].finalized);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn barrier_waits_for_transport_closed_and_marks_complete() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-btc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);

        rec.record_input("call-btc-1", &sample_options(), "openai", "gpt-4o");
        rec.record_outcome(
            "call-btc-1",
            &OutcomeRecord {
                status: OutcomeStatus::Success,
                finish_reason: None,
                error: None,
                usage: None,
            },
        );
        // outcome 已到但未封闭:flush 不应写出(无写行,文件为空)。
        rec.flush();
        let content = std::fs::read_to_string(rec.path()).unwrap();
        assert!(
            content.trim().is_empty(),
            "must not write before transport closed"
        );

        // 封闭后写出,且 complete=true。
        rec.record_transport_closed("call-btc-1");
        rec.flush();
        let content = std::fs::read_to_string(rec.path()).unwrap();
        let parsed: Recording = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.call_id, "call-btc-1");
        assert!(
            parsed.complete,
            "fully-finalized recording must be complete"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incomplete_outcome_flushes_on_shutdown() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-test3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let rec = JsonlRecorder::new(&dir);
            rec.record_input("call-orphan", &sample_options(), "openai", "gpt-4o");
            // 无 outcome:drop 触发断开 + join,writer 兜底写 incomplete(确定性)。
        }
        let content = std::fs::read_to_string(dir.join("recordings.jsonl"))
            .expect("disconnected fallback must write the incomplete line");
        let parsed: Recording = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.outcome.status, OutcomeStatus::Incomplete);
        assert!(
            !parsed.complete,
            "fallback line must not be marked complete"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
