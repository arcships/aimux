import test from 'node:test'
import assert from 'node:assert/strict'
import { roundTrip } from './dto_vectors.ts'
import type { StreamFrame } from '../src/generated/types.ts'

test('DTO vectors reject unknown and missing discriminators with the type and tag', () => {
  assert.throws(() => roundTrip('StreamFrame', '{"type":"future"}'), /Unknown StreamFrame tag: future/)
  assert.throws(() => roundTrip('ModelMessage', '{}'), /Unknown ModelMessage tag: undefined/)
})

test('DTO vectors preserve decimal handle strings and optional nulls', () => {
  const frame: Extract<StreamFrame, { type: 'end' }> = { type: 'end', seq: '18446744073709551615', outcome: 'success', error: null }
  assert.deepEqual(roundTrip('StreamFrame', JSON.stringify(frame)), frame)
  delete frame.error
  assert.deepEqual(roundTrip('StreamFrame', JSON.stringify(frame)), frame)
  assert.equal(roundTrip('HandleId', '"18446744073709551615"'), '18446744073709551615')
})
