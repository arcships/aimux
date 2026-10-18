import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dtoOmitNull, dtoTags, dtoTypeNames } from '../src/generated/dto_vectors.ts'
import type { DtoTypes } from '../src/generated/dto_vectors.ts'

function isDtoType(type: string): type is keyof DtoTypes {
  return (dtoTypeNames as readonly string[]).includes(type)
}

export function roundTrip<K extends keyof DtoTypes>(type: K, json: string): DtoTypes[K] {
  const value = JSON.parse(json) as DtoTypes[K]
  // TypeScript declarations have no decoder; mirror Rust's omission of ordinary
  // optional nulls at the DTO root, using the generated field metadata.
  for (const key of dtoOmitNull[type] ?? []) {
    const fields = value as Record<string, unknown>
    if (fields[key] === null) delete fields[key]
  }
  const tagged = dtoTags[type]
  if (tagged) {
    const tag: unknown = value !== null && typeof value === 'object'
      ? (value as Record<string, unknown>)[tagged.tag] : undefined
    if (typeof tag !== 'string' || !tagged.values.includes(tag)) {
      throw new Error(`Unknown ${type} tag: ${String(tag)}`)
    }
  }
  return JSON.parse(JSON.stringify(value)) as DtoTypes[K]
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const directory = new URL('../../../contract-tests/dto/', import.meta.url)
  const index: Array<{ type: string; file: string }> = JSON.parse(readFileSync(new URL('index.json', directory), 'utf8'))
  for (const entry of index) {
    try {
      if (!isDtoType(entry.type)) throw new Error(`Unknown DTO type: ${entry.type}`)
      const path = new URL(entry.file, directory)
      if (!fileURLToPath(path).startsWith(fileURLToPath(directory))) throw new Error(`Invalid vector path: ${entry.file}`)
      const value = roundTrip(entry.type, readFileSync(path, 'utf8'))
      console.log(JSON.stringify({ file: entry.file, ok: true, value }))
    } catch (error) {
      console.log(JSON.stringify({ file: entry.file, ok: false, error: String(error) }))
    }
  }
}
