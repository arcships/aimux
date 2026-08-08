/**
 * Aimux error hierarchy (aligned with Vercel AI SDK style).
 *
 * ```ts
 * try {
 *   await generateText(model, '...')
 * } catch (e) {
 *   if (e instanceof RateLimitedError) {
 *     // e.retryMs
 *   } else if (e instanceof AuthenticationError) {
 *     // e.status === 401
 *   } else if (e instanceof AimuxError) {
 *     // any engine / binding failure
 *   }
 * }
 * ```
 *
 * Distinct from the ts-rs wire type `AiMuxError` used inside `StreamPart`.
 */

/** Shared fields on every aimux error. */
export type AimuxErrorFields = {
  /** HTTP status when known; otherwise `-1`. */
  status: number
  /** Rate-limit hint in ms; `-1` if none; `0` means retry immediately. */
  retryMs: number
  /** Externally-tagged serde JSON of the core `AiMuxError`, when known. */
  errorValue?: string
}

export class AimuxError extends Error {
  /** Core `error_type()` code, e.g. `'InvalidArgument'`. */
  readonly code: string
  readonly status: number
  readonly retryMs: number
  /**
   * Lossless externally-tagged serde JSON of the core `AiMuxError`,
   * e.g. `'{"RateLimited":{"retry_after_ms":1500,"message":"..."}}'`.
   * Absent when the error did not originate from a core `AiMuxError`.
   */
  readonly errorValue?: string

  constructor(message: string, status: number = -1, retryMs: number = -1) {
    super(message)
    this.name = new.target.name
    this.code = CODE_BY_NAME.get(this.name) ?? 'Other'
    this.status = status === -1 ? (DEFAULT_STATUS[this.code] ?? -1) : status
    this.retryMs = retryMs
    Object.setPrototypeOf(this, new.target.prototype)
    const capture = (Error as { captureStackTrace?: (t: object, ctor: unknown) => void })
      .captureStackTrace
    if (capture) capture(this, new.target)
  }

  /** Rebuild from a native throw (or already-wrapped instance). */
  static fromNative(err: unknown): AimuxError {
    if (err instanceof AimuxError) {
      return err
    }

    let message = String(err)
    let code = 'Other'
    let status = -1
    let retryMs = -1
    let errorValue: string | undefined

    if (err instanceof Error) {
      message = err.message
      const any = err as Error & {
        code?: unknown
        status?: unknown
        retryMs?: unknown
        errorValue?: unknown
      }

      if (typeof any.status === 'number') status = any.status
      if (typeof any.retryMs === 'number') retryMs = any.retryMs
      if (typeof any.errorValue === 'string') errorValue = any.errorValue

      if (typeof any.code === 'string' && any.code.length > 0) {
        code = any.code
      } else if (typeof any.name === 'string') {
        // Native sets name to e.g. "AuthenticationError"
        code = CODE_BY_NAME.get(any.name) ?? 'Other'
      }
    }

    const wrapped = createByCode(code, message, status, retryMs)
    if (errorValue !== undefined) {
      ;(wrapped as { errorValue?: string }).errorValue = errorValue
    }
    if (err instanceof Error) {
      ;(wrapped as Error & { cause?: unknown }).cause = err
    }
    return wrapped
  }
}

export class ProviderError extends AimuxError {}
export class HttpError extends AimuxError {}
export class JsonError extends AimuxError {}
export class StreamError extends AimuxError {}
export class ToolError extends AimuxError {}
export class InvalidArgumentError extends AimuxError {}
export class InvalidPromptError extends AimuxError {}
export class RateLimitedError extends AimuxError {}
/** Auth / bad API key (HTTP 401). Named after OpenAI/Anthropic `AuthenticationError`. */
export class AuthenticationError extends AimuxError {}
export class TokenExpiredError extends AimuxError {}
export class ModelNotFoundError extends AimuxError {}
export class UnsupportedError extends AimuxError {}
export class NoSuchModelError extends AimuxError {}
export class UnknownProviderError extends AimuxError {}
/** Provider HTTP / API call failure (AI SDK `APICallError` analogue). */
export class APICallError extends AimuxError {}
export class TimeoutError extends AimuxError {}
/** Request aborted (not DOM `AbortError`). */
export class RequestAbortedError extends AimuxError {
  constructor(message: string = 'request aborted', status: number = -1, retryMs: number = -1) {
    super(message, status, retryMs)
  }
}
export class OtherError extends AimuxError {}

/** Core `error_type()` code → error class (single source for lookups below). */
const CLASS_BY_CODE: Record<
  string,
  new (message: string, status?: number, retryMs?: number) => AimuxError
> = {
  Provider: ProviderError,
  Http: HttpError,
  Json: JsonError,
  Stream: StreamError,
  Tool: ToolError,
  InvalidArgument: InvalidArgumentError,
  InvalidPrompt: InvalidPromptError,
  RateLimited: RateLimitedError,
  Auth: AuthenticationError,
  TokenExpired: TokenExpiredError,
  ModelNotFound: ModelNotFoundError,
  Unsupported: UnsupportedError,
  NoSuchModel: NoSuchModelError,
  UnknownProvider: UnknownProviderError,
  ApiCall: APICallError,
  Timeout: TimeoutError,
  Aborted: RequestAbortedError,
  Other: OtherError,
}

/** Default HTTP status per code when the native side reports none. */
const DEFAULT_STATUS: Record<string, number> = {
  RateLimited: 429,
  Auth: 401,
  TokenExpired: 401,
  ModelNotFound: 404,
}

const CODE_BY_NAME = new Map(
  Object.entries(CLASS_BY_CODE).map(([code, cls]) => [cls.name, code]),
)

function createByCode(
  code: string,
  message: string,
  status: number,
  retryMs: number,
): AimuxError {
  const Cls = CLASS_BY_CODE[code] ?? AimuxError
  return new Cls(message, status, retryMs)
}

/** Wrap an async native call so throws are typed `AimuxError` subclasses. */
export async function withAimuxError<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn()
  } catch (e) {
    throw AimuxError.fromNative(e)
  }
}
