import * as ops from './generated/ops.ts'
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

  async provider(params: ProviderCreateParams): Promise<Provider> { return new Provider(this, await this.call('0', ops.OP_PROVIDER_CREATE, params)) }
  async defaultProvider(params: unknown): Promise<Provider> { return new Provider(this, await this.call('0', ops.OP_PROVIDER_DEFAULT, params)) }
  async customProvider(params: unknown): Promise<Provider> { return new Provider(this, await this.call('0', ops.OP_PROVIDER_CUSTOM, params)) }
  async registry(entries: Record<string, Provider>, namespace = ''): Promise<Registry> {
    const refs = Object.entries(entries).map(([key, provider]) => ({ key, provider: provider.handle }))
    return new Registry(this, await this.call('0', ops.OP_REGISTRY_CREATE, { namespace, entries: refs }))
  }
  abort(signal?: AbortSignal): Abort { return new Abort(this, signal) }
  async configure(params: unknown): Promise<unknown> { return this.call('0', ops.OP_RUNTIME_CONFIGURE, params) }
  async describe(): Promise<unknown> { return this.call('0', ops.OP_RUNTIME_DESCRIBE) }
  async trace(params: unknown = {}): Promise<TraceStore> { return new TraceStore(this, handleRef(await this.call('0', ops.OP_TRACE_CREATE, params), 'TraceStore')) }
  async recorder(params: unknown): Promise<Recorder> { return new Recorder(this, handleRef(await this.call('0', ops.OP_RECORDER_CREATE, params), 'Recorder')) }
  async sessionStore(params: SessionStoreCreateParams): Promise<SessionStore> { return new SessionStore(this, handleRef(await this.call('0', ops.OP_SESSION_STORE_CREATE, params), 'SessionStore')) }
  async replay(params: unknown): Promise<ReplaySession> { return new ReplaySession(this, handleRef(await this.call('0', ops.OP_REPLAY_CREATE, params), 'ReplaySession')) }
  async middleware(params: unknown): Promise<Middleware> { return new Middleware(this, await this.call('0', ops.OP_MIDDLEWARE_CREATE, params)) }
  async transport(params: unknown): Promise<Resource> { return new Resource(this, await this.call('0', ops.OP_TRANSPORT_CREATE, params)) }
  async credential(params: unknown): Promise<Resource> { return new Resource(this, await this.call('0', ops.OP_CREDENTIAL_CREATE, params)) }
  async catalogue(params: unknown): Promise<unknown> { return this.call('0', ops.OP_CATALOGUE_FETCH, params) }
  async router(params: unknown): Promise<LanguageModel> { return new LanguageModel(this, handleRef(await this.call('0', ops.OP_COMPOSITE_ROUTER, params), 'Language')) }
  async moa(params: unknown): Promise<LanguageModel> { return new LanguageModel(this, handleRef(await this.call('0', ops.OP_COMPOSITE_MOA, params), 'Language')) }
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
    return model(this.client, await this.call(ops.OP_PROVIDER_MODEL, { modelId, ...(method ? { method } : {}) }))
  }
  async invoke<T = unknown>(method: string, call?: CallOptions): Promise<T> { return this.call(ops.OP_PROVIDER_INVOKE, { method }, call) }
  async files(): Promise<Files> { return new Files(this.client, handleRef(await this.call(ops.OP_PROVIDER_FILES), 'Files')) }
  async wrap(language: Middleware[] = [], image: Middleware[] = []): Promise<Provider> {
    return new Provider(this.client, await this.call(ops.OP_PROVIDER_WRAP, { languageModelMiddleware: language.map(value => value.handle), imageModelMiddleware: image.map(value => value.handle) }))
  }
}
export class Registry extends Resource {
  async provider(key: string): Promise<Provider> { return new Provider(this.client, await this.call(ops.OP_REGISTRY_PROVIDER, { key })) }
  async resolve(modelRef: ModelRef): Promise<Model> { return model(this.client, await this.call(ops.OP_MODEL_RESOLVE, { modelRef })) }
}
export class Model extends Resource {
  async info(): Promise<ModelInfo> { return this.call(ops.OP_MODEL_INFO) }
  async wrap(middleware: Middleware[]): Promise<Model> {
    return model(this.client, await this.call(ops.OP_MODEL_WRAP, { middleware: middleware.map(value => value.handle) }))
  }
}
export class LanguageModel extends Model {
  async generate(prompt: ModelPrompt, options: GenerateTextOptions = {}, call?: CallOptions): Promise<GenerateTextResult> { return this.call(ops.OP_TEXT_GENERATE, { prompt, options }, call) }
  async generateObject(params: unknown, call?: CallOptions): Promise<GenerateObjectResult> { return this.call(ops.OP_TEXT_GENERATE_OBJECT, params, call) }
  async consume(params: unknown, call?: CallOptions): Promise<GenerateTextResult> { return this.call(ops.OP_TEXT_CONSUME, params, call) }
  async generateOpenAI(params: unknown, call?: CallOptions): Promise<unknown> { return this.call(ops.OP_TEXT_GENERATE_OPEN_AI, params, call) }
  async stream(prompt: ModelPrompt, options: GenerateTextOptions = {}, call?: CallOptions): Promise<AsyncIterableIterator<StreamFrame>> { return this.client.stream(this.handle, ops.OP_TEXT_STREAM, { prompt, options }, call) }
  async streamOpenAI(params: unknown, call?: CallOptions): Promise<AsyncIterableIterator<StreamFrame>> { return this.client.stream(this.handle, ops.OP_TEXT_STREAM_OPEN_AI, params, call) }
}
export class EmbeddingModel extends Model {
  async embed(params: unknown, call?: CallOptions): Promise<EmbeddingResult> { return this.call(ops.OP_EMBEDDING_EMBED, params, call) }
  async embedMany(params: EmbeddingCallOptions, call?: CallOptions): Promise<EmbeddingResult> { return this.call(ops.OP_EMBEDDING_EMBED_MANY, params, call) }
}
export class ImageModel extends Model { async generate(params: ImageCallOptions, call?: CallOptions): Promise<ImageResult> { return this.call(ops.OP_IMAGE_GENERATE, params, call) } }
export class SpeechModel extends Model { async generate(params: SpeechCallOptions, call?: CallOptions): Promise<SpeechResult> { return this.call(ops.OP_SPEECH_GENERATE, params, call) } }
export class TranscriptionModel extends Model {
  async generate(params: TranscriptionCallOptions, call?: CallOptions): Promise<TranscriptionResult> { return this.call(ops.OP_TRANSCRIPTION_GENERATE, params, call) }
  async start(params: unknown, call?: CallOptions): Promise<Operation> { return new Operation(this.client, handleRef(await this.call(ops.OP_TRANSCRIPTION_START, params, call), 'Operation')) }
}
export class RerankingModel extends Model { async rerank(params: RerankingCallOptions, call?: CallOptions): Promise<RerankingResult> { return this.call(ops.OP_RERANKING_RERANK, params, call) } }
export class VideoModel extends Model { async generate(params: VideoCallOptions, call?: CallOptions): Promise<VideoResult> { return this.call(ops.OP_VIDEO_GENERATE, params, call) } }
export class SearchModel extends Model { async search(params: SearchCallOptions, call?: CallOptions): Promise<SearchResult> { return this.call(ops.OP_SEARCH_SEARCH, params, call) } }
export class DecisionModel extends Model {
  async decide(options: DecideOptions, call?: CallOptions): Promise<DecideResult> { return this.call(ops.OP_DECISION_DECIDE, { options }, call) }
  async capabilities(): Promise<unknown> { return this.call(ops.OP_DECISION_CAPABILITIES) }
}
export class Files extends Resource { async upload(params: UploadFileParams, call?: CallOptions): Promise<UploadFileResult> { return this.call(ops.OP_FILES_UPLOAD, params, call) } }
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
  async inputDone(): Promise<void> { await this.call(ops.OP_OPERATION_INPUT_DONE) }
  async next(timeoutMs: number = 1000): Promise<OperationPollResult> { return this.call(ops.OP_OPERATION_NEXT, { timeoutMs }) }
  async status(): Promise<unknown> { return this.call(ops.OP_OPERATION_STATUS) }
  async cancel(): Promise<unknown> { return this.call(ops.OP_OPERATION_CANCEL) }
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
  async aggregate(filter: TraceAggregateParams['filter'] = {}): Promise<TraceStats[]> { return this.call(ops.OP_TRACE_AGGREGATE, { filter }) }
  async sessionChain(sessionId: string): Promise<SessionChainView> { return this.call(ops.OP_TRACE_SESSION_CHAIN, { sessionId }) }
  async sessionTrajectory(sessionId: string): Promise<unknown> { return this.call(ops.OP_TRACE_SESSION_TRAJECTORY, { sessionId }) }
  async export(): Promise<unknown> { return this.call(ops.OP_TRACE_EXPORT) }
  async clear(): Promise<void> { await this.call(ops.OP_TRACE_CLEAR) }
}
export class Recorder extends Resource {
  async attach(): Promise<unknown> { return this.call(ops.OP_RECORDER_ATTACH) }
  async stop(): Promise<unknown> { return this.call(ops.OP_RECORDER_STOP) }
  async flush(): Promise<unknown> { return this.call(ops.OP_RECORDER_FLUSH) }
  async stats(): Promise<unknown> { return this.call(ops.OP_RECORDER_STATS) }
  async export(): Promise<unknown> { return this.call(ops.OP_RECORDER_EXPORT) }
}
export class SessionStore extends Resource {
  async attach(): Promise<unknown> { return this.call(ops.OP_SESSION_STORE_ATTACH) }
  async list(params: unknown = {}): Promise<unknown> { return this.call(ops.OP_SESSION_STORE_LIST, params) }
  async calls(sessionId: string): Promise<unknown> { return this.call(ops.OP_SESSION_STORE_CALLS, { sessionId }) }
}
export class ReplaySession extends Resource {
  async capabilities(): Promise<unknown> { return this.call(ops.OP_REPLAY_CAPABILITIES) }
  async inspect(): Promise<unknown> { return this.call(ops.OP_REPLAY_INSPECT) }
  async dryRun(params: unknown = {}): Promise<unknown> { return this.call(ops.OP_REPLAY_DRY_RUN, params) }
  async live(params: unknown): Promise<unknown> { return this.call(ops.OP_REPLAY_LIVE, params) }
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
