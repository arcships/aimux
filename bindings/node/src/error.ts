/**
 * Aimux error hierarchy (aligned with Vercel AI SDK style).
 *
 * ```ts
 * try {
 *   await generateText(model, '...')
 * } catch (e) {
 *   if (e instanceof APICallError) {
 *     // e.status === 429 → rate limited (e.retryMs), 401 → auth, 404 → model
 *   } else if (e instanceof RetryError) {
 *     // every retry attempt failed — e.errors is the per-attempt history
 *   } else if (e instanceof AimuxError) {
 *     // any AiMuxError
 *   } else if (e instanceof Error && (e as { code?: string }).code === 'InvalidArg') {
 *     // the binding rejected an argument (plain Error, napi code)
 *   }
 * }
 * ```
 *
 * Distinct from the ts-rs wire type `AiMuxError` used inside `StreamPart`.
 */

export class AimuxError extends Error {
  protected constructor(message: string) {
    super(message)
    this.name = new.target.name
  }
}

/**
 * A call that reached (or tried to reach) the provider. Classification is
 * `status`; everything below is what that exchange produced, and lives on this
 * class alone — the core fills these for no other variant (AI SDK likewise
 * keeps `isRetryable` / `responseBody` / `data` on `APICallError`).
 */
export class APICallError extends AimuxError {
  /** Whether retrying this provider call may help. */
  declare readonly retryable: boolean
  /** HTTP status observed from the provider; absent for transport failures. */
  declare readonly status?: number
  /** Rate-limit hint in ms; absent if none; `0` means retry immediately. */
  declare readonly retryMs?: number
  /** Sanitized request URL; absent when unknown. */
  declare readonly url?: string
  /** Sanitized request body values sent to the provider. */
  declare readonly requestBodyValues?: unknown
  /** Provider's machine-readable error code (e.g. `'rate_limit_exceeded'`). */
  declare readonly providerCode?: string
  /** Provider's failure text without Aimux's composed prefix. */
  declare readonly providerMessage?: string
  /** Raw error response body, verbatim. */
  declare readonly responseBody?: string
  /** Sanitized response headers. */
  declare readonly responseHeaders?: Record<string, string>
  /** Parsed provider error data (AI SDK `APICallError.data`). */
  declare readonly data?: unknown
}

/** Why a {@link RetryError} stopped retrying (core serde wire names). */
export type RetryErrorReason = 'maxRetriesExceeded' | 'errorNotRetryable'

/**
 * A retried model operation gave up. `errors` is the complete per-attempt
 * history, oldest first; each entry is itself a full member of this hierarchy
 * (typically {@link APICallError} with its provider detail).
 */
export class RetryError extends AimuxError {
  /** Why retrying stopped. */
  declare readonly reason: RetryErrorReason
  /** Per-attempt errors, oldest first. */
  declare readonly errors: AimuxError[]

  /** The final attempt's error (`undefined` only for a deserialized empty history). */
  get lastError(): AimuxError | undefined {
    return this.errors[this.errors.length - 1]
  }
}

export class JSONParseError extends AimuxError {}
export class InvalidResponseDataError extends AimuxError {}
export class ToolError extends AimuxError {}
export class InvalidArgumentError extends AimuxError {}
export class InvalidPromptError extends AimuxError {}
export class TokenExpiredError extends AimuxError {
  /** Token expiry is produced only from an observed 401 response. */
  declare readonly status: 401
}
export class UnsupportedFunctionalityError extends AimuxError {}
/** Registry lookup failed for a model id (AI SDK `NoSuchModelError`). */
export class NoSuchModelError extends AimuxError {
  /** The model id that did not resolve. */
  declare readonly modelId: string
  /** Model category requested (`'languageModel'`, `'imageModel'`, …). */
  declare readonly modelType?: string
}
/** Registry lookup failed for a provider name (AI SDK `NoSuchProviderError`). */
export class NoSuchProviderError extends AimuxError {
  /** The provider name that did not resolve. */
  declare readonly providerId: string
}
export class TimeoutError extends AimuxError {}
/** Request aborted (not DOM `AbortError`); `message` is the abort payload. */
export class RequestAbortedError extends AimuxError {}
export class OtherError extends AimuxError {}

/**
 * Which recording failure a {@link RecordingError} reports — the core's
 * `recording::RecordingError` variant name. Only `WriterGone`, `FlushTimeout`
 * and `Write` can come out of a flush today.
 */
export type RecordingErrorCode =
  | 'Init' // create_dir_all failed
  | 'OpenFile' // opening recordings.jsonl failed
  | 'Spawn' // writer thread could not be spawned
  | 'WriterGone' // writer thread unavailable (unwritable dir)
  | 'FlushTimeout' // no writer ack within 30s
  | 'Write' // a prior write failed (sticky, e.g. ENOSPC)

/**
 * The recorder could not confirm data on disk (`recordingTryFlush`).
 *
 * The core has two unrelated error types — `AiMuxError` and
 * `recording::RecordingError` — and so does this binding: this class extends
 * `Error` directly, not {@link AimuxError}. Catch it separately.
 */
export class RecordingError extends Error {
  /** Which recording failure. */
  declare readonly code: RecordingErrorCode

  private constructor(message: string) {
    super(message)
    this.name = new.target.name
  }
}
