//! C adapter for the synchronous and incremental async primitives.

use std::ffi::{CStr, CString, c_char, c_void};
use std::future::{Future, poll_fn};
use std::panic::AssertUnwindSafe;
use std::ptr;
use std::sync::OnceLock;
use std::task::Poll;
use std::time::Duration;

use aimux_ops::dispatch::{self, Ctx, WireParams, WirePollResult};
use aimux_ops::envelope::{BindingError, ErrorEnvelope, OpsError, ProtocolError};
#[cfg(test)]
use aimux_ops::frame::StreamFrame;
use aimux_ops::frame::{Outcome, WireFrame};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use futures::{FutureExt, Stream, StreamExt};
#[cfg(test)]
use serde_json::Value;
use tokio::runtime::{Handle, Runtime};

/// Owned error projection, captured once at the boundary.
#[allow(non_camel_case_types)]
pub struct aimux_error_t {
    envelope: ErrorEnvelope,
}

type Result<T> = std::result::Result<T, OpsError>;
type EventCallback = extern "C" fn(*const c_char, *mut c_void);
type CallCallback = extern "C" fn(*mut c_char, *mut aimux_error_t, *mut c_void);
type EventBatchCallback = extern "C" fn(*const *const c_char, usize, *mut c_void);

const BATCH_FRAMES: usize = 64;

struct CallbackContext(*mut c_void);

// SAFETY: Rust only passes the opaque pointer back to the callback. The caller
// guarantees that its context can be used on another thread until completion.
unsafe impl Send for CallbackContext {}

impl CallbackContext {
    fn get(&self) -> *mut c_void {
        self.0
    }
}

fn null_pointer(argument: &'static str) -> OpsError {
    OpsError::Binding(BindingError::NullPointer { argument })
}

fn serialization(error: impl std::error::Error + 'static) -> OpsError {
    OpsError::Binding(BindingError::result_serialization(&error))
}

fn finish(result: Result<()>) -> *mut aimux_error_t {
    match result {
        Ok(()) => ptr::null_mut(),
        Err(error) => Box::into_raw(Box::new(aimux_error_t {
            envelope: error.envelope(),
        })),
    }
}

fn json_string(value: &impl serde::Serialize) -> Result<CString> {
    CString::new(serde_json::to_string(value).map_err(serialization)?).map_err(serialization)
}

// The caller supplies live, NUL-terminated strings for the duration of the call.
unsafe fn text<'a>(value: *const c_char, argument: &'static str) -> Result<&'a str> {
    if value.is_null() {
        return Err(null_pointer(argument));
    }
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|_| OpsError::Binding(BindingError::InvalidUtf8 { argument }))
}

unsafe fn clear_string(out: *mut *mut c_char, argument: &'static str) -> Result<()> {
    if out.is_null() {
        return Err(null_pointer(argument));
    }
    unsafe { *out = ptr::null_mut() };
    Ok(())
}

fn context(abort: u64) -> Result<Ctx> {
    let mut ctx = Ctx {
        owner: Owner::CProcess,
        abort: None,
    };
    if abort != 0 {
        let entry =
            HandleRegistry::global().get(ctx.owner, HandleId(abort), &[HandleKind::Abort])?;
        if let Entry::Abort(signal) = entry {
            ctx.abort = Some((*signal).clone());
        }
    }
    Ok(ctx)
}

static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();

fn runtime() -> Result<&'static Runtime> {
    match RUNTIME.get_or_init(|| Runtime::new().map_err(|error| error.to_string())) {
        Ok(runtime) => Ok(runtime),
        Err(message) => Err(OpsError::Protocol(ProtocolError::InvalidState(
            message.clone(),
        ))),
    }
}

fn block_on<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    if Handle::try_current().is_ok() {
        return Err(OpsError::Binding(BindingError::ReentrantCall));
    }
    runtime()?.block_on(future)
}

/// ABI major and minor encoded as `(major << 16) | minor`.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_abi_version() -> u32 {
    aimux_ops::op::ABI_VERSION
}

/// Return an owned manifest string; clear the output before doing any work.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_manifest_json(out_json: *mut *mut c_char) -> *mut aimux_error_t {
    finish((|| {
        unsafe { clear_string(out_json, "out_json")? };
        let json = CString::new(aimux_ops::manifest::manifest_json()).map_err(serialization)?;
        unsafe { *out_json = json.into_raw() };
        Ok(())
    })())
}

/// Dispatch one call. All pointers must remain valid until this function returns.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_call(
    handle: u64,
    op: *const c_char,
    params_json: *const c_char,
    abort: u64,
    out_json: *mut *mut c_char,
) -> *mut aimux_error_t {
    finish((|| {
        unsafe { clear_string(out_json, "out_json")? };
        let op = unsafe { text(op, "op")? };
        let params = WireParams::new(
            unsafe { text(params_json, "params_json")? }.as_bytes(),
            "params_json",
        );
        let result = block_on(async {
            let ctx = context(abort)?;
            dispatch::call_wire(ctx, HandleId(handle), op, params).await
        })?;
        let json = CString::new(result).map_err(serialization)?;
        unsafe { *out_json = json.into_raw() };
        Ok(())
    })())
}

fn invalid_state(message: &str) -> OpsError {
    OpsError::Protocol(ProtocolError::InvalidState(message.into()))
}

/// Submit a call. Accepted submissions transfer one owned result or error to the callback.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_call_async(
    handle: u64,
    op: *const c_char,
    params_json: *const c_char,
    abort: u64,
    on_complete: Option<CallCallback>,
    user_data: *mut c_void,
) -> *mut aimux_error_t {
    finish((|| {
        let callback = on_complete.ok_or_else(|| null_pointer("on_complete"))?;
        let op = unsafe { text(op, "op")? };
        let params = unsafe { text(params_json, "params_json")? };
        let ctx = context(abort)?;
        let op = op.to_owned();
        let params = params.to_owned();
        let user_data = CallbackContext(user_data);
        runtime()?.spawn(async move {
            let params = WireParams::new(params.as_bytes(), "params_json");
            let result = AssertUnwindSafe(dispatch::call_wire(ctx, HandleId(handle), &op, params))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(invalid_state("async call panicked")))
                .and_then(|json| CString::new(json).map_err(serialization));
            match result {
                Ok(json) => callback(json.into_raw(), ptr::null_mut(), user_data.get()),
                Err(error) => callback(ptr::null_mut(), finish(Err(error)), user_data.get()),
            }
        });
        Ok(())
    })())
}

async fn deliver_frames(
    frames: impl Stream<Item = WireFrame>,
    callback: EventCallback,
    context: *mut c_void,
) -> Result<()> {
    futures::pin_mut!(frames);
    while let Some(frame) = frames.next().await {
        match frame {
            WireFrame::Event { json, .. } => {
                let json = CString::new(json).map_err(serialization)?;
                callback(json.as_ptr(), context);
            }
            WireFrame::End {
                seq,
                outcome,
                error,
            } => {
                let json = CString::new(WireFrame::encode_end(seq, outcome, error)?)
                    .map_err(serialization)?;
                callback(json.as_ptr(), context);
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Synchronously deliver frames. No callback runs after this function returns.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream(
    handle: u64,
    op: *const c_char,
    params_json: *const c_char,
    abort: u64,
    on_event: Option<EventCallback>,
    context_ptr: *mut c_void,
) -> *mut aimux_error_t {
    finish((|| {
        let callback = on_event.ok_or_else(|| null_pointer("on_event"))?;
        let op = unsafe { text(op, "op")? };
        let params = WireParams::new(
            unsafe { text(params_json, "params_json")? }.as_bytes(),
            "params_json",
        );
        block_on(async {
            let ctx = context(abort)?;
            let frames = dispatch::stream_wire(ctx, HandleId(handle), op, params)?;
            deliver_frames(frames.into_wire(), callback, context_ptr).await
        })
    })())
}

async fn deliver_frame_batches(
    frames: impl Stream<Item = WireFrame> + Send,
    callback: EventBatchCallback,
    user_data: CallbackContext,
) {
    let frames = AssertUnwindSafe(frames).catch_unwind();
    futures::pin_mut!(frames);
    let mut batch = Vec::with_capacity(BATCH_FRAMES);
    let mut next_seq = 0;
    let terminal = loop {
        // Wait for one frame and poll only immediately ready frames after it,
        // with the task's real waker. Keep the terminal out of the data batch.
        let mut frame = frames.next().await;
        let mut batch_bytes = 0;
        let terminal = loop {
            match frame {
                Some(Ok(WireFrame::Event { seq, json })) => {
                    next_seq = seq.saturating_add(1);
                    batch_bytes += json.len();
                    batch.push(CString::new(json).expect("wire JSON contains no raw NUL"));
                }
                Some(Ok(end @ WireFrame::End { .. })) => break Some(end),
                other => {
                    let error = invalid_state(if other.is_none() {
                        "accepted stream ended without an End frame"
                    } else {
                        "async stream panicked"
                    });
                    break Some(WireFrame::End {
                        seq: next_seq.to_string(),
                        outcome: Outcome::Error,
                        error: Some(error.envelope()),
                    });
                }
            }
            // Bound retained native bytes for streams of large events too.
            if batch.len() == BATCH_FRAMES || batch_bytes >= aimux_ops::limits::ITEM_BYTES {
                break None;
            }
            match poll_fn(|cx| Poll::Ready(frames.poll_next_unpin(cx))).await {
                Poll::Ready(next) => frame = next,
                Poll::Pending => break None,
            }
        };
        if !batch.is_empty() {
            // A foreign consumer may apply backpressure. Let Tokio replace
            // this worker while the callback is running.
            tokio::task::block_in_place(|| {
                let mut pointers = [ptr::null(); BATCH_FRAMES];
                for (pointer, json) in pointers.iter_mut().zip(&batch) {
                    *pointer = json.as_ptr();
                }
                callback(pointers.as_ptr(), batch.len(), user_data.get());
            });
            batch.clear();
        }
        if let Some(terminal) = terminal {
            break terminal;
        }
        tokio::task::consume_budget().await;
    };
    let WireFrame::End {
        seq,
        outcome,
        error,
    } = terminal
    else {
        unreachable!("only End frames can terminate delivery");
    };
    let json = WireFrame::encode_end(seq, outcome, error).expect("bounded End frame serializes");
    let json = CString::new(json).expect("wire JSON contains no raw NUL");
    tokio::task::block_in_place(|| callback(&json.as_ptr(), 1, user_data.get()));
}

/// Accept a stream synchronously, then deliver borrowed batches on the shared runtime.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_stream_async(
    handle: u64,
    op: *const c_char,
    params_json: *const c_char,
    abort: u64,
    on_batch: Option<EventBatchCallback>,
    user_data: *mut c_void,
) -> *mut aimux_error_t {
    finish((|| {
        let callback = on_batch.ok_or_else(|| null_pointer("on_batch"))?;
        let op = unsafe { text(op, "op")? };
        let params = WireParams::new(
            unsafe { text(params_json, "params_json")? }.as_bytes(),
            "params_json",
        );
        let ctx = context(abort)?;
        let runtime = runtime()?;
        // Entering supplies the runtime context for admission without waiting
        // on any task. Dispatch returns an owned stream before we spawn it.
        let frames = {
            let _entered = runtime.enter();
            dispatch::stream_wire(ctx, HandleId(handle), op, params)?.into_wire()
        };
        runtime.spawn(deliver_frame_batches(
            frames,
            callback,
            CallbackContext(user_data),
        ));
        Ok(())
    })())
}

/// Admit bytes before copying, using shared queue limits. NULL+0 is legal.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_operation_push(
    operation: u64,
    data: *const u8,
    length: usize,
) -> *mut aimux_error_t {
    finish((|| {
        if data.is_null() && length != 0 {
            return Err(null_pointer("data"));
        }
        // Reject oversize chunks before forming a slice or allocating a copy.
        if length > aimux_ops::limits::ITEM_BYTES || (data as usize).checked_add(length).is_none() {
            return Err(OpsError::Protocol(ProtocolError::ResourceLimit(
                "audio chunk exceeds 1 MiB".into(),
            )));
        }
        block_on(async {
            let bytes = if length == 0 {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(data, length) }
            };
            dispatch::operation_push(&context(0)?, HandleId(operation), bytes)
        })
    })())
}

/// Poll one part. State: PART=1, ENDED=2, TIMEOUT=3; errors leave both outputs zeroed.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_operation_next_part(
    operation: u64,
    timeout_ms: i64,
    out_part: *mut *mut c_char,
    out_state: *mut i32,
) -> *mut aimux_error_t {
    // Clear every supplied output, even when another output pointer is NULL.
    if !out_part.is_null() {
        unsafe { *out_part = ptr::null_mut() };
    }
    if !out_state.is_null() {
        unsafe { *out_state = 0 };
    }
    finish((|| {
        if out_part.is_null() {
            return Err(null_pointer("out_part"));
        }
        if out_state.is_null() {
            return Err(null_pointer("out_state"));
        }
        let timeout = u64::try_from(timeout_ms).map_err(|_| {
            OpsError::Protocol(ProtocolError::invalid_params(
                "timeout_ms must be nonnegative",
            ))
        })?;
        let result = block_on(async {
            dispatch::operation_next_json(
                &context(0)?,
                HandleId(operation),
                Duration::from_millis(timeout),
            )
            .await
        })?;
        let (part, state) = match result {
            WirePollResult::Part(part) => {
                (CString::new(part).map_err(serialization)?.into_raw(), 1)
            }
            WirePollResult::Ended => (ptr::null_mut(), 2),
            WirePollResult::Timeout => (ptr::null_mut(), 3),
        };
        unsafe {
            *out_part = part;
            *out_state = state;
        }
        Ok(())
    })())
}

/// Signal cancellation without entering a runtime; valid inside callbacks.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_abort(abort: u64) {
    if let Ok(Entry::Abort(signal)) =
        HandleRegistry::global().get(Owner::CProcess, HandleId(abort), &[HandleKind::Abort])
    {
        signal.abort();
    }
}

/// Release a C-owned handle. Zero and unknown handles are harmless.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_drop(handle: u64) {
    let removed = HandleRegistry::global().remove(Owner::CProcess, HandleId(handle));
    if let Some(Entry::Operation(operation)) = &removed {
        operation.consumer_close();
    }
    drop(removed);
}

/// Enqueue handle cleanup without waiting on the finalizer thread.
/// The token encodes the handle in its pointer value and is never dereferenced.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_handle_finalizer(token: *mut c_void) {
    let handle = token as usize as u64;
    // Valid C handles are created through block_on, which initializes RUNTIME.
    // A first-call/unknown token must not initialize threads on a GC thread.
    if handle != 0
        && let Some(Ok(runtime)) = RUNTIME.get()
    {
        runtime.spawn(async move {
            tokio::task::spawn_blocking(move || aimux_drop(handle));
        });
    }
}

/// Release an owned string; NULL is harmless.
/// # Safety
/// The pointer must be NULL or an unfreed string returned by this ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aimux_free_string(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}

/// Release an owned error; NULL is harmless.
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_free(error: *mut aimux_error_t) {
    if !error.is_null() {
        drop(unsafe { Box::from_raw(error) });
    }
}

/// Read the saved error code, or zero for NULL.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_code(error: *const aimux_error_t) -> i32 {
    unsafe { error.as_ref() }.map_or(0, |error| error.envelope.code)
}

/// Return an owned error message, or NULL for a NULL error.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_message(error: *const aimux_error_t) -> *mut c_char {
    unsafe { error.as_ref() }
        .and_then(|error| CString::new(error.envelope.message.replace('\0', "\u{fffd}")).ok())
        .map_or(ptr::null_mut(), CString::into_raw)
}

/// Return the saved JSON envelope.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn aimux_error_json(error: *const aimux_error_t) -> *mut c_char {
    unsafe { error.as_ref() }
        .and_then(|error| json_string(&error.envelope).ok())
        .map_or(ptr::null_mut(), CString::into_raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct Batches {
        sizes: Vec<usize>,
        frames: Vec<(usize, Value)>,
        abort: u64,
    }

    extern "C" fn collect_batch(frames: *const *const c_char, count: usize, data: *mut c_void) {
        let batches = unsafe { &mut *data.cast::<Batches>() };
        batches.sizes.push(count);
        for &frame in unsafe { std::slice::from_raw_parts(frames, count) } {
            let bytes = unsafe { CStr::from_ptr(frame) }.to_bytes();
            batches
                .frames
                .push((bytes.len(), serde_json::from_slice(bytes).unwrap()));
        }
        if batches.abort != 0 {
            aimux_abort(batches.abort);
        }
    }

    fn batch_events(
        events: impl Stream<Item = std::result::Result<Value, OpsError>> + Send + 'static,
        abort: Option<aimux_core::AbortSignal>,
        batches: &mut Batches,
    ) {
        runtime().unwrap().block_on(async {
            tokio::time::timeout(
                Duration::from_secs(5),
                deliver_frame_batches(
                    dispatch::FrameStream::new(Box::pin(events), abort).into_wire(),
                    collect_batch,
                    CallbackContext(std::ptr::from_mut(batches).cast()),
                ),
            )
            .await
            .expect("callbacks must complete without waiting to fill a batch");
        });
    }

    #[test]
    fn burst_batches_are_capped_and_terminal_is_standalone_and_ordered() {
        let mut batches = Batches::default();
        batch_events(
            futures::stream::iter((0..200).map(|i| Ok(serde_json::json!({"text": i})))),
            None,
            &mut batches,
        );
        assert_eq!(batches.sizes, [64, 64, 64, 8, 1]);
        for (seq, (_, event)) in batches.frames[..200].iter().enumerate() {
            assert_eq!(event["seq"], seq.to_string());
            assert_eq!(event["event"]["text"], seq);
        }
        assert_eq!(
            batches.frames[200].1,
            serde_json::json!({"type":"end","seq":"200","outcome":"success"})
        );
    }

    #[derive(Default)]
    struct Consumer {
        frames: Vec<Value>,
        abort: u64,
        nested_codes: Vec<i32>,
    }

    extern "C" fn consume(frame: *const c_char, context: *mut c_void) {
        let consumer = unsafe { &mut *context.cast::<Consumer>() };
        consumer.frames.push(
            serde_json::from_str(unsafe { CStr::from_ptr(frame) }.to_str().unwrap()).unwrap(),
        );
        // The frame CString stays alive across nested calls into the adapter.
        let before = unsafe { CStr::from_ptr(frame) }.to_owned();
        aimux_abort(consumer.abort);
        let mut out = ptr::dangling_mut();
        let mut state = 99;
        let errors = [
            aimux_call(0, c"runtime.describe".as_ptr(), c"{}".as_ptr(), 0, &mut out),
            aimux_stream(
                0,
                c"text.stream".as_ptr(),
                c"{}".as_ptr(),
                0,
                Some(consume),
                ptr::null_mut(),
            ),
            aimux_operation_push(0, ptr::null(), 0),
            aimux_operation_next_part(0, 0, &mut out, &mut state),
        ];
        for error in errors {
            consumer.nested_codes.push(aimux_error_code(error));
            aimux_error_free(error);
        }
        assert!(out.is_null());
        assert_eq!(state, 0);
        assert_eq!(before.as_c_str(), unsafe { CStr::from_ptr(frame) });
    }

    #[test]
    fn callback_borrows_frames_rejects_reentry_and_allows_abort() {
        let signal = Arc::new(aimux_core::AbortSignal::new());
        let abort = HandleRegistry::global()
            .insert(Owner::CProcess, Entry::Abort(signal.clone()))
            .ok()
            .unwrap();
        let mut consumer = Consumer {
            abort: abort.0,
            ..Consumer::default()
        };
        let frames = futures::stream::iter([
            StreamFrame::Event {
                seq: "0".into(),
                event: serde_json::json!({"text":"hello"}),
            },
            StreamFrame::End {
                seq: "1".into(),
                outcome: Outcome::Cancelled,
                error: None,
            },
            StreamFrame::Event {
                seq: "2".into(),
                event: Value::Null,
            },
        ]);
        assert!(
            block_on(deliver_frames(
                frames.map(|frame| WireFrame::encode(frame).expect("test frame fits")),
                consume,
                (&mut consumer as *mut Consumer).cast()
            ))
            .is_ok()
        );
        assert!(signal.is_aborted());
        assert_eq!(consumer.frames.len(), 2);
        assert_eq!(consumer.frames[1]["type"], "end");
        assert_eq!(consumer.frames[1]["outcome"], "cancelled");
        assert_eq!(consumer.nested_codes, vec![204; 8]);
        aimux_drop(abort.0);
        assert!(block_on(async { Ok(()) }).is_ok());
    }

    #[test]
    fn terminal_error_is_delivered_only_in_callback() {
        let frames = vec![StreamFrame::End {
            seq: "0".into(),
            outcome: Outcome::Error,
            error: Some(
                OpsError::Protocol(ProtocolError::InvalidState("failed".into())).envelope(),
            ),
        }];
        let mut consumer = Consumer::default();
        assert!(
            block_on(deliver_frames(
                futures::stream::iter(frames)
                    .map(|frame| WireFrame::encode(frame).expect("test frame fits")),
                consume,
                (&mut consumer as *mut Consumer).cast()
            ))
            .is_ok()
        );
        assert_eq!(consumer.frames.len(), 1);
        assert_eq!(consumer.frames[0]["type"], "end");
        assert_eq!(consumer.frames[0]["outcome"], "error");
        assert_eq!(consumer.frames[0]["error"]["code"], 304);
    }

    struct EchoTranscription;

    #[async_trait::async_trait]
    impl aimux_core::transcription_model::TranscriptionModel for EchoTranscription {
        fn provider(&self) -> &str {
            "abi-test"
        }
        fn model_id(&self) -> &str {
            "echo"
        }
        async fn do_generate(
            &self,
            _: &aimux_core::transcription_model::TranscriptionCallOptions,
        ) -> std::result::Result<
            aimux_core::transcription_model::TranscriptionResult,
            aimux_core::AiMuxError,
        > {
            Err(aimux_core::AiMuxError::UnsupportedFunctionality(
                "batch".into(),
            ))
        }
        fn do_stream(
            &self,
            options: aimux_core::transcription_model::TranscriptionStreamOptions,
        ) -> Option<
            futures::future::BoxFuture<
                '_,
                std::result::Result<
                    aimux_core::transcription_model::TranscriptionStreamResult,
                    aimux_core::AiMuxError,
                >,
            >,
        > {
            use aimux_core::transcription_model::{
                AudioChunk, TranscriptionStreamPart, TranscriptionStreamResult,
            };
            Some(Box::pin(async move {
                Ok(TranscriptionStreamResult {
                    stream: Box::pin(options.audio.map(|audio| {
                        let AudioChunk::Binary(bytes) = audio else {
                            panic!("binary ABI input")
                        };
                        Ok(TranscriptionStreamPart::TranscriptDelta {
                            id: None,
                            delta: format!("{bytes:?}"),
                            provider_metadata: None,
                        })
                    })),
                    request: None,
                    response: None,
                })
            }))
        }
    }

    #[test]
    fn raw_abort_id_cancels_ffi_call_and_preserves_preflight_errors() {
        let registry = HandleRegistry::global();
        let model = registry
            .insert(
                Owner::CProcess,
                Entry::Transcription(Arc::new(EchoTranscription)),
            )
            .unwrap();
        let signal = Arc::new(aimux_core::AbortSignal::new());
        let abort = registry
            .insert(Owner::CProcess, Entry::Abort(signal.clone()))
            .unwrap();
        let foreign_owner = Owner::new_node().unwrap();
        let foreign = registry
            .insert(foreign_owner, Entry::Abort(signal))
            .unwrap();
        aimux_abort(abort.0);

        for (abort_id, params, code) in [
            (abort.0, c"{}", 13),
            (model.0, c"{}", 203),
            (foreign.0, c"{}", 203),
            (u64::MAX, c"{}", 203),
        ] {
            let mut output = ptr::dangling_mut();
            let error = aimux_call(
                model.0,
                c"transcription.start".as_ptr(),
                params.as_ptr(),
                abort_id,
                &mut output,
            );
            assert_eq!(aimux_error_code(error), code);
            assert!(output.is_null());
            aimux_error_free(error);
        }

        aimux_drop(abort.0);
        aimux_drop(model.0);
        registry.release_owner(foreign_owner);
    }

    #[test]
    fn real_operation_push_copies_bytes_and_poll_uses_the_shared_service() {
        use aimux_ops::operation::{Operation, OperationOptions};
        let operation = block_on(Operation::start(
            Arc::new(EchoTranscription),
            OperationOptions::default(),
            None,
        ))
        .unwrap();
        let id = HandleRegistry::global()
            .insert(Owner::CProcess, Entry::Operation(operation.clone()))
            .unwrap();
        let mut part = ptr::dangling_mut();
        let mut state = 99;
        assert!(aimux_operation_next_part(id.0, 0, &mut part, &mut state).is_null());
        assert_eq!(state, 3);
        assert!(part.is_null());
        let error = aimux_operation_next_part(id.0, -1, &mut part, &mut state);
        assert_eq!(aimux_error_code(error), 301);
        aimux_error_free(error);
        assert!(part.is_null());
        assert_eq!(state, 0);
        let mut bytes = [1, 2, 3];
        assert!(aimux_operation_push(id.0, bytes.as_ptr(), bytes.len()).is_null());
        bytes.fill(9);
        assert!(aimux_operation_next_part(id.0, 2000, &mut part, &mut state).is_null());
        assert_eq!(state, 1);
        let value: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(part) }.to_str().unwrap()).unwrap();
        assert_eq!(value["delta"], "[1, 2, 3]");
        unsafe { aimux_free_string(part) };
        assert!(aimux_operation_push(id.0, ptr::null(), 0).is_null());
        assert!(aimux_operation_next_part(id.0, 2000, &mut part, &mut state).is_null());
        assert_eq!(state, 1);
        unsafe { aimux_free_string(part) };
        let mut output = ptr::null_mut();
        assert!(
            aimux_call(
                id.0,
                c"operation.inputDone".as_ptr(),
                c"{}".as_ptr(),
                0,
                &mut output
            )
            .is_null()
        );
        unsafe { aimux_free_string(output) };
        for _ in 0..2 {
            assert!(aimux_operation_next_part(id.0, 2000, &mut part, &mut state).is_null());
            assert_eq!(state, 2);
            assert!(part.is_null());
        }
        aimux_drop(id.0);
        assert!(operation.status().consumer_closed);
    }

    #[test]
    fn message_getter_sanitizes_nul_and_keeps_the_json_envelope_exact() {
        let error = finish(Err(OpsError::AiMux(aimux_core::AiMuxError::other("a\0b"))));
        let message = aimux_error_message(error);
        assert!(!message.is_null());
        assert!(
            unsafe { CStr::from_ptr(message) }
                .to_str()
                .unwrap()
                .contains("a\u{fffd}b")
        );
        let raw = aimux_error_json(error);
        let value: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(raw) }.to_str().unwrap()).unwrap();
        assert!(value["message"].as_str().unwrap().contains("a\0b"));
        unsafe {
            aimux_free_string(message);
            aimux_free_string(raw);
        }
        aimux_error_free(error);
    }

    #[test]
    fn finalizer_enqueues_idempotent_c_owned_cleanup() {
        runtime().unwrap();
        let registry = HandleRegistry::global();
        let handle = registry
            .insert(
                Owner::CProcess,
                Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
            )
            .unwrap();
        aimux_handle_finalizer(handle.0 as usize as *mut c_void);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while registry
            .get(Owner::CProcess, handle, &[HandleKind::Abort])
            .is_ok()
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        aimux_handle_finalizer(handle.0 as usize as *mut c_void);
        aimux_handle_finalizer(ptr::null_mut());
    }
}
