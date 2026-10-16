// The fixture addon contains the production adapter and real aimux-ops. Only
// the scripted EventStream and its instrumentation are enabled by a feature.
import test from 'node:test'
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { spawnSync } from 'node:child_process'
import { readdirSync, readFileSync, mkdtempSync, copyFileSync, rmSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { basename, join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import * as ops from '../src/generated/ops.ts'

const binding = fileURLToPath(new URL('..', import.meta.url))
const root = process.env.AIMUX_CONTRACT_VECTORS ?? fileURLToPath(new URL('../../../contract-tests/ops', import.meta.url))
const vectors = basename(resolve(root)) === 'scenarios' ? root : join(root, 'scenarios')
const paths = readdirSync(vectors).filter(path => path.endsWith('.json')).sort()
assert.equal(paths.length, 11, 'all shared stream scenarios must be replayed')

// Honor the build wrapper used by local/offline validation. No napi codegen is
// run here, so the production loader and generated declarations stay untouched.
const target = ({ darwin: { arm64: 'aarch64-apple-darwin', x64: 'x86_64-apple-darwin' },
  linux: { x64: 'x86_64-unknown-linux-gnu', arm64: 'aarch64-unknown-linux-gnu' },
  win32: { x64: 'x86_64-pc-windows-msvc', arm64: 'aarch64-pc-windows-msvc' } } as const)[process.platform]?.[process.arch]
assert.ok(target, `unsupported test host: ${process.platform}/${process.arch}`)
let library = process.env.AIMUX_NODE_CONTRACT_LIBRARY
if (!library) {
  const built = spawnSync(process.env.AIMUX_BUILD_COMMAND ?? 'cargo', [
    'build', '--manifest-path', join(binding, 'Cargo.toml'), '--features', 'contract-tests',
    '--target', target, '--message-format=json',
  ], { encoding: 'utf8', timeout: 1_200_000, maxBuffer: 16 * 1024 * 1024 })
  const artifacts = built.stdout.split('\n').filter(line => line.startsWith('{')).map(line => JSON.parse(line))
  const diagnostics = artifacts.filter(item => item.reason === 'compiler-message').map(item => item.message.rendered).join('')
  assert.equal(built.status, 0, `${built.error ?? ''}\n${built.stderr}\n${diagnostics}`)
  const libraries = artifacts.filter(item => item.reason === 'compiler-artifact' && item.target.name === 'aimux_node')
    .flatMap(item => item.filenames).filter((path: string) => /\.(dylib|so|dll)$/.test(path))
  assert.equal(libraries.length, 1, 'expected the native contract addon')
  library = libraries[0]
}
const folder = mkdtempSync(join(tmpdir(), 'aimux-node-vectors-'))
const addon = resolve(folder, 'contract.node')
copyFileSync(library!, addon)
const { Runtime } = createRequire(import.meta.url)(addon)
const runtime = new Runtime()
process.on('exit', () => rmSync(folder, { recursive: true, force: true }))

function normalizeError(error: Record<string, any>): object {
  assert.equal(typeof error.message, 'string')
  assert.equal(typeof error.error.name, 'string')
  return { kind: error.kind, code: error.code, error: { name: error.error.name } }
}
function normalize(observation: Record<string, any>, expected: Record<string, any>): object {
  const result = structuredClone(observation)
  if (expected.abortObserved === null) result.abortObserved = null
  if (result.preflightError) result.preflightError = normalizeError(result.preflightError)
  for (const key of ['frames', 'consumerFrames']) {
    for (const frame of result[key]) {
      if (frame.type === 'end' && frame.error) {
        if (expected[key].find((value: any) => value.type === 'end')?.error) frame.error = normalizeError(frame.error)
        else delete frame.error // Cancellation detail is optional in the vectors.
      }
    }
  }
  return result
}

for (const path of paths) {
  test(`shared native vector: ${path}`, async () => {
    const { expected, ...input } = JSON.parse(readFileSync(join(vectors, path), 'utf8'))
    const result = JSON.parse(await runtime.call('0', 'test.scenario', JSON.stringify(input)))
    assert.deepEqual(normalize(result, expected), expected)
  })
}

test('binary push rejects a full queue without waiting for capacity', async () => {
  const handle = JSON.parse(await runtime.call('0', 'test.operation', '{}'))
  const bytes = new Uint8Array([1, 2])
  await runtime.operationPush(handle, bytes)
  const pushed = runtime.operationPush(handle, new Uint8Array([3, 4]))
  let timer: ReturnType<typeof setTimeout>
  try {
    await assert.rejects(Promise.race([pushed, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('push waited for capacity')), 1000)
    })]), (error: any) => error.kind === 'protocol' && error.code === 305)
  } finally {
    clearTimeout(timer!)
    await runtime.call(handle, ops.OP_HANDLE_DROP, '{}')
  }
})

test('owner teardown releases operations while its runtime can still run tasks', async () => {
  assert.deepEqual(JSON.parse(await runtime.call('0', 'test.teardown', '{}')), { released: true, runtimeAlive: true })
})
