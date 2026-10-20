import type { ErrorDomain, ErrorEnvelope } from './generated/types.ts'
export type { ErrorDomain, ErrorEnvelope } from './generated/types.ts'

/** The exact shared ErrorEnvelope, including the AI SDK payload in `error`. */
export class AimuxError extends Error implements ErrorEnvelope {
  readonly kind: ErrorDomain
  readonly code: number
  readonly error: ErrorEnvelope['error']

  constructor(envelope: ErrorEnvelope) {
    super(envelope.message)
    this.name = 'AimuxError'
    this.kind = envelope.kind
    this.code = envelope.code
    this.error = envelope.error
  }
}

export function rethrow(error: unknown): never {
  if (isEnvelope(error)) {
    // Preserve the original native error's stack and all payload fields.
    Object.setPrototypeOf(error, AimuxError.prototype)
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
