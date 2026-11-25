import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { NonIntegerNumber, dtoTags, dtoTypeNames, validateDto } from '../src/generated/dto_vectors.ts'
import type { DtoTypes } from '../src/generated/dto_vectors.ts'

function isDtoType(type: string): type is keyof DtoTypes {
  return (dtoTypeNames as readonly string[]).includes(type)
}

function parseExactInteger(source: string): bigint | null {
  const match = /^(-?)([0-9]+)(?:\.([0-9]+))?(?:[eE]([+-]?[0-9]+))?$/.exec(source)
  if (!match) return null
  const frac = match[3] ?? ''
  const digits = match[2] + frac
  if (/^0+$/.test(digits)) return 0n
  const exp = match[4] ? Number(match[4]) : 0
  if (!Number.isSafeInteger(exp)) return null
  const shift = exp - frac.length
  if (shift < 0) {
    const cut = Math.max(0, digits.length + shift)
    if (!/^0*$/.test(digits.slice(cut))) return null
    const value = BigInt(digits.slice(0, cut) || '0')
    return match[1] === '-' ? -value : value
  }
  if (shift > 30) return null
  const value = BigInt(digits) * (10n ** BigInt(shift))
  return match[1] === '-' ? -value : value
}

function parseForValidation(json: string): unknown {
  return JSON.parse(json, (_key: string, value: unknown, context?: { source?: string }) => {
    if (typeof value === 'number' && Number.isFinite(value) && typeof context?.source === 'string') {
      const exact = parseExactInteger(context.source)
      return exact !== null ? exact : new NonIntegerNumber(value)
    }
    return value
  })
}

export function roundTrip<K extends keyof DtoTypes>(type: K, json: string): DtoTypes[K] {
  const exact = parseForValidation(json)
  const tagged = dtoTags[type]
  if (tagged) {
    const tag: unknown = exact !== null && typeof exact === 'object'
      ? (exact as Record<string, unknown>)[tagged.tag] : undefined
    if (typeof tag !== 'string' || !tagged.values.includes(tag)) {
      throw new Error(`Unknown ${type} tag: ${String(tag)}`)
    }
  }
  validateDto(type, exact)
  const value = JSON.parse(json) as DtoTypes[K]
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
