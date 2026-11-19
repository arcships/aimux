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

test('DTO vectors reject strict unknown keys, bad enums, non-objects, and invalid handles', () => {
  assert.throws(() => roundTrip('OperationNextParams', '{"timeoutMs":50,"extra":true}'), /Unknown field extra/)
  assert.throws(() => roundTrip('OperationNextParams', '"not-an-object"'), /Expected object/)
  assert.throws(() => roundTrip('FinishReasonUnified', '"future-reason"'), /Invalid FinishReasonUnified/)
  assert.throws(() => roundTrip('HandleId', '"18446744073709551616"'), /Expected u64 string/)
  assert.throws(() => roundTrip('HandleId', '123'), /Expected u64 string/)
  assert.throws(() => roundTrip('OperationNextParams', '{"timeoutMs":-1}'), /Expected integer/)
  assert.throws(() => roundTrip('BackendConfig', '{"type":"ring","capacity":1,"extra":true}'), /Unknown field extra/)
  assert.throws(() => roundTrip('StreamFrame', '{"type":"end","seq":"0","outcome":"success","extra":true}'), /Unknown field extra/)
  assert.equal(roundTrip('HandleId', '"0001"'), '0001')
  assert.deepEqual(roundTrip('BackendConfig', '{"type":"ring","capacity":1}'), { type: 'ring', capacity: 1 })
})
