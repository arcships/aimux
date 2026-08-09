//! RFC-0023: 调用上下文录制(P1 — 数据模型 + Recorder trait + 门控 + JsonlRecorder)。
//!
//! 录制一次 `generate_text`/`stream_text` 调用的三层完整上下文:
//! ① 输入侧(prompt + options)、② 配置侧(provider 身份)、③ HTTP 侧(wire 交换)。
//!
//! 关键性质(按 2026-08-06 定稿):
//! - **call_id 关联**:一次逻辑调用一个 `call_id`(与 RFC-0015/24/25 语义一致,
//!   区别于 HTTP 请求级 ID 与跨服务 trace)。
//! - **默认关闭**:不调 `init_recording`,热路径 = 1 读锁 + clone(次 ns 级)。
//! - **隐私受控**:api_key / Authorization / token 系恒脱敏(contains 式,含 `x-goog-api-key`、`x-amz-security-token`);
//!   `InputRecord.options` 序列化前递归脱敏。
//! - **completion barrier**:outcome 与全部 exchange(流式含终结)齐才写行。
//! - **专用 writer thread + oneshot flush**:同步 `flush()` 阻塞至落盘,不依赖运行时。
//! - **recorder 快照绑定**:层 A 入口取一次 `Arc<dyn Recorder>` 随调用,禁止各点重读。

use crate::language_model_message::LanguageModelPrompt;
use crate::options::CallOptions;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, RwLock};
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

    /// 会话归组(RFC-0024 P3):所在会话 id。None = 未归组(无 session_id 且
    /// 推断关闭)。由 `Recorder::record_session` 填充,写入 InputRecord 之前
    /// 或之后均可(writer 端按 call_id 合并)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 会话内步号(0 起,由 SessionStore 分配)。与 `session_id` 同生命周期。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
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
            session_id: None,
            step: None,
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
    /// 敏感头(authorization/cookie/含 api-key/key/token 等)已脱敏为 "[REDACTED]"。
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

/// 录制器初始化/刷盘错误(A9/N4:把原本静默/panic 的失败显式暴露为 `Result`)。
///
/// 注意:`Recorder::flush` 仍返回 `()` 以兼容 FFI/绑定/既有测试调用点
/// (`rec.flush()`);需要观测写失败时改用 [`Recorder::try_flush`]。
#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    /// `create_dir_all` 失败(`try_new`)。
    #[error("recording init failed for {path}: {source}")]
    Init {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// 打开 jsonl 文件失败(`try_new`)。
    #[error("failed to open recording file {path}: {source}")]
    OpenFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// 派生 writer 线程失败(`try_new`,替代原 `.expect` panic)。
    #[error("failed to spawn recording writer thread: {source}")]
    Spawn {
        #[source]
        source: std::io::Error,
    },
    /// writer 线程已退出(通道断),flush 无法送达。
    #[error("recording writer thread unavailable")]
    WriterGone,
    /// flush 在 30s 内未收到 writer 回执。
    #[error("recording flush timed out")]
    FlushTimeout,
}

/// 录制器 trait。
pub trait Recorder: Send + Sync {
    /// 录制输入侧 + 配置侧最小信息(层 A 入口调用)。
    fn record_input(&self, call_id: &str, options: &CallOptions, provider: &str, model_id: &str);
    /// 录制会话归组信息(RFC-0024 P3):session_id + 会话内步号。
    ///
    /// 默认空实现(不参与录制的 Recorder 无需关心);开启录制且调用被归组
    /// 时,`generate_text`/`stream_text` 入口调用它。
    fn record_session(&self, _call_id: &str, _session_id: &str, _step: u32) {}
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
    ///
    /// 返回 `()` 以兼容既有调用点(FFI `aimux_recording_flush`、绑定、测试)。
    /// 失败( writer 退出 / 超时)被静默吞掉;需观测写失败请用 [`try_flush`](Self::try_flush)。
    fn flush(&self);
    /// 显式 flush:同 [`flush`](Self::flush) 但把 writer 退出/超时等写失败作为
    /// `Result` 返回(A9/N4)。默认 `Ok(())`(无 I/O 的实现,如 `RingRecorder`)。
    fn try_flush(&self) -> Result<(), RecordingError> {
        Ok(())
    }
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
///
/// needle 集合与 `aimux_provider_utils::logging::is_sensitive_key` 对齐
/// (`authorization`/`api-key`/`apikey`/`key`/`token`),录制侧额外覆盖
/// `cookie`/`set-cookie`(logging 不脱敏 cookie)。其中 `key` 取 **exact** 匹配
/// 而非 contains——避免误伤 `X-Key`/`monkey`/`keyboard`/`keyword` 等含 "key"
/// 子串的非凭据名(既有 `redact_json` 测试即要求 `X-Key` 值保留)。其余 needle
/// 维持 contains,以覆盖 `x-goog-api-key`/`x-amz-security-token`/`proxy-
/// authorization` 等变体。
pub fn is_sensitive_key(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "cookie"
        || n == "set-cookie"
        || n == "key"
        || n.contains("authorization")
        || n.contains("api-key")
        || n.contains("api_key")
        || n.contains("apikey")
        || n.contains("token")
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
    /// 会话归组信息(RFC-0024 P3):所在会话 + 会话内步号。
    Session {
        call_id: String,
        session_id: String,
        step: u32,
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

/// JsonlRecorder 事件通道容量(有界,A4):防止 writer 跟不上时录制热路径
/// 无界堆积内存。满时 **drop-newest** 并递增 `dropped` 计数(经
/// [`JsonlRecorder::dropped_count`] 可查),保持 `send_ev` 非阻塞;writer
/// 已断开时同样丢弃并计数(失败可查而非静默)。
const JSONL_CHANNEL_CAPACITY: usize = 1024;

/// 每条完整 `Recording` 一行 jsonl;分片按 call_id 在专用线程合并。
/// 热路径仅 mpsc `try_send`(非阻塞,A4 有界 + drop-newest),I/O 全部在专用线程。
pub struct JsonlRecorder {
    tx: Option<SyncSender<RecordEvent>>,
    dir: PathBuf,
    thread: Option<std::thread::JoinHandle<()>>,
    /// 有界通道满 / writer 断开时丢弃的事件计数(drop-newest,可查)。
    dropped: AtomicU64,
    /// 被标记为不一致的 call_id(C4-7:重复 attempt / update 未命中或歧义)。
    /// 存于 recorder 侧而非 `Recording` 字段——后者被 `replay.rs` 以全字段
    /// 字面量构造,新增 serde 字段会破坏其它模块编译,故用旁路集合标记。
    inconsistent: Arc<Mutex<HashSet<String>>>,
}

impl JsonlRecorder {
    /// 显式构造(`try_new`,A9/N4):`create_dir_all` / 打开文件 / 派生 writer
    /// 线程失败均返回 `Err`(替代原 `.ok()` 静默与 `.expect` panic)。
    /// 文件:`{dir}/recordings.jsonl`。
    pub fn try_new(dir: impl Into<PathBuf>) -> Result<Self, RecordingError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|source| RecordingError::Init { path: dir.clone(), source })?;
        let path = dir.join("recordings.jsonl");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| RecordingError::OpenFile { path: path.clone(), source })?;
        let w = BufWriter::new(file);
        let (tx, rx) = sync_channel::<RecordEvent>(JSONL_CHANNEL_CAPACITY);
        let inconsistent = Arc::new(Mutex::new(HashSet::new()));
        let writer_inconsistent = inconsistent.clone();
        let handle = std::thread::Builder::new()
            .name("aimux-recording".into())
            .spawn(move || writer_loop(rx, w, writer_inconsistent))
            .map_err(|source| RecordingError::Spawn { source })?;
        Ok(Self {
            tx: Some(tx),
            dir,
            thread: Some(handle),
            dropped: AtomicU64::new(0),
            inconsistent,
        })
    }

    /// 在 `dir` 下创建录制器并启动 writer 线程(兼容入口:FFI `aimux_init_recording`
    /// 及绑定均以 infallible `new` 调用,A9 要求保持兼容)。内部委托 [`try_new`],
    /// 失败时降级为无 writer 的 no-op recorder(`tx = None`,事件静默丢弃),
    /// 行为等价于原先 `create_dir_all().ok()` + writer 打开文件失败早退。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self::try_new(dir.clone()).unwrap_or_else(|_| Self::disabled(dir))
    }

    /// 降级 recorder:writer 不可用,事件入队即丢(`send_ev` no-op)。
    fn disabled(dir: PathBuf) -> Self {
        Self {
            tx: None,
            dir,
            thread: None,
            dropped: AtomicU64::new(0),
            inconsistent: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 事件入队(有界 `try_send`;满或 writer 断开 → drop-newest 并计数)。
    fn send_ev(&self, ev: RecordEvent) {
        if let Some(tx) = &self.tx {
            send_or_drop(tx, ev, &self.dropped);
        }
    }

    /// 有界通道溢出 / writer 断开时丢弃的事件总数(A4,drop-newest 可查)。
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// 被标记为不一致的 call_id(C4-7,诊断用;排序以稳定测试)。
    pub fn inconsistent_call_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inconsistent.lock().unwrap().iter().cloned().collect();
        v.sort();
        v
    }

    /// 显式 flush(A9/N4):把 writer 退出 / 超时作为 `Result` 返回。阻塞至落盘。
    pub fn try_flush(&self) -> Result<(), RecordingError> {
        let Some(tx) = &self.tx else {
            return Err(RecordingError::WriterGone);
        };
        let (ack_tx, ack_rx) = sync_channel::<()>(0);
        // Flush 事件必须送达 writer(不可 drop-newest),故用阻塞 send 等待空位;
        // writer 断开时立即返回 Err。30s 回执超时兜底 writer 卡死。
        tx.send(RecordEvent::Flush { ack: ack_tx })
            .map_err(|_| RecordingError::WriterGone)?;
        ack_rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| RecordingError::FlushTimeout)
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

    fn record_session(&self, call_id: &str, session_id: &str, step: u32) {
        self.send_ev(RecordEvent::Session {
            call_id: call_id.to_string(),
            session_id: session_id.to_string(),
            step,
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

    /// 兼容 flush:委托 [`try_flush`](JsonlRecorder::try_flush) 并吞掉错误
    /// (FFI/绑定/既有测试以 `rec.flush()` 调用,需保持 `()` 返回)。
    fn flush(&self) {
        let _ = JsonlRecorder::try_flush(self);
    }

    fn try_flush(&self) -> Result<(), RecordingError> {
        JsonlRecorder::try_flush(self)
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
///
/// 文件已由 [`JsonlRecorder::try_new`] 打开并作为 `BufWriter` 传入(A9/N4:
/// 打开失败在 `try_new` 走 `Result`,而非此处静默早退)。`inconsistent` 为与
/// recorder 共享的旁路集合,用于标记 C4-7 检测到的不一致 call_id。
fn writer_loop(
    rx: Receiver<RecordEvent>,
    mut w: BufWriter<std::fs::File>,
    inconsistent: Arc<Mutex<HashSet<String>>>,
) {
    let mut pending: HashMap<String, Recording> = HashMap::new();

    while let Ok(ev) = rx.recv() {
        match ev {
            RecordEvent::Input {
                call_id,
                input,
                provider,
            } => {
                // C4-6:Input 仅在当前 provider 仍为空占位时填充最小 provider,
                // 避免乱序(Provider 先到)时把已写入的完整快照覆盖回最小值。
                let rec = pending
                    .entry(call_id.clone())
                    .or_insert_with(|| Recording::new(&call_id, input.clone(), provider.clone()));
                rec.input = input;
                if rec.provider.provider.is_empty() {
                    rec.provider = provider;
                }
            }
            RecordEvent::Session {
                call_id,
                session_id,
                step,
            } => {
                let rec = entry_or_init(&mut pending, &call_id);
                rec.session_id = Some(session_id);
                rec.step = Some(step);
                try_finalize(&mut w, &mut pending, &call_id);
            }
            // C4-6:用 entry_or_init(而非 get_mut)兜底建条目,保证乱序
            // (先于 Input 到达)的 Provider 快照不丢。
            RecordEvent::Provider { call_id, provider } => {
                entry_or_init(&mut pending, &call_id).provider = provider;
            }
            RecordEvent::Exchange { call_id, exchange } => {
                // C4-7:同 (call_id, attempt) 重复 → 合并骨架并标记 inconsistent。
                let rec = entry_or_init(&mut pending, &call_id);
                if insert_exchange(rec, exchange) {
                    mark_inconsistent(&inconsistent, &call_id);
                }
                try_finalize(&mut w, &mut pending, &call_id);
            }
            RecordEvent::ExchangeUpdate {
                call_id,
                attempt,
                response,
                error,
            } => {
                // C4-7:要求恰好一个匹配 attempt;0 或 >1 → 标记 inconsistent,
                // 不静默 patch 第一条。
                let rec = entry_or_init(&mut pending, &call_id);
                if !matches!(
                    apply_exchange_update(rec, attempt, response, error),
                    UpdateMatch::Patched
                ) {
                    mark_inconsistent(&inconsistent, &call_id);
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

// ── 合并/一致性 helper(C4-7)─────────────────────────────────────────────

/// `record_exchange_update` 的匹配结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateMatch {
    /// 恰好一个 attempt 匹配,已 patch。
    Patched,
    /// 无匹配骨架。
    NotFound,
    /// 多个匹配(重复 attempt),不静默 patch 第一条。
    Ambiguous,
}

/// 插入一条 exchange;若同 attempt 已存在则合并骨架并返回 `true`(调用方标记
/// inconsistent)。合并时保留已补全的 response/error/finalized(除非新骨架带值),
/// 避免重复插入导致的重复条目或更新丢失。
fn insert_exchange(rec: &mut Recording, exchange: HttpExchange) -> bool {
    if let Some(existing) = rec.exchanges.iter_mut().find(|e| e.attempt == exchange.attempt) {
        merge_exchange(existing, &exchange);
        true
    } else {
        rec.exchanges.push(exchange);
        false
    }
}

fn merge_exchange(existing: &mut HttpExchange, new: &HttpExchange) {
    existing.request = new.request.clone();
    existing.timing = new.timing.clone();
    if new.response.is_some() {
        existing.response = new.response.clone();
    }
    if new.error.is_some() {
        existing.error = new.error.clone();
    }
    // 不回退已终结状态:新骨架 finalized=true 才推进,避免重复骨架把已补全的打成未终结。
    if new.finalized {
        existing.finalized = true;
    }
}

/// 把 exchange 更新应用到恰好一个匹配 attempt;0 或 >1 匹配时不 patch,返回
/// `NotFound`/`Ambiguous` 供调用方标记 inconsistent(C4-7:不得静默 patch 第一条)。
fn apply_exchange_update(
    rec: &mut Recording,
    attempt: u32,
    response: ResponseRecord,
    error: Option<String>,
) -> UpdateMatch {
    let matches: Vec<usize> = rec
        .exchanges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.attempt == attempt)
        .map(|(i, _)| i)
        .collect();
    match matches.len() {
        0 => UpdateMatch::NotFound,
        1 => {
            let ex = &mut rec.exchanges[matches[0]];
            ex.response = Some(response);
            if let Some(err) = error {
                ex.error = Some(err);
            }
            ex.finalized = true;
            UpdateMatch::Patched
        }
        _ => UpdateMatch::Ambiguous,
    }
}

/// 标记 call_id 不一致(共享旁路集合;锁中毒时静默跳过,不影响写路径)。
fn mark_inconsistent(set: &Mutex<HashSet<String>>, call_id: &str) {
    if let Ok(mut g) = set.lock() {
        g.insert(call_id.to_string());
    }
}

// ── 有界 channel send(A4:drop-newest + 计数)─────────────────────────────

/// 有界 `try_send`:满(drop-newest)或 writer 断开时丢弃事件并递增 `dropped`,
/// 保持热路径非阻塞且失败可查(而非静默 `let _ = tx.send(...)`)。
fn send_or_drop(tx: &SyncSender<RecordEvent>, ev: RecordEvent, dropped: &AtomicU64) {
    if let Err(err) = tx.try_send(ev) {
        // err 持有未入队事件,随 err 析构丢弃;仅计数,保持非阻塞。
        drop(err);
        dropped.fetch_add(1, Ordering::Relaxed);
    }
}

// ── 工具 ───────────────────────────────────────────────────────────────────

/// 当前 UTC 时间,RFC 3339 格式(毫秒精度,`...Z`)。
fn iso8601_now() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_rfc3339_utc(d)
}

/// 由 `duration_since(UNIX_EPOCH)` 计算 RFC 3339 UTC(`YYYY-MM-DDTHH:MM:SS.mmmZ`)。
///
/// N11:原实现 `format!("{:?}", d)` 产出非 RFC3339(`t123s`),回放/审计无法解析。
/// 本 crate 未引入 chrono/humantime,故手写 Howard Hinnant `civil_from_days`
/// 算法(与 `session::rfc3339_now` 同格式同算法),纯整数运算,无 panic。
fn format_rfc3339_utc(d: Duration) -> String {
    let secs = d.as_secs() as i64;
    let millis = d.subsec_millis();
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let hms = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        hms / 3600,
        (hms % 3600) / 60,
        hms % 60,
    )
}

/// 自 1970-01-01 起的天数 → (year, month, day)。`z >= 0` 对当前纪元恒成立。
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

// ── RingRecorder(P6:内存有界 ring;同 RingTraceStore 样式各自实现)────────

/// 默认 ring 容量(与 RFC-0023 §3.5 对齐:2048 条)。
pub const DEFAULT_RING_CAPACITY: usize = 2048;

/// 内存有界录制器:分片按 `call_id` 合并,completion barrier 满足后入 ring,
/// FIFO 淘汰最旧(默认 2048 条),丢弃计数对外可查。**不落盘**。
///
/// 与 [`crate::trace::RingTraceStore`] 同款样式(Mutex + VecDeque +
/// with_capacity + export_jsonl),不共用类型。适用进程内近端调试 / 测试,
/// 无需文件 I/O;落盘用 [`JsonlRecorder`]。
pub struct RingRecorder {
    inner: Mutex<RingInner>,
}

/// pending 上限(独立于 ring cap 的绝对上界,A3):防止未完成分片无界堆积。
/// 取 `cap * 2`(饱和,至少 2),既与 ring 规模联动又保证测试可用小 cap 触发。
fn ring_pending_max(cap: usize) -> usize {
    cap.saturating_mul(2).max(2)
}

struct RingInner {
    cap: usize,
    /// pending 上限(A3):超过即淘汰最旧 pending → 转 incomplete 入 ring。
    pending_max: usize,
    /// pending 插入顺序(淘汰最旧用;finalize/remove 不主动摘除,惰性清理)。
    pending_order: VecDeque<String>,
    /// 未完成分片(barrier 未满足),按 call_id 合并。
    pending: HashMap<String, Recording>,
    /// 已完成录制(ready 后迁入),FIFO ring。
    completed: VecDeque<Recording>,
    /// ring 超容淘汰计数(队列语义:bounded + drop-newest 可查)。
    dropped: u64,
    /// pending 超限淘汰计数(A3,可查)。
    pending_evicted: u64,
    /// 被标记为不一致的 call_id(C4-7)。
    inconsistent: HashSet<String>,
}

impl RingRecorder {
    /// 默认容量(2048 条)。
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RING_CAPACITY)
    }

    pub fn with_capacity(cap: usize) -> Self {
        assert!(cap > 0, "RingRecorder capacity must be > 0");
        Self {
            inner: Mutex::new(RingInner {
                cap,
                pending_max: ring_pending_max(cap),
                pending_order: VecDeque::new(),
                pending: HashMap::new(),
                completed: VecDeque::with_capacity(cap),
                dropped: 0,
                pending_evicted: 0,
                inconsistent: HashSet::new(),
            }),
        }
    }

    /// 已完成录制条数。
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().completed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 尚未完成(仍等待分片)的 call 数。
    pub fn pending_count(&self) -> usize {
        self.inner.lock().unwrap().pending.len()
    }

    /// FIFO 淘汰丢弃计数(bounded + drop-newest 对外可查)。
    pub fn dropped_count(&self) -> u64 {
        self.inner.lock().unwrap().dropped
    }

    /// pending 超限淘汰计数(A3,可查)。
    pub fn pending_evicted_count(&self) -> u64 {
        self.inner.lock().unwrap().pending_evicted
    }

    /// 被标记为不一致的 call_id(C4-7,排序以稳定测试)。
    pub fn inconsistent_call_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.lock().unwrap().inconsistent.iter().cloned().collect();
        v.sort();
        v
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending.clear();
        inner.pending_order.clear();
        inner.completed.clear();
        inner.inconsistent.clear();
        inner.dropped = 0;
        inner.pending_evicted = 0;
    }

    /// 导出全部已完成录制(每行一个 `Recording`)。pending 未完成条目不导出
    /// (但被 A3 淘汰的 pending 会以 incomplete 形式进 ring,可导出)。
    pub fn export_jsonl(&self, w: &mut impl Write) -> std::io::Result<()> {
        let inner = self.inner.lock().unwrap();
        for rec in &inner.completed {
            let line = serde_json::to_string(rec).map_err(std::io::Error::other)?;
            writeln!(w, "{line}")?;
        }
        Ok(())
    }
}

impl RingInner {
    /// 取或建 pending 条目(有界,A3):新 call_id 触发惰性清理 + 必要时淘汰最旧。
    fn entry_or_init_bounded(&mut self, call_id: &str) -> &mut Recording {
        if !self.pending.contains_key(call_id) {
            self.evict_pending_if_needed();
            self.pending_order.push_back(call_id.to_string());
            self.pending.insert(
                call_id.to_string(),
                Recording::new(
                    call_id,
                    InputRecord {
                        prompt: Vec::new(),
                        options: serde_json::Value::Null,
                    },
                    ProviderRecord::minimal("", ""),
                ),
            );
        }
        self.pending.get_mut(call_id).expect("just inserted or present")
    }

    /// 惰性清理已终结的顺序条目,再淘汰最旧 pending 直到低于上限(A3)。
    /// 淘汰的 pending 转 `complete=false` 的 incomplete 记录入 ring(不静默丢)。
    fn evict_pending_if_needed(&mut self) {
        while self
            .pending_order
            .front()
            .is_some_and(|id| !self.pending.contains_key(id))
        {
            self.pending_order.pop_front();
        }
        while self.pending.len() >= self.pending_max {
            let Some(old) = self.pending_order.pop_front() else { break };
            if let Some(mut rec) = self.pending.remove(&old) {
                rec.complete = false;
                if rec.outcome.status == OutcomeStatus::Pending {
                    rec.outcome.status = OutcomeStatus::Incomplete;
                }
                self.pending_evicted += 1;
                self.push(rec);
            }
        }
    }

    /// 合并一个已 ready 的分片(barrier 满足则入 ring)。
    fn finalize(&mut self, call_id: &str) {
        if self.pending.get(call_id).is_some_and(|r| r.ready())
            && let Some(mut rec) = self.pending.remove(call_id)
        {
            if !rec.exchanges.is_empty() || rec.transport_closed {
                rec.complete = true;
            }
            self.push(rec);
        }
    }

    /// FIFO ring 追加;超容淘汰最旧并计数。
    fn push(&mut self, rec: Recording) {
        if self.completed.len() >= self.cap {
            self.completed.pop_front();
            self.dropped += 1;
        }
        self.completed.push_back(rec);
    }

    fn mark_inconsistent(&mut self, call_id: &str) {
        self.inconsistent.insert(call_id.to_string());
    }
}

impl Default for RingRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder for RingRecorder {
    fn record_input(&self, call_id: &str, options: &CallOptions, provider: &str, model_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        // C4-6 + A3:有界建条目;Input 仅在 provider 仍为空占位时填最小 provider,
        // 避免乱序(Provider 先到)覆盖完整快照。
        let rec = inner.entry_or_init_bounded(call_id);
        rec.input = InputRecord::from_call_options(options);
        if rec.provider.provider.is_empty() {
            rec.provider = ProviderRecord::minimal(provider, model_id);
        }
    }

    fn record_provider(&self, call_id: &str, snapshot: &ProviderRecord) {
        // 与 JsonlRecorder 一致的边界强制脱敏,不依赖 provider 端自觉。
        let mut snap = snapshot.clone();
        snap.provider_options = snap.provider_options.take().map(redact_json);
        snap.profile = snap.profile.take().map(redact_json);
        let mut inner = self.inner.lock().unwrap();
        // C4-6:entry_or_init_bounded 兜底建条目,保证乱序 Provider 快照不丢。
        inner.entry_or_init_bounded(call_id).provider = snap;
    }

    fn record_exchange(&self, call_id: &str, exchange: &HttpExchange) {
        let mut inner = self.inner.lock().unwrap();
        // C4-7:同 attempt 重复 → 合并并标记 inconsistent。
        let rec = inner.entry_or_init_bounded(call_id);
        if insert_exchange(rec, exchange.clone()) {
            inner.mark_inconsistent(call_id);
        }
        inner.finalize(call_id);
    }

    fn record_exchange_update(
        &self,
        call_id: &str,
        attempt: u32,
        response: &ResponseRecord,
        error: Option<String>,
    ) {
        let mut inner = self.inner.lock().unwrap();
        // C4-7:要求恰好一个匹配;0 或 >1 → 标记 inconsistent,不静默 patch 第一条。
        let rec = inner.entry_or_init_bounded(call_id);
        if !matches!(
            apply_exchange_update(rec, attempt, response.clone(), error),
            UpdateMatch::Patched
        ) {
            inner.mark_inconsistent(call_id);
        }
        inner.finalize(call_id);
    }

    fn record_outcome(&self, call_id: &str, outcome: &OutcomeRecord) {
        let mut inner = self.inner.lock().unwrap();
        inner.entry_or_init_bounded(call_id).outcome = outcome.clone();
        inner.finalize(call_id);
    }

    fn record_transport_closed(&self, call_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.entry_or_init_bounded(call_id).transport_closed = true;
        inner.finalize(call_id);
    }

    /// 内存模式:把 barrier 已满足的 pending 全部收进 completed(对应
    /// JsonlRecorder 的 write_ready_all),无落盘等待。
    fn flush(&self) {
        let mut inner = self.inner.lock().unwrap();
        let ids: Vec<String> = inner
            .pending
            .iter()
            .filter(|(_, r)| r.ready())
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(mut rec) = inner.pending.remove(&id) {
                rec.complete = true;
                inner.push(rec);
            }
        }
    }

    /// 内存录制无 I/O,flush 不会失败:委托 [`flush`](Self::flush) 后返回 `Ok`。
    fn try_flush(&self) -> Result<(), RecordingError> {
        RingRecorder::flush(self);
        Ok(())
    }
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

    // ── RFC-0024 P3: session 归组信息进 Recording ─────────────────────────

    #[test]
    fn record_session_writes_session_fields_to_jsonl() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-sess-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);

        rec.record_input("call-sess-1", &sample_options(), "openai", "gpt-4o");
        // session 事件可在 Input 之后到达(writer 按 call_id 合并)。
        rec.record_session("call-sess-1", "sess-abc", 3);
        rec.record_outcome(
            "call-sess-1",
            &OutcomeRecord {
                status: OutcomeStatus::Success,
                finish_reason: Some("stop".into()),
                error: None,
                usage: None,
            },
        );
        rec.record_transport_closed("call-sess-1");
        rec.flush();

        let content = std::fs::read_to_string(rec.path()).unwrap();
        let parsed: Recording = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.session_id.as_deref(), Some("sess-abc"));
        assert_eq!(parsed.step, Some(3));
        // 序列化时 null 省略:只有归组的调用才带这两个字段。
        assert!(content.contains("\"session_id\":\"sess-abc\""));
        assert!(content.contains("\"step\":3"));
    }

    #[test]
    fn ungrouped_recording_omits_session_fields_and_old_jsonl_parses() {
        // 未归组:session 字段为 None 且序列化时省略(skip_serializing_if)。
        let rec = Recording::new(
            "call-1",
            InputRecord::from_call_options(&sample_options()),
            ProviderRecord::minimal("openai", "gpt-4o"),
        );
        let json = serde_json::to_string(&rec).unwrap();
        assert!(!json.contains("session_id"), "ungrouped must omit: {json}");
        assert!(!json.contains("\"step\""), "ungrouped must omit: {json}");

        // 旧 jsonl(无 session 字段)仍能反序列化(serde default)。
        let old_json = serde_json::to_string(&{
            let mut r = serde_json::to_value(&rec).unwrap();
            r.as_object_mut()
                .unwrap()
                .insert("transport_closed".into(), serde_json::json!(true));
            r
        })
        .unwrap();
        let back: Recording = serde_json::from_str(&old_json).unwrap();
        assert_eq!(back.session_id, None);
        assert_eq!(back.step, None);
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
    fn error_patches_existing_exchange_not_duplicate() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-errpatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);
        rec.record_input("call-errp", &sample_options(), "openai", "gpt-4o");
        rec.record_exchange(
            "call-errp",
            &HttpExchange {
                attempt: 3,
                request: HttpRecord {
                    method: "post".into(),
                    url: "u".into(),
                    headers: vec![],
                    body: Some("{}".into()),
                },
                response: Some(ResponseRecord {
                    status: 200,
                    headers: vec![],
                    body: None,
                    stream_chunks: None,
                    ttfb_ms: None,
                }),
                timing: TimingRecord {
                    latency_ms: 1,
                    ttfb_ms: None,
                },
                error: None,
                finalized: false,
            },
        );
        // 流中途 error → patch 到同一条(骨架),带 body 补全 + error(S1)。
        rec.record_exchange_update(
            "call-errp",
            3,
            &ResponseRecord {
                status: 200,
                headers: vec![],
                body: Some(r#"{"partial":true}"#.into()),
                stream_chunks: Some(1),
                ttfb_ms: Some(12),
            },
            Some("mid-stream error".to_string()),
        );
        rec.record_transport_closed("call-errp");
        rec.record_outcome(
            "call-errp",
            &OutcomeRecord {
                status: OutcomeStatus::Error,
                finish_reason: None,
                error: Some("mid-stream".into()),
                usage: None,
            },
        );
        rec.flush();
        let content = std::fs::read_to_string(rec.path()).unwrap();
        let parsed: Recording = serde_json::from_str(content.trim()).unwrap();
        // 只有 1 条 exchange(骨架 patch,不重复)。
        assert_eq!(
            parsed.exchanges.len(),
            1,
            "error must patch same attempt, not duplicate"
        );
        assert_eq!(parsed.exchanges[0].attempt, 3);
        let ex = &parsed.exchanges[0];
        assert!(ex.finalized);
        assert_eq!(ex.error.as_deref(), Some("mid-stream error"));
        // body 补全 + ttfb 保留。
        assert!(
            ex.response
                .as_ref()
                .unwrap()
                .body
                .as_deref()
                .unwrap()
                .contains("partial")
        );
        assert_eq!(ex.response.as_ref().unwrap().ttfb_ms, Some(12));
        // error 已 patch 进骨架 + finalized + closed → wire 完整 → complete。
        assert!(parsed.complete, "finalized exchange + closed -> complete");
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

    // ── RingRecorder(P6)───────────────────────────────────────────────────

    fn sample_exchange() -> HttpExchange {
        HttpExchange {
            attempt: 0,
            request: HttpRecord {
                method: "post".into(),
                url: "https://api.openai.com/v1/chat/completions".into(),
                headers: vec![],
                body: Some("{}".into()),
            },
            response: Some(ResponseRecord {
                status: 200,
                headers: vec![],
                body: Some("{\"ok\":true}".into()),
                stream_chunks: None,
                ttfb_ms: None,
            }),
            timing: TimingRecord {
                latency_ms: 1,
                ttfb_ms: None,
            },
            error: None,
            finalized: true,
        }
    }

    fn success_outcome() -> OutcomeRecord {
        OutcomeRecord {
            status: OutcomeStatus::Success,
            finish_reason: Some("stop".into()),
            error: None,
            usage: None,
        }
    }

    /// 驱动一条完整调用:input + exchange + outcome + transport_closed。
    fn drive_full_call(rec: &RingRecorder, call_id: &str) {
        rec.record_input(call_id, &sample_options(), "openai", "gpt-4o");
        rec.record_exchange(call_id, &sample_exchange());
        rec.record_outcome(call_id, &success_outcome());
        rec.record_transport_closed(call_id);
    }

    #[test]
    fn ring_merges_shards_and_finalizes_on_barrier() {
        let ring = RingRecorder::new();
        drive_full_call(&ring, "call-a");
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.pending_count(), 0);

        let mut buf = Vec::new();
        ring.export_jsonl(&mut buf).unwrap();
        let parsed: Recording =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert_eq!(parsed.call_id, "call-a");
        assert_eq!(parsed.exchanges.len(), 1);
        assert_eq!(parsed.outcome.status, OutcomeStatus::Success);
        assert!(parsed.complete);
    }

    #[test]
    fn ring_keeps_incomplete_in_pending_until_closed() {
        let ring = RingRecorder::new();
        ring.record_input("call-x", &sample_options(), "openai", "gpt-4o");
        ring.record_outcome("call-x", &success_outcome());
        // outcome 已到但 transport 未封闭:仍 pending,不进 completed。
        ring.flush();
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.pending_count(), 1);

        ring.record_transport_closed("call-x");
        ring.flush();
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.pending_count(), 0);
    }

    #[test]
    fn ring_evicts_oldest_and_counts_drops() {
        let ring = RingRecorder::with_capacity(2);
        drive_full_call(&ring, "call-1");
        drive_full_call(&ring, "call-2");
        drive_full_call(&ring, "call-3");
        assert_eq!(ring.len(), 2, "ring is bounded");
        assert_eq!(ring.dropped_count(), 1, "oldest evicted and counted");

        // 淘汰的是最旧的 call-1。
        let mut buf = Vec::new();
        ring.export_jsonl(&mut buf).unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&buf)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("call-2"));
        assert!(lines[1].contains("call-3"));
    }

    #[test]
    fn ring_flush_finalizes_ready_pending() {
        let ring = RingRecorder::new();
        ring.record_input("call-f", &sample_options(), "openai", "gpt-4o");
        ring.record_outcome("call-f", &success_outcome());
        ring.record_transport_closed("call-f");
        // 不主动 flush 前,exchange 无但 barrier 已满足(无 exchange 也算 ready)——
        // record_outcome/closed 路径已 finalize;此处验证 flush 幂等(不重复入 ring)。
        ring.flush();
        ring.flush();
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn ring_clear_resets_everything() {
        let ring = RingRecorder::with_capacity(2);
        drive_full_call(&ring, "call-1");
        ring.record_input("call-p", &sample_options(), "openai", "gpt-4o"); // pending
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.pending_count(), 1);

        ring.clear();
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.pending_count(), 0);
        assert_eq!(ring.dropped_count(), 0);
    }

    // ── N11:RFC 3339 时间戳 ────────────────────────────────────────────────

    /// `civil_from_days` 的逆运算(独立实现,交叉验证 `format_rfc3339_utc`)。
    fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = (if y >= 0 { y } else { y - 399 }) / 400;
        let yoe = (y - era * 400) as u64;
        let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
        let doy = (153 * mp + 2) / 5 + d as u64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe as i64 - 719_468
    }

    /// 解析 `format_rfc3339_utc` 产物 → (总秒数, 毫秒)。None = 格式不符。
    fn parse_rfc3339_secs(s: &str) -> Option<(i64, u32)> {
        let b = s.as_bytes();
        if s.len() < 24
            || b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b'T'
            || b[13] != b':'
            || b[16] != b':'
            || b[19] != b'.'
            || !s.ends_with('Z')
        {
            return None;
        }
        let year: i64 = s[0..4].parse().ok()?;
        let month: u32 = s[5..7].parse().ok()?;
        let day: u32 = s[8..10].parse().ok()?;
        let hour: u64 = s[11..13].parse().ok()?;
        let min: u64 = s[14..16].parse().ok()?;
        let sec: u64 = s[17..19].parse().ok()?;
        let millis: u32 = s[20..s.len() - 1].parse().ok()?;
        let total = days_from_civil(year, month, day) * 86_400 + (hour * 3600 + min * 60 + sec) as i64;
        Some((total, millis))
    }

    #[test]
    fn format_rfc3339_utc_is_valid_rfc3339() {
        // epoch。
        assert_eq!(format_rfc3339_utc(Duration::ZERO), "1970-01-01T00:00:00.000Z");
        // 1_700_000_000s(独立 days_from_civil 交叉验证)。
        let d = Duration::from_secs(1_700_000_000);
        let s = format_rfc3339_utc(d);
        assert_eq!(s.len(), 24);
        assert!(s.ends_with('Z'));
        let (secs, millis) = parse_rfc3339_secs(&s).expect("must parse");
        assert_eq!(secs, 1_700_000_000);
        assert_eq!(millis, 0);
        // 毫秒精度保留。
        let (s2, m2) = parse_rfc3339_secs(&format_rfc3339_utc(Duration::from_millis(12_345))).unwrap();
        assert_eq!((s2, m2), (12, 345));
        // iso8601_now() 同格式。
        let now = iso8601_now();
        assert!(parse_rfc3339_secs(&now).is_some(), "now not rfc3339: {now}");
    }

    // ── A4:有界 channel + drop-newest 计数 ────────────────────────────────

    #[test]
    fn send_or_drop_counts_overflow() {
        // 容量 1 + 不消费:第 1 条入队,后两条 Full → drop-newest 并计数。
        let (tx, rx) = sync_channel::<RecordEvent>(1);
        let dropped = AtomicU64::new(0);
        let ev = || RecordEvent::TransportClosed { call_id: "c".into() };
        send_or_drop(&tx, ev(), &dropped);
        send_or_drop(&tx, ev(), &dropped);
        send_or_drop(&tx, ev(), &dropped);
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
        assert!(rx.try_recv().is_ok()); // 第 1 条仍在
        assert!(rx.try_recv().is_err()); // 无更多
    }

    #[test]
    fn jsonl_dropped_count_starts_zero() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);
        assert_eq!(rec.dropped_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── A9/N4:try_new / try_flush ──────────────────────────────────────────

    #[test]
    fn jsonl_try_new_fails_on_invalid_dir() {
        let blocker = std::env::temp_dir().join(format!("aimux-rec-blocker-{}", std::process::id()));
        let _ = std::fs::remove_file(&blocker);
        std::fs::write(&blocker, b"x").unwrap();
        // blocker 是文件 → create_dir_all(blocker/sub) 失败 → Err(Init)。
        let res = JsonlRecorder::try_new(blocker.join("sub"));
        assert!(
            matches!(res, Err(RecordingError::Init { .. })),
            "expected Init, got {:?}",
            res.as_ref().err()
        );
        let _ = std::fs::remove_file(&blocker);
    }

    #[test]
    fn jsonl_try_flush_ok_and_disabled_err() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-tf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);
        rec.record_input("c", &sample_options(), "openai", "gpt-4o");
        rec.record_outcome("c", &success_outcome());
        rec.record_transport_closed("c");
        assert!(rec.try_flush().is_ok());
        let content = std::fs::read_to_string(rec.path()).unwrap();
        assert!(content.contains("\"call_id\":\"c\""));
        let _ = std::fs::remove_dir_all(&dir);

        // 非法目录 → new 降级(tx=None)→ try_flush 返回 WriterGone。
        let blocker = std::env::temp_dir().join(format!("aimux-rec-blocker2-{}", std::process::id()));
        let _ = std::fs::remove_file(&blocker);
        std::fs::write(&blocker, b"x").unwrap();
        let disabled = JsonlRecorder::new(blocker.join("sub"));
        assert!(matches!(disabled.try_flush(), Err(RecordingError::WriterGone)));
        let _ = std::fs::remove_file(&blocker);
    }

    // ── A3:RingRecorder pending 有界 ───────────────────────────────────────

    #[test]
    fn ring_pending_evicts_oldest_when_bounded() {
        let ring = RingRecorder::with_capacity(2); // pending_max = 4
        for i in 1..=5u32 {
            ring.record_input(&format!("call-{i}"), &sample_options(), "openai", "gpt-4o");
        }
        assert_eq!(ring.pending_count(), 4, "pending bounded to 4");
        assert_eq!(ring.len(), 1, "oldest evicted into ring as incomplete");
        assert_eq!(ring.pending_evicted_count(), 1);

        let mut buf = Vec::new();
        ring.export_jsonl(&mut buf).unwrap();
        let parsed: Recording =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert_eq!(parsed.call_id, "call-1", "oldest (call-1) evicted");
        assert!(!parsed.complete, "evicted pending → incomplete");
        assert_eq!(parsed.outcome.status, OutcomeStatus::Incomplete);
    }

    // ── C4-6:Provider 先于 Input 不丢 ─────────────────────────────────────

    fn full_provider() -> ProviderRecord {
        ProviderRecord {
            provider: "openai".into(),
            model_id: "gpt-4o".into(),
            base_url: Some("https://api.openai.com".into()),
            api_key_source: "env:OPENAI_API_KEY".into(),
            profile: None,
            provider_options: None,
        }
    }

    #[test]
    fn ring_provider_before_input_preserved() {
        let ring = RingRecorder::new();
        ring.record_provider("c", &full_provider()); // 乱序:Provider 先到
        ring.record_input("c", &sample_options(), "minimal", "minimal-model");
        ring.record_outcome("c", &success_outcome());
        ring.record_transport_closed("c");

        let mut buf = Vec::new();
        ring.export_jsonl(&mut buf).unwrap();
        let parsed: Recording =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert_eq!(parsed.provider.provider, "openai");
        assert_eq!(
            parsed.provider.base_url.as_deref(),
            Some("https://api.openai.com"),
            "full snapshot preserved, not overwritten by minimal"
        );
        assert_eq!(parsed.provider.model_id, "gpt-4o");
    }

    #[test]
    fn jsonl_provider_before_input_preserved() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-pbi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);
        rec.record_provider("c", &full_provider()); // 乱序:Provider 先到
        rec.record_input("c", &sample_options(), "minimal", "minimal-model");
        rec.record_outcome("c", &success_outcome());
        rec.record_transport_closed("c");
        rec.flush();
        let content = std::fs::read_to_string(rec.path()).unwrap();
        let parsed: Recording = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.provider.base_url.as_deref(), Some("https://api.openai.com"));
        assert_eq!(parsed.provider.provider, "openai");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── C4-7:重复 attempt / update 匹配语义 ────────────────────────────────

    fn resp_status(status: u16) -> ResponseRecord {
        ResponseRecord {
            status,
            headers: vec![],
            body: None,
            stream_chunks: None,
            ttfb_ms: None,
        }
    }

    fn skeleton_at(attempt: u32) -> HttpExchange {
        HttpExchange {
            attempt,
            request: HttpRecord {
                method: "post".into(),
                url: "u".into(),
                headers: vec![],
                body: None,
            },
            response: None,
            timing: TimingRecord {
                latency_ms: 1,
                ttfb_ms: None,
            },
            error: None,
            finalized: false,
        }
    }

    fn empty_recording(call_id: &str) -> Recording {
        Recording::new(
            call_id,
            InputRecord {
                prompt: Vec::new(),
                options: serde_json::Value::Null,
            },
            ProviderRecord::minimal("", ""),
        )
    }

    #[test]
    fn insert_exchange_and_apply_update_match_semantics() {
        let mut rec = empty_recording("c");

        // 新 attempt → 插入(false)。
        assert!(!insert_exchange(&mut rec, skeleton_at(0)));
        assert!(!insert_exchange(&mut rec, skeleton_at(1)));
        assert_eq!(rec.exchanges.len(), 2);

        // 同 attempt 重复 → 合并(true),条目数不变,response/error/finalized 合并进。
        let mut dup = skeleton_at(0);
        dup.response = Some(resp_status(500));
        dup.error = Some("dup".into());
        dup.finalized = true;
        assert!(insert_exchange(&mut rec, dup));
        assert_eq!(rec.exchanges.len(), 2);
        let e0 = rec.exchanges.iter().find(|e| e.attempt == 0).unwrap();
        assert_eq!(e0.response.as_ref().unwrap().status, 500);
        assert!(e0.finalized);
        assert_eq!(e0.error.as_deref(), Some("dup"));

        // apply:1 匹配 → Patched。
        assert_eq!(
            apply_exchange_update(&mut rec, 1, resp_status(200), None),
            UpdateMatch::Patched
        );
        assert_eq!(
            rec.exchanges
                .iter()
                .find(|e| e.attempt == 1)
                .unwrap()
                .response
                .as_ref()
                .unwrap()
                .status,
            200
        );

        // apply:0 匹配 → NotFound。
        assert_eq!(
            apply_exchange_update(&mut rec, 9, resp_status(200), None),
            UpdateMatch::NotFound
        );

        // apply:2 匹配(手动塞重复)→ Ambiguous,不 patch 任何一条。
        rec.exchanges.push(skeleton_at(1));
        assert_eq!(
            apply_exchange_update(&mut rec, 1, resp_status(503), Some("amb".into())),
            UpdateMatch::Ambiguous
        );
        assert!(rec
            .exchanges
            .iter()
            .filter(|e| e.attempt == 1)
            .all(|e| e.response.as_ref().is_none_or(|r| r.status != 503)));
    }

    #[test]
    fn ring_duplicate_attempt_marks_inconsistent_and_merges() {
        let ring = RingRecorder::new();
        ring.record_input("c", &sample_options(), "openai", "gpt-4o");
        ring.record_exchange("c", &sample_exchange()); // attempt 0
        ring.record_exchange("c", &sample_exchange()); // 重复 attempt 0 → 合并 + 标记
        assert!(ring.inconsistent_call_ids().contains(&"c".to_string()));

        // 合并后仍只有 1 条 attempt=0;补齐 barrier 后导出验证。
        ring.record_outcome("c", &success_outcome());
        ring.record_transport_closed("c");
        let mut buf = Vec::new();
        ring.export_jsonl(&mut buf).unwrap();
        let parsed: Recording =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert_eq!(parsed.exchanges.len(), 1, "duplicate merged, not appended");
        assert_eq!(parsed.exchanges[0].attempt, 0);
    }

    #[test]
    fn ring_update_not_found_marks_inconsistent() {
        let ring = RingRecorder::new();
        ring.record_input("c", &sample_options(), "openai", "gpt-4o");
        // 无骨架直接 update → 0 匹配 → 标记 inconsistent,不静默丢弃。
        ring.record_exchange_update("c", 0, &resp_status(200), None);
        assert!(ring.inconsistent_call_ids().contains(&"c".to_string()));
        assert_eq!(ring.pending_count(), 1, "call still pending, no exchange created");
    }

    #[test]
    fn jsonl_duplicate_attempt_marks_inconsistent() {
        let dir = std::env::temp_dir().join(format!("aimux-rec-dup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = JsonlRecorder::new(&dir);
        rec.record_input("c", &sample_options(), "openai", "gpt-4o");
        rec.record_exchange("c", &sample_exchange());
        rec.record_exchange("c", &sample_exchange()); // 重复 attempt 0
        rec.flush();
        assert!(rec.inconsistent_call_ids().contains(&"c".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
