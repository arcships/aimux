import type { ErrorDomain, ErrorEnvelope } from './generated/types.ts'
export type { ErrorDomain, ErrorEnvelope } from './generated/types.ts'

/**
 * The exact shared ErrorEnvelope, including the AI SDK payload in `error`.
 * The payload's `cause` is also the standard `Error.cause`, restored as an
 * `AimuxCauseError` (a nested AI SDK error) or an `AimuxSourceError` (an
 * underlying OS, TLS or parser error), each with its own `cause` down the
 * chain. A retry error's `cause` is its last attempt, the AI SDK `lastError`.
 */
export class AimuxError extends Error implements ErrorEnvelope {
  readonly kind: ErrorDomain
  readonly code: number
  readonly error: ErrorEnvelope['error']
  declare readonly cause?: unknown

  constructor(envelope: ErrorEnvelope) {
    super(envelope.message)
    this.name = 'AimuxError'
    this.kind = envelope.kind
    this.code = envelope.code
    this.error = envelope.error
    attachCause(this, this.error)
  }
}

/** A nested AI SDK error: `name` is its AI SDK name and `error` its whole payload. */
export class AimuxCauseError extends Error {
  readonly error: Record<string, unknown>
  declare readonly cause?: unknown

  constructor(payload: Record<string, unknown>) {
    super(typeof payload.message === 'string' ? payload.message : String(payload.name))
    this.name = String(payload.name)
    this.error = payload
    attachCause(this, payload)
  }
}

/** An underlying OS, TLS or parser error, from `{ message, cause? }`. */
export class AimuxSourceError extends Error {
  declare readonly cause?: unknown

  constructor(payload: Record<string, unknown>) {
    super(typeof payload.message === 'string' ? payload.message : '')
    this.name = 'AimuxSourceError'
    attachCause(this, payload)
  }
}

function causeOf(payload: Record<string, unknown>): unknown {
  if ('cause' in payload) return payload.cause
  // The AI SDK's `lastError`, as Rust's `source()` of a retry error.
  if (payload.name === 'AI_RetryError' && Array.isArray(payload.errors)) return payload.errors[payload.errors.length - 1]
  return undefined
}

function restore(cause: unknown): unknown {
  if (cause === null || typeof cause !== 'object') return cause
  const payload = cause as Record<string, unknown>
  return typeof payload.name === 'string' ? new AimuxCauseError(payload) : new AimuxSourceError(payload)
}

// Like `new Error(message, { cause })`: an own, non-enumerable property.
function attachCause(error: Error, payload: unknown): void {
  if (payload === null || typeof payload !== 'object') return
  const cause = causeOf(payload as Record<string, unknown>)
  if (cause !== undefined) {
    Object.defineProperty(error, 'cause', { value: restore(cause), writable: true, configurable: true })
  }
}

export function rethrow(error: unknown): never {
  if (isEnvelope(error)) {
    // Preserve the original native error's stack and all payload fields.
    Object.setPrototypeOf(error, AimuxError.prototype)
    attachCause(error as AimuxError, error.error)
  }
  throw error
}

export function isEnvelope(value: unknown): value is ErrorEnvelope {
  if (value === null || typeof value !== 'object') return false
  const e = value as Partial<ErrorEnvelope>
  return ['aimux', 'recording', 'binding', 'protocol'].includes(e.kind as string)
    && typeof e.code === 'number' && typeof e.message === 'string'
    && e.error !== null && typeof e.error === 'object' && 'name' in e.error && typeof e.error.name === 'string'
}

export function protocolError(code: number, name: string, message: string): AimuxError {
  return new AimuxError({ kind: 'protocol', code, message, error: { name, message } })
}
