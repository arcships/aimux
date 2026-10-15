/* Native JSON adapter. The typed facade is exported from @arcships/aimux. */
export class Runtime {
  constructor()
  dropHandle(handle: string): void
  manifestJson(): string
  manifestDigest(): string
  call(handle: string, op: string, paramsJson: string, abort?: Abort): Promise<string>
  stream(handle: string, op: string, paramsJson: string, abort?: Abort): Promise<FrameIterator>
  operationPush(handle: string, data: Uint8Array): Promise<void>
}
export class Abort {
  constructor(runtime: Runtime, signal?: AbortSignal)
  readonly handle: string
  readonly aborted: boolean
  signal(): void
  close(): void
}
/** Yields canonical event/end frames. Pre-acceptance errors reject stream(). */
export class FrameIterator implements AsyncIterable<string> {
  [Symbol.asyncIterator](): AsyncIterableIterator<string>
}
