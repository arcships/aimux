// Raw JSON entrypoint. The adapter is asynchronous and never calls the C ABI.
import { Runtime as NativeRuntime, Abort as NativeAbort } from '../index.js'
import { ABI_VERSION, OPS_VERSION, MANIFEST_DIGEST, TYPE_SCHEMA_HASHES } from './generated/ops.ts'
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
  types?: ReadonlyArray<{ id?: unknown; schemaHash?: unknown; [key: string]: unknown }>
  [key: string]: unknown
}

function findMismatchedTypes(types: unknown): string[] | null {
  const expectedEntries = Object.entries(TYPE_SCHEMA_HASHES)
  if (expectedEntries.length === 0 || !Array.isArray(types) || types.length === 0) {
    return null
  }
  const actual = new Map<string, string>()
  for (const entry of types) {
    if (entry !== null && typeof entry === 'object') {
      const record = entry as { id?: unknown; schemaHash?: unknown }
      if (typeof record.id === 'string' && typeof record.schemaHash === 'string') {
        actual.set(record.id, record.schemaHash)
      }
    }
  }
  const mismatches: string[] = []
  for (const [id, expectedHash] of expectedEntries) {
    if (actual.get(id) !== expectedHash) {
      mismatches.push(id)
    }
  }
  return mismatches
}

function verifyVersions(abiVersion: unknown, opsVersion: unknown): void {
  const expected = { abiVersion: ABI_VERSION, opsVersion: OPS_VERSION }
  const actual = { abiVersion, opsVersion }
  for (const field of ['abiVersion', 'opsVersion'] as const) {
    if (actual[field] !== expected[field]) {
      throw protocolError(302, 'UnsupportedOpsVersion', `Incompatible manifest ${field}: expected ${expected[field]}, received ${String(actual[field])}`)
    }
  }
}

export function verifyManifest(manifest: Pick<Manifest, 'abiVersion' | 'opsVersion' | 'digest'> & Partial<Pick<Manifest, 'types'>>): void {
  verifyVersions(manifest.abiVersion, manifest.opsVersion)
  if (manifest.digest === MANIFEST_DIGEST) {
    return
  }
  const mismatches = findMismatchedTypes(manifest.types)
  if (mismatches !== null && mismatches.length === 0) {
    return
  }
  const detail = mismatches && mismatches.length > 0 ? ` (incompatible types: ${mismatches.slice(0, 5).join(', ')})` : ''
  throw protocolError(302, 'UnsupportedOpsVersion', `Incompatible manifest digest: expected ${MANIFEST_DIGEST}, received ${String(manifest.digest)}${detail}`)
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
    const abiVersion = this.#inner.abiVersion()
    const opsVersion = this.#inner.opsVersion()
    verifyVersions(abiVersion, opsVersion)
    if (this.manifestDigest !== MANIFEST_DIGEST) {
      this.#manifest = this.parseAndVerifyManifest(abiVersion, opsVersion)
    }
  }

  private parseAndVerifyManifest(expectedAbi: number, expectedOps: string): Manifest {
    const manifest = JSON.parse(this.#inner.manifestJson()) as Manifest
    if (manifest.abiVersion !== expectedAbi || manifest.opsVersion !== expectedOps) {
      verifyVersions(manifest.abiVersion, manifest.opsVersion)
    }
    if (manifest.digest !== this.manifestDigest) {
      throw protocolError(302, 'UnsupportedOpsVersion', `Incompatible manifest digest: expected ${this.manifestDigest}, received ${String(manifest.digest)}`)
    }
    verifyManifest(manifest)
    return manifest
  }

  private readManifest(): Manifest {
    if (this.#manifest === undefined) {
      this.#manifest = this.parseAndVerifyManifest(ABI_VERSION, OPS_VERSION)
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
