/**
 * Aimux error hierarchy (aligned with Vercel AI SDK style).
 *
 * ```ts
 * try {
 *   await generateText(model, '...')
 * } catch (e) {
 *   if (e instanceof APICallError) {
 *     // Classification is the status field (AI SDK APICallError.statusCode):
 *     // e.status === 429 → rate limited (e.retryMs), 401 → auth, 404 → model
 *   } else if (e instanceof AimuxError) {
 *     // any engine / binding failure
 *   }
 * }
 * ```
 *
 * Distinct from the ts-rs wire type `AiMuxError` used inside `StreamPart`.
 */

export class AimuxError extends Error {
  /** Core variant name, e.g. `'InvalidArgument'`. */
  readonly code: string
  readonly status: number
  readonly retryMs: number
  /** Stored retry verdict (`APICallError.isRetryable` analogue). */
  readonly retryable: boolean
  /** Provider's machine-readable error code (e.g. `'rate_limit_exceeded'`). */
  readonly providerCode?: string
  /** Raw error response body, verbatim (AI SDK `APICallError.responseBody`). */
  readonly responseBody?: string
  /** Provider-assigned request id (`x-request-id` / `request-id` header). */
  readonly requestId?: string
  /**
   * Lossless externally-tagged serde JSON of the core `AiMuxError`,
   * e.g. `'{"ApiCall":{"status_code":429,"retry_after_ms":1500,...}}'`.
   * Absent when the error did not originate from a core `AiMuxError`.
   */
  readonly errorValue?: string

  constructor(message: string, status: number = -1, retryMs: number = -1, retryable: boolean = false) {
    super(message)
    this.name = new.target.name
    this.code = CODE_BY_NAME.get(this.name) ?? 'Other'
    this.status = status
    this.retryMs = retryMs
    this.retryable = retryable
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
    let retryable = false
    let providerCode: string | undefined
    let responseBody: string | undefined
    let requestId: string | undefined
    let errorValue: string | undefined

    if (err instanceof Error) {
      message = err.message
      const any = err as Error & {
        code?: unknown
        status?: unknown
        retryMs?: unknown
        retryable?: unknown
        providerCode?: unknown
        responseBody?: unknown
        requestId?: unknown
        errorValue?: unknown
      }

      if (typeof any.status === 'number') status = any.status
      if (typeof any.retryMs === 'number') retryMs = any.retryMs
      if (typeof any.retryable === 'boolean') retryable = any.retryable
      if (typeof any.providerCode === 'string') providerCode = any.providerCode
      if (typeof any.responseBody === 'string') responseBody = any.responseBody
      if (typeof any.requestId === 'string') requestId = any.requestId
      if (typeof any.errorValue === 'string') errorValue = any.errorValue

      if (typeof any.code === 'string' && any.code.length > 0) {
        code = any.code
      } else if (typeof any.name === 'string') {
        // Native sets name to e.g. "APICallError"
        code = CODE_BY_NAME.get(any.name) ?? 'Other'
      }
    }

    const wrapped = createByCode(code, message, status, retryMs, retryable)
    if (providerCode !== undefined) {
      ;(wrapped as { providerCode?: string }).providerCode = providerCode
    }
    if (responseBody !== undefined) {
      ;(wrapped as { responseBody?: string }).responseBody = responseBody
    }
    if (requestId !== undefined) {
      ;(wrapped as { requestId?: string }).requestId = requestId
    }
    if (errorValue !== undefined) {
      ;(wrapped as { errorValue?: string }).errorValue = errorValue
    }
    if (err instanceof Error) {
      ;(wrapped as Error & { cause?: unknown }).cause = err
    }
    return wrapped
  }
}

export class APICallError extends AimuxError {}
export class JSONParseError extends AimuxError {}
export class InvalidResponseDataError extends AimuxError {}
export class ToolError extends AimuxError {}
export class InvalidArgumentError extends AimuxError {}
export class InvalidPromptError extends AimuxError {}
export class TokenExpiredError extends AimuxError {}
export class UnsupportedFunctionalityError extends AimuxError {}
export class NoSuchModelError extends AimuxError {}
export class NoSuchProviderError extends AimuxError {}
export class TimeoutError extends AimuxError {}
/** Request aborted (not DOM `AbortError`). */
export class RequestAbortedError extends AimuxError {
  constructor(
    message: string = 'request aborted',
    status: number = -1,
    retryMs: number = -1,
    retryable: boolean = false,
  ) {
    super(message, status, retryMs, retryable)
  }
}
export class OtherError extends AimuxError {}

/** Core variant name → error class (single source for lookups below). */
const CLASS_BY_CODE: Record<
  string,
  new (message: string, status?: number, retryMs?: number, retryable?: boolean) => AimuxError
> = {
  ApiCall: APICallError,
  JsonParse: JSONParseError,
  InvalidResponseData: InvalidResponseDataError,
  Tool: ToolError,
  InvalidArgument: InvalidArgumentError,
  InvalidPrompt: InvalidPromptError,
  TokenExpired: TokenExpiredError,
  UnsupportedFunctionality: UnsupportedFunctionalityError,
  NoSuchModel: NoSuchModelError,
  NoSuchProvider: NoSuchProviderError,
  Timeout: TimeoutError,
  Aborted: RequestAbortedError,
  Other: OtherError,
}

const CODE_BY_NAME = new Map(
  Object.entries(CLASS_BY_CODE).map(([code, cls]) => [cls.name, code]),
)

function createByCode(
  code: string,
  message: string,
  status: number,
  retryMs: number,
  retryable: boolean,
): AimuxError {
  const Cls = CLASS_BY_CODE[code] ?? AimuxError
  return new Cls(message, status, retryMs, retryable)
}

/** Wrap an async native call so throws are typed `AimuxError` subclasses. */
export async function withAimuxError<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn()
  } catch (e) {
    throw AimuxError.fromNative(e)
  }
}
