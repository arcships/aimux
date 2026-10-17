use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::envelope::{BindingError, OpsError, ProtocolError};

/// Global scope is 0; live resource identifiers are never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HandleId(pub u64);

impl Serialize for HandleId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl std::str::FromStr for HandleId {
    type Err = String;

    /// The one handle-id rule: a decimal string that fits u64.
    fn from_str(value: &str) -> Result<Self, String> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("expected a decimal handle string".into());
        }
        value.parse().map(Self).map_err(|error| format!("{error}"))
    }
}

impl<'de> Deserialize<'de> for HandleId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Owner {
    CProcess,
    Node(u64),
    Python(u64),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize)]
pub enum HandleKind {
    Provider,
    Registry,
    Language,
    Embedding,
    Image,
    Speech,
    Transcription,
    Reranking,
    Video,
    Search,
    Decision,
    Files,
    Operation,
    Abort,
    Fetch,
    WebSocket,
    Download,
    CredentialResolver,
    Middleware,
    TraceStore,
    Recorder,
    SessionStore,
    ReplaySession,
}

impl HandleKind {
    pub const ALL: &'static [Self] = &[
        Self::Provider,
        Self::Registry,
        Self::Language,
        Self::Embedding,
        Self::Image,
        Self::Speech,
        Self::Transcription,
        Self::Reranking,
        Self::Video,
        Self::Search,
        Self::Decision,
        Self::Files,
        Self::Operation,
        Self::Abort,
        Self::Fetch,
        Self::WebSocket,
        Self::Download,
        Self::CredentialResolver,
        Self::Middleware,
        Self::TraceStore,
        Self::Recorder,
        Self::SessionStore,
        Self::ReplaySession,
    ];
}

/// Strong references are cloned under the registry lock and used outside it.
#[derive(Clone)]
pub enum Entry {
    Provider(Arc<dyn aimux_core::provider::Provider>),
    Registry(Arc<aimux_core::provider_registry::ProviderRegistry>),
    Language(Arc<dyn aimux_core::language_model::LanguageModel>),
    Embedding(Arc<dyn aimux_core::embedding_model::EmbeddingModel>),
    Image(Arc<dyn aimux_core::image_model::ImageModel>),
    Speech(Arc<dyn aimux_core::speech_model::SpeechModel>),
    Transcription(Arc<dyn aimux_core::transcription_model::TranscriptionModel>),
    Reranking(Arc<dyn aimux_core::reranking_model::RerankingModel>),
    Video(Arc<dyn aimux_core::video_model::VideoModel>),
    Search(Arc<dyn aimux_core::search_model::SearchModel>),
    Decision(Arc<dyn aimux_core::decision_model::DecisionModel>),
    Files(Arc<dyn aimux_core::files_model::Files>),
    Operation(Arc<crate::operation::Operation>),
    Abort(Arc<aimux_core::AbortSignal>),
    Fetch(Arc<dyn aimux_provider_utils::Fetch>),
    WebSocket(Arc<dyn aimux_provider_utils::ws::WsConnector>),
    Download(Arc<aimux_core::download::DownloadFunction>),
    CredentialResolver(Arc<aimux_provider_utils::Resolvable<String>>),
    Middleware(Arc<Middleware>),
    TraceStore(Arc<aimux_core::trace::RingTraceStore>),
    Recorder(Arc<dyn aimux_core::recording::Recorder>),
    SessionStore(Arc<aimux_core::session::SessionStore>),
    ReplaySession(Arc<aimux_core::replay::MockReplayModel>),
}

impl Entry {
    #[must_use]
    pub fn kind(&self) -> HandleKind {
        match self {
            Self::Provider(_) => HandleKind::Provider,
            Self::Registry(_) => HandleKind::Registry,
            Self::Language(_) => HandleKind::Language,
            Self::Embedding(_) => HandleKind::Embedding,
            Self::Image(_) => HandleKind::Image,
            Self::Speech(_) => HandleKind::Speech,
            Self::Transcription(_) => HandleKind::Transcription,
            Self::Reranking(_) => HandleKind::Reranking,
            Self::Video(_) => HandleKind::Video,
            Self::Search(_) => HandleKind::Search,
            Self::Decision(_) => HandleKind::Decision,
            Self::Files(_) => HandleKind::Files,
            Self::Operation(_) => HandleKind::Operation,
            Self::Abort(_) => HandleKind::Abort,
            Self::Fetch(_) => HandleKind::Fetch,
            Self::WebSocket(_) => HandleKind::WebSocket,
            Self::Download(_) => HandleKind::Download,
            Self::CredentialResolver(_) => HandleKind::CredentialResolver,
            Self::Middleware(_) => HandleKind::Middleware,
            Self::TraceStore(_) => HandleKind::TraceStore,
            Self::Recorder(_) => HandleKind::Recorder,
            Self::SessionStore(_) => HandleKind::SessionStore,
            Self::ReplaySession(_) => HandleKind::ReplaySession,
        }
    }
}

#[derive(Default)]
pub struct HandleRegistry {
    entries: Mutex<HashMap<u64, (Owner, Entry)>>,
}

// Even separate registry instances allocate from the same process-wide sequence.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl Owner {
    /// Allocate an owner token in trusted adapter code, never from user input.
    /// # Errors
    /// Returns HandleExhausted when the process owner allocator is exhausted.
    pub fn new_node() -> Result<Self, OpsError> {
        allocate(&NEXT_OWNER).map(Self::Node)
    }
    /// Allocate a Python interpreter owner token.
    /// # Errors
    /// Returns HandleExhausted when the process owner allocator is exhausted.
    pub fn new_python() -> Result<Self, OpsError> {
        allocate(&NEXT_OWNER).map(Self::Python)
    }
}
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

impl HandleRegistry {
    /// Remove all owned entries before signalling operations or dropping resources.
    pub fn release_owner(&self, owner: Owner) -> usize {
        let removed = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let ids: Vec<_> = entries
                .iter()
                .filter_map(|(&id, (found, _))| (*found == owner).then_some(id))
                .collect();
            ids.into_iter()
                .filter_map(|id| entries.remove(&id).map(|(_, entry)| entry))
                .collect::<Vec<_>>()
        };
        let count = removed.len();
        for entry in &removed {
            if let Entry::Operation(operation) = entry {
                operation.consumer_close();
            }
        }
        drop(removed);
        count
    }

    #[must_use]
    pub fn global() -> &'static Self {
        static REGISTRY: OnceLock<HandleRegistry> = OnceLock::new();
        REGISTRY.get_or_init(Self::default)
    }

    /// Insert a resource, failing permanently rather than wrapping the allocator.
    ///
    /// # Errors
    /// Returns HandleExhausted on overflow.
    pub fn insert(&self, owner: Owner, entry: Entry) -> Result<HandleId, OpsError> {
        let id = allocate(&NEXT_ID)?;
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, (owner, entry));
        Ok(HandleId(id))
    }

    /// Clone a live resource after checking its owner and kind.
    ///
    /// # Errors
    /// Returns InvalidHandle without revealing a foreign owner's resource kind.
    pub fn get(
        &self,
        owner: Owner,
        id: HandleId,
        allowed: &[HandleKind],
    ) -> Result<Entry, OpsError> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = entries
            .get(&id.0)
            .filter(|(entry_owner, _)| *entry_owner == owner);
        match entry {
            Some((_, entry)) if allowed.contains(&entry.kind()) => Ok(entry.clone()),
            entry => Err(OpsError::Binding(BindingError::InvalidHandle {
                expected: allowed.to_vec(),
                actual: entry.map(|(_, entry)| entry.kind()),
            })),
        }
    }

    /// Remove before performing any cancellation, I/O or resource cleanup.
    #[must_use]
    pub fn remove(&self, owner: Owner, id: HandleId) -> Option<Entry> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if entries
            .get(&id.0)
            .is_some_and(|(entry_owner, _)| *entry_owner == owner)
        {
            entries.remove(&id.0).map(|(_, entry)| entry)
        } else {
            None
        }
    }
}

/// Take the next nonzero id from a counter that starts at 1, failing once it is exhausted.
///
/// # Errors
/// Returns HandleExhausted after u64::MAX has been allocated.
pub fn allocate(next: &AtomicU64) -> Result<u64, OpsError> {
    // 0 is an exhausted sentinel, reached after allocating u64::MAX once.
    let mut id = next.load(Ordering::Relaxed);
    loop {
        if id == 0 {
            return Err(OpsError::Protocol(ProtocolError::HandleExhausted));
        }
        let following = id.checked_add(1).unwrap_or(0);
        match next.compare_exchange_weak(id, following, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(allocated) => return Ok(allocated),
            Err(current) => id = current,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_kind_and_drop_preserve_strong_reference() {
        let registry = HandleRegistry::default();
        let signal = Arc::new(aimux_core::AbortSignal::new());
        let id = registry
            .insert(Owner::Node(1), Entry::Abort(signal.clone()))
            .unwrap();
        let retained = registry
            .get(Owner::Node(1), id, &[HandleKind::Abort])
            .unwrap();
        assert!(matches!(
            registry.get(Owner::Node(2), id, &[HandleKind::Abort]),
            Err(OpsError::Binding(BindingError::InvalidHandle {
                actual: None,
                ..
            }))
        ));
        assert!(
            matches!(registry.get(Owner::Node(1), id, &[HandleKind::Language]),
            Err(OpsError::Binding(BindingError::InvalidHandle { expected, actual: Some(HandleKind::Abort) }))
            if expected == vec![HandleKind::Language])
        );
        assert!(registry.remove(Owner::Python(1), id).is_none());
        assert!(registry.remove(Owner::Node(1), id).is_some());
        assert!(registry.remove(Owner::Node(1), id).is_none());
        assert!(
            registry
                .get(Owner::Node(1), id, &[HandleKind::Abort])
                .is_err()
        );
        let Entry::Abort(retained) = retained else {
            unreachable!()
        };
        retained.abort();
        assert!(signal.is_aborted());
        let next = registry
            .insert(Owner::CProcess, Entry::Abort(signal))
            .unwrap();
        assert!(next.0 > id.0);
    }

    #[test]
    fn global_and_unknown_handles_are_not_resources() {
        let registry = HandleRegistry::default();
        for id in [HandleId(0), HandleId(u64::MAX)] {
            assert!(registry.get(Owner::CProcess, id, HandleKind::ALL).is_err());
            assert!(registry.remove(Owner::CProcess, id).is_none());
        }
    }

    #[test]
    fn allocator_exhaustion_does_not_wrap() {
        let next = AtomicU64::new(u64::MAX);
        assert_eq!(allocate(&next).unwrap(), u64::MAX);
        assert!(matches!(
            allocate(&next),
            Err(OpsError::Protocol(ProtocolError::HandleExhausted))
        ));
        assert!(allocate(&next).is_err());
    }

    #[test]
    fn independent_registries_share_unique_process_ids_under_concurrent_insertion() {
        let workers: Vec<_> = (0..4)
            .map(|owner| {
                std::thread::spawn(move || {
                    let registry = HandleRegistry::default();
                    let ids: Vec<_> = (0..16)
                        .map(|_| {
                            registry
                                .insert(
                                    Owner::Node(owner),
                                    Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
                                )
                                .unwrap()
                                .0
                        })
                        .collect();
                    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
                    ids
                })
            })
            .collect();
        let mut ids: Vec<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        assert!(ids.iter().all(|id| *id != 0));
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 64);
    }

    #[test]
    fn handles_are_decimal_strings_including_above_js_integer_range() {
        let id = HandleId(u64::MAX);
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            "\"18446744073709551615\""
        );
        assert_eq!(
            serde_json::from_str::<HandleId>("\"18446744073709551615\"").unwrap(),
            id
        );
        for invalid in [
            "7",
            "\"-1\"",
            "\"+1\"",
            "\"1e3\"",
            "\"\"",
            "\"18446744073709551616\"",
        ] {
            assert!(serde_json::from_str::<HandleId>(invalid).is_err());
        }
    }
}

/// Built-in modality-specific middleware, with no host callback bridge.
#[derive(Clone)]
pub enum Middleware {
    Language(Arc<dyn aimux_core::language_model_middleware::LanguageModelMiddleware>),
    Image(Arc<dyn aimux_core::image_model_middleware::ImageModelMiddleware>),
}
