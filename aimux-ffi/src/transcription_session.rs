//! Transcription streaming sessions across the C ABI (RFC-0028 Phase 2).
//!
//! Bidirectional streaming cannot reuse the blocking callback pattern of
//! `aimux_stream_text` (the host must push audio while receiving parts), so
//! sessions are a dedicated wire shape: a session handle plus
//! push / input-done / next-part operations.
//!
//! ```text
//! session_new ──► spawn tokio task ──► TranscriptionModel::do_stream(audio_rx)
//!                                             │ parts flow out
//! push_audio  ──► bounded mpsc sender         ▼
//! input_done  ──► drop sender (= end of audio)
//! next_part   ◄── mpsc receiver + recv_timeout
//! session_drop ─► deregister → abort → bounded join
//! ```
//!
//! Deadlock notes (RFC-0028 §4.2):
//! - `push_audio` BLOCKS while the audio channel is full (capacity 64) —
//!   WebSocket backpressure propagates back to the host's capture loop.
//! - The driver task's forward loop selects on the abort token, so a full
//!   parts channel (host stopped consuming) cannot wedge `session_drop`'s
//!   join.
//! - `session_drop` removes the handle from the registry BEFORE joining, so
//!   the global registry mutex is never held across the join.

use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};

use aimux_core::error::AiMuxError;
use aimux_core::shared::AbortSignal;
use aimux_core::transcription_model::{
    AudioChunk, InputAudioFormat, TranscriptionModel, TranscriptionStreamOptions,
    TranscriptionStreamPart,
};

use crate::runtime;

/// Capacity of the audio-in channel (chunks). Bounds host memory when the
/// provider is slower than the capture loop (RFC-0028 Open Question 1).
const AUDIO_CHANNEL_CAPACITY: usize = 64;
/// Capacity of the parts-out channel.
const PARTS_CHANNEL_CAPACITY: usize = 256;
/// Upper bound for `session_drop`'s join. The abort token should end the
/// driver promptly; this is a belt-and-suspenders guard (detaches on expiry).
const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

/// FFI-side transcription stream options (parsed from `session_new`'s JSON).
#[derive(Default)]
pub struct SessionOptions {
    pub input_audio_format: Option<InputAudioFormat>,
    pub provider_options: Option<std::collections::HashMap<String, serde_json::Value>>,
    pub headers: Option<std::collections::HashMap<String, String>>,
    pub include_raw_chunks: bool,
    pub timeout: Option<aimux_core::options::TimeoutConfiguration>,
}

/// A live transcription streaming session (FFI-side).
pub struct TranscriptionFfiSession {
    /// Audio-in sender. `None` once `input_done` was called (or construction
    /// failed). Mutex<Option<…>> so `push_audio` after `input_done` errors
    /// rather than silently buffering.
    audio_tx: std::sync::Mutex<Option<mpsc::Sender<AudioChunk>>>,
    /// Parts-out receiver. Tokio mutex: held across the `recv` await.
    parts_rx: tokio::sync::Mutex<mpsc::Receiver<Result<TranscriptionStreamPart, AiMuxError>>>,
    /// Cancels the driver task (fired by `session_drop`).
    token: AbortSignal,
    /// Driver join handle (taken by `session_drop`).
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TranscriptionFfiSession {
    /// Spawn the driver for `model.do_stream` and return the session.
    ///
    /// `user_abort` (optional) and the internal drop token are both linked
    /// into the effective abort signal passed to the model: either firing
    /// aborts the call.
    pub fn spawn(
        model: Arc<dyn TranscriptionModel>,
        opts: SessionOptions,
        user_abort: Option<AbortSignal>,
    ) -> Arc<Self> {
        let (audio_tx, audio_rx) = mpsc::channel::<AudioChunk>(AUDIO_CHANNEL_CAPACITY);
        let (mut parts_tx, parts_rx) =
            mpsc::channel::<Result<TranscriptionStreamPart, AiMuxError>>(PARTS_CHANNEL_CAPACITY);

        let token = AbortSignal::new();
        // Effective abort = user abort OR drop token. `AbortSignal` has no
        // OR-composition, so link both by forwarding into one signal.
        let effective = AbortSignal::new();
        for source in std::iter::once(token.clone()).chain(user_abort) {
            let linked = effective.clone();
            runtime().spawn(async move {
                source.cancelled().await;
                linked.abort();
            });
        }

        let task = runtime().spawn(async move {
            let options = TranscriptionStreamOptions {
                audio: Box::pin(audio_rx),
                input_audio_format: opts.input_audio_format.unwrap_or(InputAudioFormat {
                    format_type: "audio/pcm".to_string(),
                    rate: None,
                }),
                provider_options: opts.provider_options,
                abort_signal: Some(effective.clone()),
                headers: opts.headers,
                include_raw_chunks: opts.include_raw_chunks,
                timeout: opts.timeout,
            };
            let result = model.do_stream(options).await;
            match result {
                Ok(stream_result) => {
                    let mut stream = stream_result.stream;
                    // Forward loop. Delivery is immediate when the channel has
                    // capacity (so the terminal Err(Aborted) part from the
                    // model is never preempted); the abort token only unblocks
                    // a FULL channel (host stopped consuming), which would
                    // otherwise wedge `session_drop`'s join (RFC-0028 §4.2 S5).
                    while let Some(part) = stream.next().await {
                        let mut part = part;
                        loop {
                            match parts_tx.try_send(part) {
                                Ok(()) => break,
                                Err(send_err) => {
                                    if send_err.is_disconnected() {
                                        // Receiver dropped (session dropped).
                                        return;
                                    }
                                    // Channel full: wait for capacity or
                                    // abort.
                                    // NOTE: if abort fires while full, the model's terminal
                                    // Err(Aborted) part may be dropped — a host that stopped
                                    // consuming then sees "ended normally" rather than Aborted.
                                    // Accepted trade-off (bounded memory beats exact terminal
                                    // delivery under a 256-part backlog). (Sink::flush on futures mpsc
                                    // maps Disconnected to Ok — never rely on
                                    // it for receiver-loss detection.)
                                    part = send_err.into_inner();
                                    tokio::select! {
                                        _ = effective.cancelled() => return,
                                        res = parts_tx.flush() => {
                                            let _ = res;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    // Connect/establishment failure: surface as the first
                    // (and only) channel item so next_part reports it. Select
                    // on abort so a full channel can't stall the drop join.
                    let mut err = Some(e);
                    loop {
                        let e = err.take().expect("err only None after successful send");
                        match parts_tx.try_send(Err(e)) {
                            Ok(()) => break,
                            Err(send_err) => {
                                if send_err.is_disconnected() {
                                    return;
                                }
                                // into_inner returns the unsent Result — the
                                // Err payload is the original error.
                                err = send_err.into_inner().err();
                                tokio::select! {
                                    _ = effective.cancelled() => return,
                                    res = parts_tx.flush() => {
                                        let _ = res;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        Arc::new(Self {
            audio_tx: std::sync::Mutex::new(Some(audio_tx)),
            parts_rx: tokio::sync::Mutex::new(parts_rx),
            token,
            task: std::sync::Mutex::new(Some(task)),
        })
    }

    /// Push one audio chunk. **Blocking**: waits while the bounded channel is
    /// full (backpressure propagation). Fails if the session already ended
    /// (task finished / channel closed) or `input_done` was called.
    pub async fn push_audio(&self, bytes: Vec<u8>) -> Result<(), AiMuxError> {
        // Clone the sender and drop the guard BEFORE awaiting (await-holding-
        // lock). Cloning is benign: `input_done` drops the stored sender; an
        // in-flight push finishes sending on its clone and the channel then
        // closes with it — no chunk is lost.
        let mut tx = {
            let guard = self.audio_tx.lock().expect("session audio mutex poisoned");
            match guard.as_ref() {
                None => {
                    return Err(AiMuxError::Other(
                        "transcription session audio input already finished".into(),
                    ));
                }
                Some(tx) => tx.clone(),
            }
        };
        tx.send(AudioChunk::Binary(bytes))
            .await
            .map_err(|_| AiMuxError::Other("transcription session ended".into()))
    }

    /// Signal end-of-audio (the audio stream yields `None`). Idempotent.
    pub fn input_done(&self) {
        // Dropping the sender closes the channel: the receiver stream ends.
        self.audio_tx
            .lock()
            .expect("session audio mutex poisoned")
            .take();
    }

    /// Receive the next part.
    ///
    /// - `Ok(Some(part))` — a part arrived.
    /// - `Ok(None)` — the part channel closed: stream ended normally (the
    ///   driver exits after the forward loop; a terminal stream `Err` is
    ///   delivered as `Err(..)` before closing).
    /// - `Err(Timeout)` — no part within `timeout`.
    pub async fn next_part(
        &self,
        timeout: Option<Duration>,
    ) -> Result<Option<Result<TranscriptionStreamPart, AiMuxError>>, AiMuxError> {
        let mut rx = self.parts_rx.lock().await;
        let recv = rx.next();
        match timeout {
            None => Ok(recv.await),
            Some(d) => match tokio::time::timeout(d, recv).await {
                Ok(part) => Ok(part),
                Err(_) => Err(AiMuxError::Timeout(
                    "no transcription part within timeout".into(),
                )),
            },
        }
    }

    /// Terminate: fire the abort token and join the driver (bounded).
    /// The caller must have removed the handle from the registry already —
    /// this must run without the registry mutex held.
    pub fn terminate(&self) {
        self.input_done();
        self.token.abort();
        let task = self
            .task
            .lock()
            .expect("session task mutex poisoned")
            .take();
        if let Some(task) = task {
            // Bounded join via ffi_block_on (re-entrancy guard): on expiry or
            // on a re-entrant call (drop from within an aimux callback — not
            // allowed to block_on) the task is detached; the abort token and
            // closed channels end it shortly after.
            // The timeout future is constructed inside the async block:
            // creating a tokio Sleep needs a reactor, which only exists once
            // block_on polls it.
            let joined =
                crate::ffi_block_on(async { tokio::time::timeout(JOIN_TIMEOUT, task).await });
            if joined.is_err() {
                tracing::warn!(
                    "aimux-ffi: transcription session join timed out or was re-entrant; detaching"
                );
            }
        }
    }
}
