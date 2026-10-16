import { Runtime } from './native.ts'
import type { Abort as RawAbort, Manifest } from './native.ts'
import { AimuxError, protocolError } from './error.ts'
import type {
  DecideOptions,
  DecideResult,
  EmbeddingCallOptions,
  EmbeddingResult,
  GenerateObjectResult,
  GenerateTextOptions,
  GenerateTextResult,
  HandleKind,
  HandleResult,
  ImageCallOptions,
  ImageResult,
  ModelInfo,
  ModelMethod,
  ModelPrompt,
  ModelRef,
  OperationPollResult,
  ProviderCreateParams,
  RerankingCallOptions,
  RerankingResult,
  SearchCallOptions,
  SearchResult,
  SessionChainView,
  SessionStoreCreateParams,
  SpeechCallOptions,
  SpeechResult,
  StreamFrame,
  TraceAggregateParams,
  TraceStats,
  TranscriptionCallOptions,
  TranscriptionResult,
  TranscriptionStreamPart,
  UploadFileParams,
  UploadFileResult,
  VideoCallOptions,
  VideoResult,
} from './generated/types.ts'

export * from './error.ts'
export type * from './types.ts'
export type { Manifest } from './native.ts'

export interface CallOptions { signal?: AbortSignal; abort?: Abort }

function json(value: unknown): string {
  try {
    const serialized = JSON.stringify(value, (_key, item: unknown) => {
      if (typeof item === 'function' || typeof item === 'symbol') throw new TypeError('Ops parameters must contain JSON data only')
      return item
    })
    if (serialized === undefined) throw new TypeError('Parameters must be JSON data')
    return serialized
  } catch (error) {
    throw new AimuxError({ kind: 'binding', code: 202, message: `params: ${String(error)}`, error: { name: 'InvalidWireJson', argument: 'params', message: String(error) } })
  }
}

/** A Client owns a realm-local runtime; IDs must never be transferred to workers. */
export class Client {
  readonly raw: Runtime
  constructor() { this.raw = new Runtime() }
  get manifest(): Manifest { return this.raw.manifest }
  get manifestDigest(): string { return this.raw.manifestDigest }

  async call<T = unknown>(handle: string, op: string, params: unknown = {}, options: CallOptions = {}): Promise<T> {
    const abort = this.bridge(options)
    return JSON.parse(await this.raw.call(handle, op, json(params), abort)) as T
  }

  async stream(handle: string, op: string, params: unknown = {}, options: CallOptions = {}): Promise<AsyncIterableIterator<StreamFrame>> {
    const abort = this.bridge(options)
    const inner = await this.raw.stream(handle, op, json(params), abort)
    return mapIterator(inner, value => JSON.parse(value) as StreamFrame, () => abort?.signal())
  }

  private bridge(options: CallOptions): RawAbort | undefined {
    if (options.abort) {
      if (options.signal) throw protocolError(301, 'InvalidParams', 'Pass either signal or abort')
      return options.abort.native
    }
    return options.signal ? this.raw.abort(options.signal) : undefined
  }

  async provider(params: ProviderCreateParams): Promise<Provider> { return new Provider(this, await this.call('0', 'provider.create', params)) }
  async defaultProvider(params: unknown): Promise<Provider> { return new Provider(this, await this.call('0', 'provider.default', params)) }
  async customProvider(params: unknown): Promise<Provider> { return new Provider(this, await this.call('0', 'provider.custom', params)) }
  async registry(entries: Record<string, Provider>, namespace = ''): Promise<Registry> {
    const refs = Object.entries(entries).map(([key, provider]) => ({ key, provider: provider.handle }))
    return new Registry(this, await this.call('0', 'registry.create', { namespace, entries: refs }))
  }
  abort(signal?: AbortSignal): Abort { return new Abort(this, signal) }
  async configure(params: unknown): Promise<unknown> { return this.call('0', 'runtime.configure', params) }
  async describe(): Promise<unknown> { return this.call('0', 'runtime.describe') }
  async trace(params: unknown = {}): Promise<TraceStore> { return new TraceStore(this, handleRef(await this.call('0', 'trace.create', params), 'TraceStore')) }
  async recorder(params: unknown): Promise<Recorder> { return new Recorder(this, handleRef(await this.call('0', 'recorder.create', params), 'Recorder')) }
  async sessionStore(params: SessionStoreCreateParams): Promise<SessionStore> { return new SessionStore(this, handleRef(await this.call('0', 'sessionStore.create', params), 'SessionStore')) }
  async replay(params: unknown): Promise<ReplaySession> { return new ReplaySession(this, handleRef(await this.call('0', 'replay.create', params), 'ReplaySession')) }
  async middleware(params: unknown): Promise<Middleware> { return new Middleware(this, await this.call('0', 'middleware.create', params)) }
  async transport(params: unknown): Promise<Resource> { return new Resource(this, await this.call('0', 'transport.create', params)) }
  async credential(params: unknown): Promise<Resource> { return new Resource(this, await this.call('0', 'credential.create', params)) }
  async catalogue(params: unknown): Promise<unknown> { return this.call('0', 'catalogue.fetch', params) }
  async router(params: unknown): Promise<LanguageModel> { return new LanguageModel(this, handleRef(await this.call('0', 'composite.router', params), 'Language')) }
  async moa(params: unknown): Promise<LanguageModel> { return new LanguageModel(this, handleRef(await this.call('0', 'composite.moa', params), 'Language')) }
}

// Finalization is a fallback. Explicit close/flush remains the normal protocol.
const finalizer = new FinalizationRegistry<{ client: Client; handle: string }>(held => {
  try { held.client.raw.drop(held.handle) } catch { /* Env cleanup owns remaining resources. */ }
})

export class Resource {
  readonly client: Client
  readonly kind: HandleKind
  #handle: string | undefined
  constructor(client: Client, ref: HandleResult) {
    this.client = client
    this.kind = ref.kind
    this.#handle = ref.handle
    finalizer.register(this, { client, handle: ref.handle }, this)
  }
  get handle(): string {
    if (this.#handle === undefined) throw new AimuxError({ kind: 'binding', code: 203, message: 'Handle is closed', error: { name: 'InvalidHandle', expected: [this.kind] } })
    return this.#handle
  }
  async close(): Promise<void> {
    const handle = this.#handle
    if (handle === undefined) return
    this.#handle = undefined
    finalizer.unregister(this)
    this.client.raw.drop(handle)
  }
  protected call<T = unknown>(op: string, params: unknown = {}, options?: CallOptions): Promise<T> { return this.client.call(this.handle, op, params, options) }
}

export class Abort {
  readonly client: Client
  readonly native: RawAbort
  constructor(client: Client, signal?: AbortSignal) { this.client = client; this.native = client.raw.abort(signal) }
  get handle(): string { return this.native.handle }
  get aborted(): boolean { return this.native.aborted }
  signal(): void { this.native.signal() }
  close(): void { this.native.close() }
}

export class Provider extends Resource {
  async model(modelId: string, method?: ModelMethod): Promise<Model> {
    return model(this.client, await this.call('provider.model', { modelId, ...(method ? { method } : {}) }))
  }
  async invoke<T = unknown>(method: string, call?: CallOptions): Promise<T> { return this.call('provider.invoke', { method }, call) }
  async files(): Promise<Files> { return new Files(this.client, handleRef(await this.call('provider.files'), 'Files')) }
  async wrap(language: Middleware[] = [], image: Middleware[] = []): Promise<Provider> {
    return new Provider(this.client, await this.call('provider.wrap', { languageModelMiddleware: language.map(value => value.handle), imageModelMiddleware: image.map(value => value.handle) }))
  }
}
export class Registry extends Resource {
  async provider(key: string): Promise<Provider> { return new Provider(this.client, await this.call('registry.provider', { key })) }
  async resolve(modelRef: ModelRef): Promise<Model> { return model(this.client, await this.call('model.resolve', { modelRef })) }
}
export class Model extends Resource {
  async info(): Promise<ModelInfo> { return this.call('model.info') }
  async wrap(middleware: Middleware[]): Promise<Model> {
    return model(this.client, await this.call('model.wrap', { middleware: middleware.map(value => value.handle) }))
  }
}
export class LanguageModel extends Model {
  async generate(prompt: ModelPrompt, options: GenerateTextOptions = {}, call?: CallOptions): Promise<GenerateTextResult> { return this.call('text.generate', { prompt, options }, call) }
  async generateObject(params: unknown, call?: CallOptions): Promise<GenerateObjectResult> { return this.call('text.generateObject', params, call) }
  async consume(params: unknown, call?: CallOptions): Promise<GenerateTextResult> { return this.call('text.consume', params, call) }
  async generateOpenAI(params: unknown, call?: CallOptions): Promise<unknown> { return this.call('text.generateOpenAI', params, call) }
  async stream(prompt: ModelPrompt, options: GenerateTextOptions = {}, call?: CallOptions): Promise<AsyncIterableIterator<StreamFrame>> { return this.client.stream(this.handle, 'text.stream', { prompt, options }, call) }
  async streamOpenAI(params: unknown, call?: CallOptions): Promise<AsyncIterableIterator<StreamFrame>> { return this.client.stream(this.handle, 'text.streamOpenAI', params, call) }
}
export class EmbeddingModel extends Model {
  async embed(params: unknown, call?: CallOptions): Promise<EmbeddingResult> { return this.call('embedding.embed', params, call) }
  async embedMany(params: EmbeddingCallOptions, call?: CallOptions): Promise<EmbeddingResult> { return this.call('embedding.embedMany', params, call) }
}
export class ImageModel extends Model { async generate(params: ImageCallOptions, call?: CallOptions): Promise<ImageResult> { return this.call('image.generate', params, call) } }
export class SpeechModel extends Model { async generate(params: SpeechCallOptions, call?: CallOptions): Promise<SpeechResult> { return this.call('speech.generate', params, call) } }
export class TranscriptionModel extends Model {
  async generate(params: TranscriptionCallOptions, call?: CallOptions): Promise<TranscriptionResult> { return this.call('transcription.generate', params, call) }
  async start(params: unknown, call?: CallOptions): Promise<Operation> { return new Operation(this.client, handleRef(await this.call('transcription.start', params, call), 'Operation')) }
}
export class RerankingModel extends Model { async rerank(params: RerankingCallOptions, call?: CallOptions): Promise<RerankingResult> { return this.call('reranking.rerank', params, call) } }
export class VideoModel extends Model { async generate(params: VideoCallOptions, call?: CallOptions): Promise<VideoResult> { return this.call('video.generate', params, call) } }
export class SearchModel extends Model { async search(params: SearchCallOptions, call?: CallOptions): Promise<SearchResult> { return this.call('search.search', params, call) } }
export class DecisionModel extends Model {
  async decide(options: DecideOptions, call?: CallOptions): Promise<DecideResult> { return this.call('decision.decide', { options }, call) }
  async capabilities(): Promise<unknown> { return this.call('decision.capabilities') }
}
export class Files extends Resource { async upload(params: UploadFileParams, call?: CallOptions): Promise<UploadFileResult> { return this.call('files.upload', params, call) } }
export class Middleware extends Resource {}

function handleRef(value: string | HandleResult, kind: HandleKind): HandleResult {
  return { handle: typeof value === 'string' ? value : value.handle, kind }
}

function model(client: Client, ref: HandleResult): Model {
  switch (ref.kind) {
    case 'Language': return new LanguageModel(client, ref)
    case 'Embedding': return new EmbeddingModel(client, ref)
    case 'Image': return new ImageModel(client, ref)
    case 'Speech': return new SpeechModel(client, ref)
    case 'Transcription': return new TranscriptionModel(client, ref)
    case 'Reranking': return new RerankingModel(client, ref)
    case 'Video': return new VideoModel(client, ref)
    case 'Search': return new SearchModel(client, ref)
    case 'Decision': return new DecisionModel(client, ref)
    default: throw protocolError(304, 'InvalidState', `Invalid model kind: ${ref.kind}`)
  }
}

export class Operation extends Resource {
  async push(data: Uint8Array): Promise<void> { await this.client.raw.operationPush(this.handle, data) }
  async inputDone(): Promise<void> { await this.call('operation.inputDone') }
  async next(timeoutMs: number = 1000): Promise<OperationPollResult> { return this.call('operation.next', { timeoutMs }) }
  async status(): Promise<unknown> { return this.call('operation.status') }
  async cancel(): Promise<unknown> { return this.call('operation.cancel') }
  events(): AsyncIterableIterator<TranscriptionStreamPart> {
    const operation = this
    let ended = false
    return {
      async next() {
        if (ended) return { done: true, value: undefined }
        try {
          for (;;) {
            const next = await operation.next()
            if (ended) return { done: true, value: undefined }
            if (next.type === 'part') return { done: false, value: next.part }
            if (next.type === 'ended') { ended = true; return { done: true, value: undefined } }
          }
        } catch (error) { if (!(error instanceof AimuxError && error.code === 303)) ended = true; throw error }
      },
      async return() { ended = true; await operation.cancel(); return { done: true, value: undefined } },
      async throw(error?: unknown) { ended = true; await operation.cancel(); throw error },
      [Symbol.asyncIterator]() { return this },
    }
  }
}

export class TraceStore extends Resource {
  async aggregate(filter: TraceAggregateParams['filter'] = {}): Promise<TraceStats[]> { return this.call('trace.aggregate', { filter }) }
  async sessionChain(sessionId: string): Promise<SessionChainView> { return this.call('trace.sessionChain', { sessionId }) }
  async sessionTrajectory(sessionId: string): Promise<unknown> { return this.call('trace.sessionTrajectory', { sessionId }) }
  async export(): Promise<unknown> { return this.call('trace.export') }
  async clear(): Promise<void> { await this.call('trace.clear') }
}
export class Recorder extends Resource {
  async attach(): Promise<unknown> { return this.call('recorder.attach') }
  async stop(): Promise<unknown> { return this.call('recorder.stop') }
  async flush(): Promise<unknown> { return this.call('recorder.flush') }
  async stats(): Promise<unknown> { return this.call('recorder.stats') }
  async export(): Promise<unknown> { return this.call('recorder.export') }
}
export class SessionStore extends Resource {
  async attach(): Promise<unknown> { return this.call('sessionStore.attach') }
  async list(params: unknown = {}): Promise<unknown> { return this.call('sessionStore.list', params) }
  async calls(sessionId: string): Promise<unknown> { return this.call('sessionStore.calls', { sessionId }) }
}
export class ReplaySession extends Resource {
  async capabilities(): Promise<unknown> { return this.call('replay.capabilities') }
  async inspect(): Promise<unknown> { return this.call('replay.inspect') }
  async dryRun(params: unknown = {}): Promise<unknown> { return this.call('replay.dryRun', params) }
  async live(params: unknown): Promise<unknown> { return this.call('replay.live', params) }
}

function mapIterator<A, B>(inner: AsyncIterableIterator<A>, map: (value: A) => B, cancel: () => void): AsyncIterableIterator<B> {
  let ended = false
  return {
    async next() {
      if (ended) return { done: true, value: undefined }
      try {
        const result = await inner.next()
        if (ended || result.done) { ended = true; return { done: true, value: undefined } }
        return { done: false, value: map(result.value) }
      } catch (error) {
        if (error instanceof AimuxError && error.code === 303) throw error
        ended = true; cancel(); if (inner.return) await inner.return().catch(() => {}); throw error
      }
    },
    async return() { ended = true; cancel(); if (inner.return) await inner.return(); return { done: true, value: undefined } },
    async throw(error?: unknown) { ended = true; cancel(); if (inner.return) await inner.return(); throw error },
    [Symbol.asyncIterator]() { return this },
  }
}

/** A pull source with zero prefetch; reader.cancel() immediately invokes return(). */
export function readable<T>(iterator: AsyncIterableIterator<T>): ReadableStream<T> {
  return new ReadableStream<T>({
    async pull(controller) {
      try {
        const result = await iterator.next()
        if (result.done) controller.close()
        else controller.enqueue(result.value)
      } catch (error) { controller.error(error); if (iterator.return) await iterator.return().catch(() => {}) }
    },
    async cancel() { if (iterator.return) await iterator.return() },
  }, { highWaterMark: 0 })
}
