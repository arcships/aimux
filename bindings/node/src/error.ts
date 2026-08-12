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
  /**
   * HTTP status when the error carries one — the observed status on
   * `APICallError`, `401` on `TokenExpiredError` — otherwise `-1`.
   */
  readonly status: number
  /**
   * Lossless externally-tagged serde JSON of the core `AiMuxError`,
   * e.g. `'{"ApiCall":{"status_code":429,"retry_after_ms":1500,...}}'`.
   * Absent when the error did not originate from a core `AiMuxError`.
   */
  readonly errorValue?: string

  constructor(message: string, status: number = -1) {
    super(message)
    this.name = new.target.name
    this.code = CODE_BY_NAME.get(this.name) ?? 'Other'
    this.status = status
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

    const props = (err instanceof Error ? err : {}) as Record<string, unknown>
    const str = (key: string): string | undefined =>
      typeof props[key] === 'string' ? (props[key] as string) : undefined

    let message = String(err)
    let code = 'Other'
    let status = -1
    let retryMs = -1
    let retryable = false

    if (err instanceof Error) {
      message = err.message
      if (typeof props.status === 'number') status = props.status
      if (typeof props.retryMs === 'number') retryMs = props.retryMs
      if (typeof props.retryable === 'boolean') retryable = props.retryable
      // Native sets `name` to the subclass, e.g. "APICallError".
      code = str('code') || CODE_BY_NAME.get(err.name) || 'Other'
    }

    const wrapped = createByCode(code, message, status, retryMs, retryable)
    // Each payload belongs to the one variant the core fills it for.
    if (wrapped instanceof APICallError) {
      assign(wrapped, {
        providerCode: str('providerCode'),
        providerMessage: str('providerMessage'),
        responseBody: str('responseBody'),
        requestId: str('requestId'),
      })
    } else if (wrapped instanceof NoSuchModelError) {
      assign(wrapped, { modelId: str('modelId'), modelType: str('modelType') })
    } else if (wrapped instanceof NoSuchProviderError) {
      assign(wrapped, { providerId: str('providerId') })
    }
    assign(wrapped, { errorValue: str('errorValue') })
    if (err instanceof Error) {
      ;(wrapped as Error & { cause?: unknown }).cause = err
    }
    return wrapped
  }
}

/** Set the fields that are present; they are `readonly` to callers only. */
function assign(target: object, fields: Record<string, string | undefined>): void {
  for (const [key, value] of Object.entries(fields)) {
    if (value !== undefined) (target as Record<string, unknown>)[key] = value
  }
}

/**
 * A call that reached (or tried to reach) the provider. Classification is
 * `status`; everything below is what that exchange produced, and lives on this
 * class alone — the core fills these for no other variant (AI SDK likewise
 * keeps `isRetryable` / `responseBody` / `data` on `APICallError`).
 */
export class APICallError extends AimuxError {
  /** Rate-limit hint in ms; `-1` if none; `0` means retry immediately. */
  readonly retryMs: number
  /** Stored retry verdict (AI SDK `APICallError.isRetryable`). */
  readonly retryable: boolean
  /** Provider's machine-readable error code (e.g. `'rate_limit_exceeded'`). */
  readonly providerCode?: string
  /**
   * The failure's own text, without the composed prefix: `message` reads
   * `'API call error: HTTP 429: slow down'`, this reads `'slow down'`.
   * Usually the provider's words; on a transport failure or a body the
   * extractor could not read, it is ours. `responseBody` is the evidence.
   */
  readonly providerMessage?: string
  /** Raw error response body, verbatim (AI SDK `APICallError.responseBody`). */
  readonly responseBody?: string
  /** Provider-assigned request id (`x-request-id` / `request-id` header). */
  readonly requestId?: string

  constructor(
    message: string,
    status: number = -1,
    retryMs: number = -1,
    retryable: boolean = false,
  ) {
    super(message, status)
    this.retryMs = retryMs
    this.retryable = retryable
  }
}
export class JSONParseError extends AimuxError {}
export class InvalidResponseDataError extends AimuxError {}
export class ToolError extends AimuxError {}
export class InvalidArgumentError extends AimuxError {}
export class InvalidPromptError extends AimuxError {}
export class TokenExpiredError extends AimuxError {}
export class UnsupportedFunctionalityError extends AimuxError {}
/** Registry lookup failed for a model id (AI SDK `NoSuchModelError`). */
export class NoSuchModelError extends AimuxError {
  /** The model id that did not resolve. */
  readonly modelId?: string
  /** Kind of model requested (`'languageModel'`, `'imageModel'`, …). */
  readonly modelType?: string
}
/** Registry lookup failed for a provider name (AI SDK `NoSuchProviderError`). */
export class NoSuchProviderError extends AimuxError {
  /** The provider name that did not resolve. */
  readonly providerId?: string
}
export class TimeoutError extends AimuxError {}
/** Request aborted (not DOM `AbortError`). */
export class RequestAbortedError extends AimuxError {
  constructor(message: string = 'request aborted', status: number = -1) {
    super(message, status)
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
