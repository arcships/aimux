//! Session aggregation (RFC-0024): session_id 归组基础设施.
//!
//! Groups consecutive calls into sessions via an explicit `session_id`
//! (`CallOptions.session_id`, explicit-first) with an optional prompt-prefix
//! continuation inferer as fallback (opt-in, off by default). The session
//! index itself is an in-process, bounded LRU — it is never persisted
//! (RFC-0024 §6.5: Recording/TraceRecord keep the `session_id` field, so an
//! index can be rebuilt after a restart).
//!
//! Registration is explicit and opt-in, mirroring RFC-0023's `init_recording`
//! pattern: nothing is recorded and no state exists until
//! [`init_session_store`] is called. The optional inferer can be enabled
//! programmatically ([`init_session_infer`]) or via `AIMUX_SESSION_INFER=1`
//! (checked once, lazily, like RFC-0014's env auto-init).
//!
//! This module is the observability layer — it is orthogonal to RFC-0019
//! (session-affinity headers for upstream routing) and does not model fork
//! semantics, agent loops, or chain structures.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::language_model_message::LanguageModelPrompt;

// ─────────────────────────────────────────────────────────────────────────────
// Data model
// ─────────────────────────────────────────────────────────────────────────────

/// Where a session id came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SessionSource {
    /// Passed explicitly via `CallOptions.session_id` / `GenerateTextOptions.session_id`.
    Explicit,
    /// Produced by the opt-in `SessionInferer` (ids carry an `auto-` prefix).
    Inferred,
}

/// A single call within a session (an index entry only — call content lives
/// in Recording (RFC-0023) / TraceRecord (RFC-0015)).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionCall {
    /// Call-level unique id. This is the association key for the future
    /// Recording / TraceRecord `call_id` (RFC-0015/0023): once those land,
    /// this field carries their value.
    pub call_id: String,
    /// Step within the session (0-based).
    pub step: u32,
    /// When the call was recorded (RFC 3339 UTC, millisecond precision).
    pub recorded_at: String,
}

/// Aggregated view of one session (session_id → ordered calls).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionView {
    pub session_id: String,
    /// Where the session id came from (first call's source).
    pub source: SessionSource,
    /// Ordered calls (by step).
    pub calls: Vec<SessionCall>,
}

// ─────────────────────────────────────────────────────────────────────────────
// SessionStore — in-memory bounded LRU session index
// ─────────────────────────────────────────────────────────────────────────────

/// Default number of sessions retained (RFC-0024 §9: 256 sessions).
pub const DEFAULT_MAX_SESSIONS: usize = 256;
/// Default number of calls retained per session (RFC-0024 §9: 64 calls).
pub const DEFAULT_MAX_CALLS_PER_SESSION: usize = 64;

/// Session index: `session_id → [calls]`, bounded LRU (both in total sessions
/// and calls per session). Thread-safe, designed to be shared via
/// [`init_session_store`]. Not persisted.
pub struct SessionStore {
    inner: Mutex<Inner>,
    max_sessions: usize,
    max_calls_per_session: usize,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Entry>,
    /// LRU order, front = least recently used.
    lru: VecDeque<String>,
}

struct Entry {
    source: SessionSource,
    calls: VecDeque<SessionCall>,
    /// Monotonic per-session step counter (independent of call retention).
    next_step: u64,
}

impl SessionStore {
    /// A store with default bounds (256 sessions × 64 calls/session).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_SESSIONS, DEFAULT_MAX_CALLS_PER_SESSION)
    }

    /// A store with explicit bounds. Panics if either bound is zero.
    pub fn with_capacity(max_sessions: usize, max_calls_per_session: usize) -> Self {
        assert!(
            max_sessions > 0 && max_calls_per_session > 0,
            "SessionStore bounds must be positive"
        );
        Self {
            inner: Mutex::new(Inner::default()),
            max_sessions,
            max_calls_per_session,
        }
    }

    /// Record a call in the session. Returns the index entry for the call.
    ///
    /// Called from the `generate_text` / `stream_text` entry points (before
    /// the call is dispatched), so failures are still part of the session.
    /// `call_id`:调用方已有(RFC-0023 层 A 生成)则复用,保证 session/recording/
    /// trace 三系统同 ID;None 时内部自生成。
    pub fn append(
        &self,
        session_id: &str,
        call_id: Option<&str>,
        source: SessionSource,
    ) -> SessionCall {
        let mut inner = self.inner.lock().unwrap();
        inner.touch(session_id);
        if !inner.sessions.contains_key(session_id) {
            // Bounded sessions: evict the least-recently-used session first.
            if inner.sessions.len() >= self.max_sessions
                && let Some(oldest) = inner.lru.pop_front()
            {
                inner.sessions.remove(&oldest);
            }
            inner.sessions.insert(
                session_id.to_string(),
                Entry {
                    source,
                    calls: VecDeque::new(),
                    next_step: 0,
                },
            );
        }
        let entry = inner.sessions.get_mut(session_id).unwrap();
        // `step` is monotonic per session and independent of call retention:
        // evicting the oldest call must NOT reuse its step (RFC-0024 "0 起").
        // Counter is u64; the wire field is u32, so saturate at u32::MAX
        // (needs ~4.3B calls in one session to be reachable).
        let step = u32::try_from(entry.next_step).unwrap_or(u32::MAX);
        entry.next_step += 1;
        let call = SessionCall {
            call_id: call_id.map(str::to_string).unwrap_or_else(new_call_id),
            step,
            recorded_at: rfc3339_now(),
        };
        // Bounded calls per session: drop the oldest call.
        if entry.calls.len() >= self.max_calls_per_session {
            entry.calls.pop_front();
        }
        entry.calls.push_back(call.clone());
        call
    }

    /// All calls of a session, ordered by step. Empty if unknown.
    pub fn session_calls(&self, session_id: &str) -> Vec<SessionCall> {
        let mut inner = self.inner.lock().unwrap();
        // Touch only known sessions — touching unknown ids would let a
        // query pollute the LRU and break the session capacity bound.
        if inner.sessions.contains_key(session_id) {
            inner.touch(session_id);
        }
        inner
            .sessions
            .get(session_id)
            .map(|e| e.calls.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// All known sessions (unordered), each with its calls.
    pub fn list_sessions(&self) -> Vec<SessionView> {
        let inner = self.inner.lock().unwrap();
        inner
            .sessions
            .iter()
            .map(|(id, e)| SessionView {
                session_id: id.clone(),
                source: e.source,
                calls: e.calls.iter().cloned().collect(),
            })
            .collect()
    }

    /// Drop all sessions (tests / reset).
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.sessions.clear();
        inner.lru.clear();
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Inner {
    /// Mark `id` as most recently used.
    fn touch(&mut self, id: &str) {
        if let Some(pos) = self.lru.iter().position(|k| k == id) {
            self.lru.remove(pos);
        }
        self.lru.push_back(id.to_string());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SessionInferer — opt-in prompt-prefix continuation
// ─────────────────────────────────────────────────────────────────────────────

/// Default number of recent calls remembered for prefix matching.
pub const DEFAULT_INFERER_CAPACITY: usize = 64;

/// Infers session ids by prompt-prefix continuation.
///
/// Opt-in and off by default (explicit ids are the primary mechanism). When
/// enabled, a new call whose prompt fully contains a recent call's prompt as
/// a prefix (message-level, equality allowed — e.g. a retry of the same step)
/// joins that call's session; otherwise a new `auto-` session is started.
/// Only strong prefix continuation is merged — weak similarity is not.
pub struct SessionInferer {
    enabled: bool,
    /// Most recent first: `(prompt, session_id)`.
    recent: VecDeque<(LanguageModelPrompt, String)>,
    capacity: usize,
}

impl SessionInferer {
    /// Inferer with the default capacity, enabled or disabled.
    pub fn new(enabled: bool) -> Self {
        Self::with_capacity(enabled, DEFAULT_INFERER_CAPACITY)
    }

    /// Inferer with an explicit capacity (recent calls remembered). Panics if
    /// the capacity is zero.
    pub fn with_capacity(enabled: bool, capacity: usize) -> Self {
        assert!(capacity > 0, "SessionInferer capacity must be positive");
        Self {
            enabled,
            recent: VecDeque::new(),
            capacity,
        }
    }

    /// Resolve the session id for one call.
    ///
    /// - explicit id: used as-is and remembered (later calls without an id can
    ///   continue this session via prefix matching);
    /// - no id + enabled: strong-prefix match against recent calls, otherwise
    ///   a new `auto-` session id;
    /// - no id + disabled: `None` (isolated request view).
    pub fn resolve(
        &mut self,
        explicit: Option<&str>,
        prompt: &LanguageModelPrompt,
    ) -> Option<(String, SessionSource)> {
        if let Some(id) = explicit {
            self.remember(prompt, id);
            return Some((id.to_string(), SessionSource::Explicit));
        }
        if !self.enabled {
            return None;
        }
        let id = match self.find_by_strong_prefix(prompt) {
            Some(id) => id,
            None => format!("auto-{}", new_call_id()),
        };
        self.remember(prompt, &id);
        Some((id, SessionSource::Inferred))
    }

    /// Strong prefix continuation: the new prompt fully contains a recent
    /// call's prompt as a message-level prefix (equality included). Most
    /// recent first.
    fn find_by_strong_prefix(&self, prompt: &LanguageModelPrompt) -> Option<String> {
        for (old, id) in &self.recent {
            if prompt.len() >= old.len() && prompt[..old.len()] == old[..] {
                return Some(id.clone());
            }
        }
        None
    }

    fn remember(&mut self, prompt: &LanguageModelPrompt, session_id: &str) {
        if self.recent.len() >= self.capacity {
            self.recent.pop_back();
        }
        self.recent
            .push_front((prompt.clone(), session_id.to_string()));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Global registration (explicit, opt-in) + query API
// ─────────────────────────────────────────────────────────────────────────────

static SESSION_STORE: RwLock<Option<Arc<SessionStore>>> = RwLock::new(None);
static SESSION_INFERER: RwLock<Option<Arc<Mutex<SessionInferer>>>> = RwLock::new(None);
static INFER_ENV_CHECKED: Once = Once::new();

/// Register the global session store (replaces any previous one).
///
/// Until this is called, calls are not grouped: no session state exists and
/// the query API returns empty results (RFC-0024 §2.5 opt-in).
pub fn init_session_store(store: Arc<SessionStore>) {
    *SESSION_STORE.write().unwrap() = Some(store);
}

/// The registered store, if any.
pub fn session_store() -> Option<Arc<SessionStore>> {
    SESSION_STORE.read().unwrap().clone()
}

/// Enable the global session inferer (replaces any previous one).
///
/// The inferer is off by default; explicit `session_id` values always win
/// regardless of this setting.
pub fn init_session_infer(enabled: bool) {
    *SESSION_INFERER.write().unwrap() = Some(Arc::new(Mutex::new(SessionInferer::new(enabled))));
}

/// The registered inferer, if any.
pub fn session_inferer() -> Option<Arc<Mutex<SessionInferer>>> {
    SESSION_INFERER.read().unwrap().clone()
}

/// Lazily honor `AIMUX_SESSION_INFER=1` (checked once, like RFC-0014's env
/// auto-init). No-op when the inferer is already registered programmatically
/// (explicit configuration wins) or the env var is not set.
pub fn ensure_inferer_from_env() {
    if INFER_ENV_CHECKED.is_completed() {
        return;
    }
    INFER_ENV_CHECKED.call_once(|| {
        if std::env::var("AIMUX_SESSION_INFER").as_deref() == Ok("1") && session_inferer().is_none()
        {
            init_session_infer(true);
        }
    });
}

/// Resolve the session id for one call: explicit first, inferred fallback.
///
/// Returns `(session_id, source)` or `None` when no explicit id is present
/// and inference is off (or not registered).
pub fn resolve_session_id(
    explicit: Option<&str>,
    prompt: &LanguageModelPrompt,
) -> Option<(String, SessionSource)> {
    ensure_inferer_from_env();
    if let Some(inferer) = session_inferer() {
        return inferer.lock().unwrap().resolve(explicit, prompt);
    }
    explicit.map(|id| (id.to_string(), SessionSource::Explicit))
}

/// Query: all calls of a session (by step). Empty if the session is unknown
/// or no store is registered.
pub fn session_calls(session_id: &str) -> Vec<SessionCall> {
    session_store()
        .map(|s| s.session_calls(session_id))
        .unwrap_or_default()
}

/// Query: all known sessions.
pub fn list_sessions() -> Vec<SessionView> {
    session_store()
        .map(|s| s.list_sessions())
        .unwrap_or_default()
}

// `session_cache_trajectory` is intentionally NOT part of this milestone:
// it aggregates TraceRecord verdicts and depends on the RFC-0015 `Verdict`
// type, so it lands with the cache-probe integration (RFC-0024 §10 P4).

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

static CALL_SEQ: AtomicU64 = AtomicU64::new(0);

/// Process-unique call/session id: `call-{unix_nanos}-{seq}`.
fn new_call_id() -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("call-{ns}-{}", CALL_SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Current time as RFC 3339 UTC with millisecond precision
/// (`2026-08-05T04:52:30.123Z`).
fn rfc3339_now() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs() as i64;
    let millis = d.subsec_millis();
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let hms = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        hms / 3600,
        (hms % 3600) / 60,
        hms % 60
    )
}

/// Days since 1970-01-01 → civil (year, month, day). `z` must be ≥ 0.
/// Howard Hinnant's `civil_from_days` algorithm.
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

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::ContentPart;
    use crate::language_model_message::LanguageModelPromptMessage;
    use crate::message::Role;

    fn msg(text: &str) -> LanguageModelPromptMessage {
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: text.to_string(),
                provider_options: None,
            }],
            provider_options: None,
        }
    }

    fn prompt(messages: Vec<LanguageModelPromptMessage>) -> LanguageModelPrompt {
        messages
    }

    /// `recorded_at` must look like RFC 3339 UTC: `YYYY-MM-DDTHH:MM:SS.mmmZ`.
    fn is_rfc3339(s: &str) -> bool {
        let b = s.as_bytes();
        s.len() >= 24
            && b[4] == b'-'
            && b[7] == b'-'
            && b[10] == b'T'
            && b[13] == b':'
            && b[16] == b':'
            && b[19] == b'.'
            && s.ends_with('Z')
    }

    #[test]
    fn append_increments_step_and_keeps_order() {
        let store = SessionStore::new();
        store.append("s1", None, SessionSource::Explicit);
        store.append("s1", None, SessionSource::Explicit);
        store.append("s1", None, SessionSource::Explicit);

        let calls = store.session_calls("s1");
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].step, 0);
        assert_eq!(calls[1].step, 1);
        assert_eq!(calls[2].step, 2);
        // trace ids are unique
        let mut ids: Vec<_> = calls.iter().map(|c| c.call_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3);
        // recorded_at is RFC 3339 UTC
        for c in &calls {
            assert!(
                is_rfc3339(&c.recorded_at),
                "bad timestamp: {}",
                c.recorded_at
            );
        }
    }

    #[test]
    fn unknown_session_is_empty_and_sessions_list_is_complete() {
        let store = SessionStore::new();
        assert!(store.session_calls("nope").is_empty());
        assert!(store.list_sessions().is_empty());

        store.append("s1", None, SessionSource::Explicit);
        store.append("s2", None, SessionSource::Inferred);

        let views = store.list_sessions();
        assert_eq!(views.len(), 2);
        let s1 = views.iter().find(|v| v.session_id == "s1").unwrap();
        assert_eq!(s1.source, SessionSource::Explicit);
        assert_eq!(s1.calls.len(), 1);
        let s2 = views.iter().find(|v| v.session_id == "s2").unwrap();
        assert_eq!(s2.source, SessionSource::Inferred);
    }

    #[test]
    fn lru_evicts_oldest_session_when_full() {
        let store = SessionStore::with_capacity(2, 8);
        store.append("a", None, SessionSource::Explicit);
        store.append("b", None, SessionSource::Explicit);
        // Touching "a" makes it the most recently used.
        store.session_calls("a");
        store.append("c", None, SessionSource::Explicit);

        assert!(
            store.session_calls("b").is_empty(),
            "oldest session evicted"
        );
        assert_eq!(store.session_calls("a").len(), 1);
        assert_eq!(store.session_calls("c").len(), 1);
    }

    #[test]
    fn calls_per_session_are_bounded() {
        let store = SessionStore::with_capacity(8, 2);
        store.append("s1", None, SessionSource::Explicit);
        store.append("s1", None, SessionSource::Explicit);
        store.append("s1", None, SessionSource::Explicit);

        let calls = store.session_calls("s1");
        assert_eq!(calls.len(), 2, "oldest call dropped");
        assert_eq!(calls[0].step, 1);
        assert_eq!(calls[1].step, 2);
    }

    #[test]
    fn steps_are_monotonic_across_call_eviction() {
        // Regression: step must be monotonic per session even when the oldest
        // call is evicted — eviction must not reuse a step.
        let store = SessionStore::with_capacity(8, 2);
        for _ in 0..4 {
            store.append("s1", None, SessionSource::Explicit);
        }
        let calls = store.session_calls("s1");
        assert_eq!(calls.len(), 2, "only the two newest calls are retained");
        assert_eq!(calls[0].step, 2);
        assert_eq!(calls[1].step, 3);
    }

    #[test]
    fn querying_unknown_sessions_does_not_break_session_capacity() {
        // Regression: touching unknown ids must not pollute the LRU, which
        // would let sessions exceed max_sessions (and let the LRU grow).
        let store = SessionStore::with_capacity(2, 8);
        store.append("a", None, SessionSource::Explicit);
        store.append("b", None, SessionSource::Explicit);

        // Repeatedly query unknown ids — before the fix these polluted the LRU.
        for i in 0..16 {
            assert!(store.session_calls(&format!("unknown-{i}")).is_empty());
        }

        for id in ["c", "d", "e", "f", "g"] {
            store.append(id, None, SessionSource::Explicit);
        }
        assert!(
            store.list_sessions().len() <= 2,
            "session count must stay within max_sessions, got {}",
            store.list_sessions().len()
        );
        assert_eq!(store.session_calls("a").len(), 0, "a was evicted");
        assert_eq!(store.session_calls("g").len(), 1);
    }

    #[test]
    fn clear_drops_all_sessions() {
        let store = SessionStore::new();
        store.append("s1", None, SessionSource::Explicit);
        store.clear();
        assert!(store.list_sessions().is_empty());
    }

    // ── SessionInferer ──────────────────────────────────────────────────────

    #[test]
    fn inferer_disabled_returns_none() {
        let mut inf = SessionInferer::new(false);
        assert!(inf.resolve(None, &prompt(vec![msg("hi")])).is_none());
    }

    #[test]
    fn inferer_explicit_wins_and_is_remembered() {
        let mut inf = SessionInferer::new(true);
        let p1 = prompt(vec![msg("hi")]);
        let (id, source) = inf.resolve(Some("sess-1"), &p1).unwrap();
        assert_eq!(id, "sess-1");
        assert_eq!(source, SessionSource::Explicit);

        // Continuation without an explicit id joins the remembered session.
        let p2 = prompt(vec![msg("hi"), msg("hello")]);
        let (id2, source2) = inf.resolve(None, &p2).unwrap();
        assert_eq!(id2, "sess-1");
        assert_eq!(source2, SessionSource::Inferred);
    }

    #[test]
    fn inferer_strong_prefix_continuation_groups_calls() {
        let mut inf = SessionInferer::new(true);
        let p1 = prompt(vec![msg("u1")]);
        let (id1, source1) = inf.resolve(None, &p1).unwrap();
        assert!(id1.starts_with("auto-"));
        assert_eq!(source1, SessionSource::Inferred);

        // Full-prefix continuation → same session.
        let p2 = prompt(vec![msg("u1"), msg("a1"), msg("u2")]);
        let (id2, _) = inf.resolve(None, &p2).unwrap();
        assert_eq!(id2, id1);

        // Equality (retry of the same step) → same session.
        let (id3, _) = inf.resolve(None, &p1).unwrap();
        assert_eq!(id3, id1);
    }

    #[test]
    fn inferer_distinct_prompts_start_new_sessions() {
        let mut inf = SessionInferer::new(true);
        let (id1, _) = inf
            .resolve(None, &prompt(vec![msg("what is rust")]))
            .unwrap();
        let (id2, _) = inf
            .resolve(None, &prompt(vec![msg("tell me a joke")]))
            .unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn inferer_prefix_is_directional() {
        let mut inf = SessionInferer::new(true);
        let (id1, _) = inf.resolve(None, &prompt(vec![msg("abc")])).unwrap();
        // Message-level prefix continuation: leading messages identical and
        // the new prompt is longer → same session.
        let (id2, _) = inf
            .resolve(None, &prompt(vec![msg("abc"), msg("a1"), msg("u2")]))
            .unwrap();
        assert_eq!(id2, id1);
        // A prompt that diverges on the first message → new session.
        let (id3, _) = inf.resolve(None, &prompt(vec![msg("abd")])).unwrap();
        assert_ne!(id3, id1, "abd is not a prefix continuation");
    }

    #[test]
    fn inferer_chain_continuation_stays_in_session() {
        let mut inf = SessionInferer::new(true);
        let p1 = prompt(vec![msg("common")]);
        let p2 = prompt(vec![msg("common"), msg("b")]);
        let p3 = prompt(vec![msg("common"), msg("b"), msg("c")]);
        let (id1, _) = inf.resolve(None, &p1).unwrap();
        let (id2, _) = inf.resolve(None, &p2).unwrap();
        let (id3, _) = inf.resolve(None, &p3).unwrap();
        assert_eq!(id2, id1);
        assert_eq!(id3, id1);
    }

    #[test]
    fn inferer_capacity_evicts_oldest_prompt() {
        let mut inf = SessionInferer::with_capacity(true, 2);
        let p1 = prompt(vec![msg("one")]);
        let (id1, _) = inf.resolve(None, &p1).unwrap();
        let p2 = prompt(vec![msg("two")]);
        let (id2, _) = inf.resolve(None, &p2).unwrap();
        let p3 = prompt(vec![msg("three")]);
        let (id3, _) = inf.resolve(None, &p3).unwrap();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);

        // p1 was evicted; a new call extending it starts a NEW session.
        let p1_ext = prompt(vec![msg("one"), msg("more")]);
        let (id4, _) = inf.resolve(None, &p1_ext).unwrap();
        assert_ne!(id4, id1, "evicted prompt no longer matches");
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    #[test]
    fn rfc3339_now_is_well_formed() {
        let s = rfc3339_now();
        assert!(is_rfc3339(&s), "bad timestamp: {s}");
    }
}
