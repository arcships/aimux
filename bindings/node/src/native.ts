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

export function verifyManifest(manifest: Pick<Manifest, 'abiVersion' | 'opsVersion' | 'digest'>): void {
  const expected = { abiVersion: ABI_VERSION, opsVersion: OPS_VERSION, digest: MANIFEST_DIGEST }
  for (const field of ['abiVersion', 'opsVersion', 'digest'] as const) {
    if (manifest[field] !== expected[field]) {
      throw protocolError(302, 'UnsupportedOpsVersion', `Incompatible manifest ${field}: expected ${expected[field]}, received ${String(manifest[field])}`)
    }
  }
}

export class Runtime {
  readonly #inner: NativeRuntime
  readonly manifest!: Manifest
  #manifest?: Manifest
  readonly manifestDigest: string

  constructor() {
    this.#inner = new NativeRuntime()
    Object.defineProperty(this, 'manifest', { enumerable: true, get: () => this.readManifest() })
    this.manifestDigest = this.#inner.manifestDigest()
    verifyManifest({
      abiVersion: this.#inner.abiVersion(),
      opsVersion: this.#inner.opsVersion(),
      digest: this.manifestDigest,
    })
  }

  private readManifest(): Manifest {
    if (this.#manifest === undefined) {
      const manifest = JSON.parse(this.#inner.manifestJson()) as Manifest
      verifyManifest(manifest)
      this.#manifest = manifest
    }
    return this.#manifest
  }

  abort(signal?: AbortSignal): NativeAbort { return new NativeAbort(this.#inner, signal) }

  drop(handle: string): void {
    try { this.#inner.dropHandle(handle) } catch (error) { rethrow(error) }
  }

  async call(handle: string, op: string, paramsJson: string, abort?: NativeAbort): Promise<string> {
    try { return await this.#inner.call(handle, op, paramsJson, abort) } catch (error) { rethrow(error) }
  }

  async stream(handle: string, op: string, paramsJson: string, abort?: NativeAbort): Promise<AsyncIterableIterator<string>> {
    let native: Awaited<ReturnType<NativeRuntime['stream']>>
    try { native = await this.#inner.stream(handle, op, paramsJson, abort) } catch (error) { rethrow(error) }
    const inner = native[Symbol.asyncIterator]()
    let batch: Array<string | undefined> = []
    let cursor = 0
    let reading = false
    let closed = false
    const close = () => { closed = true; batch = [] }
    // This is deliberately an explicit iterator: return() must cancel while next() is pending.
    const iterator: AsyncIterableIterator<string> = {
      async next() {
        if (closed) return { done: true, value: undefined }
        if (reading) throw protocolError(303, 'ConcurrentConsumer', 'stream already has a pending consumer')
        if (cursor === batch.length) {
          reading = true
          try {
            const frames = await native.nextBatch()
            if (closed || frames.length === 0) {
              close()
              return { done: true, value: undefined }
            }
            batch = frames
            cursor = 0
          } catch (error) { rethrow(error) } finally { reading = false }
        }
        const value = batch[cursor]!
        batch[cursor++] = undefined
        return { done: false, value }
      },
      async return() {
        close()
        try {
          await inner.return()
          return { done: true, value: undefined }
        } catch (error) { rethrow(error) }
      },
      async throw(error?: unknown) {
        close()
        try {
          await inner.throw(error)
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
