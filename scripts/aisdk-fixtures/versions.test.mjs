// node --test scripts/aisdk-fixtures/versions.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkVersions, lockIntegrity, readJson, readVersions, versionsDocument } from './versions.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const pinnedPath = join(here, '..', '..', 'fixtures', 'aisdk', 'VERSIONS.json');
const lockPath = join(here, 'package-lock.json');
const pinnedText = readFileSync(pinnedPath, 'utf8');
const pinned = JSON.parse(pinnedText);
const installed = Object.fromEntries(Object.entries(pinned).map(([p, e]) => [p, e.version]));

/** A well-formed sha512 SRI that no tarball has. */
const fakeSri = (c) => `sha512-${c.repeat(86)}==`;

test('every VERSIONS.json entry is the {version, integrity} object form', () => {
  assert.deepEqual(readVersions(pinnedPath), pinned);
});

test('the object form is the only form', () => {
  const { version, integrity } = pinned.ai;
  const cases = {
    'bare version string': { ai: version },
    'no integrity': { ai: { version } },
    'sha1 integrity': { ai: { version, integrity: 'sha1-2jmj7l5rSw0yVb/vlWAYkK/YBwk=' } },
    'version range': { ai: { version: `^${version}`, integrity } },
    'path in the package name': { '../ai': { version, integrity } },
    'one bad entry among good ones': { ...pinned, zod: '4.0.0' },
    'no packages': {},
    'array': [],
  };
  for (const [name, doc] of Object.entries(cases)) {
    assert.throws(() => checkVersions(doc), /VERSIONS\.json: /, name);
  }
});

test('rewriting the pinned file preserves every integrity, byte for byte', () => {
  const doc = versionsDocument(installed, pinned, {});
  assert.deepEqual(doc, pinned);
  assert.equal(JSON.stringify(doc, null, 2) + '\n', pinnedText);
});

test('the lockfile supplies the same integrity when the previous file has no entry', () => {
  // A lockfileVersion 3 document as npm writes it: the root package under "",
  // dependencies under node_modules/, nested copies further down, links
  // without integrity.
  const lock = { lockfileVersion: 3, packages: { '': { name: 'aimux-aisdk-fixtures', dependencies: installed } } };
  for (const [pkg, { version, integrity }] of Object.entries(pinned)) {
    lock.packages[`node_modules/${pkg}`] = { version, integrity };
  }
  lock.packages['node_modules/ai/node_modules/@ai-sdk/provider'] = { version: '0.0.1', integrity: fakeSri('C') };
  lock.packages['node_modules/linked'] = { resolved: '../linked', link: true };
  assert.deepEqual(versionsDocument(installed, {}, lockIntegrity(lock)), pinned);
});

test(
  'the recorder package-lock.json pins the same integrity',
  { skip: !existsSync(lockPath) && 'scripts/aisdk-fixtures/package-lock.json does not exist yet' },
  () => {
    const locked = lockIntegrity(readJson(lockPath, {}));
    assert.deepEqual(versionsDocument(installed, {}, locked), pinned);
  },
);

test('a version bump never keeps the stale pin: the lock supplies it, or writing fails', () => {
  const bumped = { ...installed, ai: '9.9.9' };
  assert.throws(() => versionsDocument(bumped, pinned, {}), /ai@9\.9\.9: no integrity/);
  const locked = { ai: { version: '9.9.9', integrity: fakeSri('A') } };
  assert.deepEqual(versionsDocument(bumped, pinned, locked).ai, { version: '9.9.9', integrity: fakeSri('A') });
});

test('the previous file and the lock must agree for the same version', () => {
  const locked = { ai: { version: installed.ai, integrity: fakeSri('B') } };
  assert.throws(() => versionsDocument(installed, pinned, locked), /ai@[^:]+: VERSIONS\.json pins /);
});
