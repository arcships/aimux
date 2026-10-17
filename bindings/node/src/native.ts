// Raw JSON entrypoint. The adapter is asynchronous and never calls the C ABI.
import { Runtime as NativeRuntime, Abort as NativeAbort } from '../index.js'
import { ABI_VERSION, OPS_VERSION, MANIFEST_DIGEST } from './generated/ops.ts'
import { rethrow, protocolError } from './error.ts'
export * from './error.ts'
export type { FrameIterator } from '../index.js'
export type { NativeAbort as Abort }

export interface Manifest {
  abiVersion: number
  opsVersion: string
  digest: string
  manifestRevision: string | number
  ops: ReadonlyArray<{ name: string; mode: string; [key: string]: unknown }>
  handleKinds: ReadonlyArray<unknown>
  errorCodes: ReadonlyArray<unknown>
  [key: string]: unknown
}

export function verifyManifest(manifest: Manifest): void {
  for (const [field, expected] of Object.entries({ abiVersion: ABI_VERSION, opsVersion: OPS_VERSION, digest: MANIFEST_DIGEST })) {
    if (manifest[field] !== expected) {
      throw protocolError(302, 'UnsupportedOpsVersion', `Incompatible manifest ${field}: expected ${expected}, received ${String(manifest[field])}`)
    }
  }
}

export class Runtime {
  readonly #inner: NativeRuntime
  readonly manifest: Manifest
  readonly manifestDigest: string

  constructor() {
    this.#inner = new NativeRuntime()
    this.manifest = JSON.parse(this.#inner.manifestJson()) as Manifest
    verifyManifest(this.manifest)
    this.manifestDigest = this.#inner.manifestDigest()
  }

  abort(signal?: AbortSignal): NativeAbort { return new NativeAbort(this.#inner, signal) }

  drop(handle: string): void {
    try { this.#inner.dropHandle(handle) } catch (error) { rethrow(error) }
  }

  async call(handle: string, op: string, paramsJson: string, abort?: NativeAbort): Promise<string> {
    try { return await this.#inner.call(handle, op, paramsJson, abort) } catch (error) { rethrow(error) }
  }

  async stream(handle: string, op: string, paramsJson: string, abort?: NativeAbort): Promise<AsyncIterableIterator<string>> {
    let inner: AsyncIterableIterator<string>
    try { inner = (await this.#inner.stream(handle, op, paramsJson, abort))[Symbol.asyncIterator]() } catch (error) { rethrow(error) }
    // This is deliberately an explicit iterator: return() must cancel while next() is pending.
    const iterator: AsyncIterableIterator<string> = {
      async next() { try { return await inner.next() } catch (error) { rethrow(error) } },
      async return() { try { return inner.return ? await inner.return() : { done: true as const, value: undefined } } catch (error) { rethrow(error) } },
      async throw(error?: unknown) {
        try {
          if (inner.throw) return await inner.throw(error)
          if (inner.return) await inner.return()
          throw error
        } catch (error) { rethrow(error) }
      },
      [Symbol.asyncIterator]() { return this },
    }
    return iterator
  }

  async operationPush(handle: string, data: Uint8Array): Promise<void> {
    try { await this.#inner.operationPush(handle, data) } catch (error) { rethrow(error) }
  }
}
