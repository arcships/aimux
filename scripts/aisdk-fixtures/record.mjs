#!/usr/bin/env node
// Protocol fixture recorder: runs the official Vercel AI SDK packages against a mock
// `fetch` and writes what was sent / returned to fixtures/aisdk/<package>/<case>.json.
// See README.md. Usage: `npm ci && node record.mjs [package[/case] ...]`.
import { readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { generateText, streamText, embed, embedMany, generateImage } from 'ai';
import { createMockFetch, lowerSortedHeaders } from './mock-fetch.mjs';
import { FAKE_KEYS, FIXED_NOW_MS } from './common.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(here, '../../fixtures/aisdk');

const PACKAGES = [
  { dir: 'openai', pkg: '@ai-sdk/openai' },
  { dir: 'openai-compatible', pkg: '@ai-sdk/openai-compatible' },
  { dir: 'anthropic', pkg: '@ai-sdk/anthropic' },
  { dir: 'google', pkg: '@ai-sdk/google' },
];
const VERSIONED = ['ai', '@ai-sdk/provider', '@ai-sdk/provider-utils', ...PACKAGES.map((p) => p.pkg)];

function installedVersion(pkg) {
  return JSON.parse(readFileSync(join(here, 'node_modules', pkg, 'package.json'), 'utf8')).version;
}

// ---------------------------------------------------------------- determinism
const RealDate = Date;
class FixedDate extends RealDate {
  constructor(...args) {
    if (args.length === 0) super(FIXED_NOW_MS);
    else super(...args);
  }
  static now() {
    return FIXED_NOW_MS;
  }
}
globalThis.Date = FixedDate;

// The provider-utils user-agent embeds `runtime/<navigator.userAgent>`; pin it so the
// fixtures do not depend on the Node major version of whoever regenerates them.
Object.defineProperty(globalThis, 'navigator', { value: { userAgent: 'Node.js/22' }, configurable: true, writable: true });

function seedRandom(seed = 0x2f6e2b1) {
  let a = seed >>> 0;
  Math.random = () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function makeIdGenerator() {
  let n = 0;
  return () => `id-${++n}`;
}

// ---------------------------------------------------------------- serialization
function sortKeys(v) {
  if (Array.isArray(v)) return v.map(sortKeys);
  if (v && typeof v === 'object') {
    return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sortKeys(v[k])]));
  }
  return v;
}

function replacer(_key, value) {
  if (value instanceof Error) return { name: value.name, message: value.message };
  if (value instanceof Uint8Array) return { uint8ArrayLength: value.length };
  if (typeof value === 'bigint') return value.toString();
  // AI SDK tool objects: keep description + the JSON schema, drop functions/symbols.
  if (value && typeof value === 'object' && 'inputSchema' in value && !Array.isArray(value)) {
    const s = value.inputSchema;
    return { description: value.description, inputSchema: s && typeof s === 'object' && 'jsonSchema' in s ? s.jsonSchema : s };
  }
  return value;
}

/** JSON-roundtrip with SDK-object handling, then key-sorted. */
function toPlain(v) {
  const s = JSON.stringify(v, replacer);
  return s === undefined ? undefined : sortKeys(JSON.parse(s));
}

function redactHeaders(headers) {
  const secretNames = new Set(['authorization', 'x-api-key', 'api-key', 'x-goog-api-key']);
  const out = {};
  for (const [k, v] of Object.entries(headers)) {
    const name = k.toLowerCase();
    const leaks = FAKE_KEYS.some((key) => key !== '' && v.includes(key));
    out[name] = secretNames.has(name) || leaks ? '<redacted>' : v;
  }
  return sortKeys(out);
}

// ---------------------------------------------------------------- SDK operations
function generateTextResult(r) {
  return {
    content: r.content,
    finishReason: r.finishReason,
    providerMetadata: r.providerMetadata,
    rawFinishReason: r.rawFinishReason,
    response: { headers: r.response?.headers, id: r.response?.id, modelId: r.response?.modelId },
    text: r.text,
    usage: r.usage,
    warnings: r.warnings,
  };
}

async function runOperation(operation, model, input, internal) {
  switch (operation) {
    case 'generateText':
      return generateTextResult(await generateText({ model, ...input, _internal: internal }));
    case 'streamText': {
      const r = streamText({ model, ...input, _internal: internal, onError: () => {} });
      const parts = [];
      for await (const p of r.fullStream) parts.push(p);
      return { finishReason: await r.finishReason, parts, usage: await r.usage };
    }
    case 'embed': {
      const r = await embed({ model, ...input });
      return { embeddings: [r.embedding], providerMetadata: r.providerMetadata, usage: r.usage };
    }
    case 'embedMany': {
      const r = await embedMany({ model, ...input });
      return { embeddings: r.embeddings, providerMetadata: r.providerMetadata, usage: r.usage };
    }
    case 'generateImage': {
      const r = await generateImage({ model, ...input });
      return {
        images: r.images.map((i) => ({ base64Length: i.base64?.length ?? 0, mediaType: i.mediaType })),
        warnings: r.warnings,
      };
    }
    default:
      throw new Error(`unknown operation ${operation}`);
  }
}

// ---------------------------------------------------------------- one case
const SCRUB_ENV = /^(OPENAI|ANTHROPIC|GOOGLE|GROQ|AI_GATEWAY|VERCEL)_/;

async function recordCase({ pkg, version, c }) {
  seedRandom();
  const generateId = makeIdGenerator();
  const internal = { generateId, generateCallId: generateId, now: () => FIXED_NOW_MS };

  // Hermetic environment: no ambient provider env vars, plus whatever the case sets.
  const savedEnv = { ...process.env };
  for (const k of Object.keys(process.env)) if (SCRUB_ENV.test(k)) delete process.env[k];
  for (const [k, v] of Object.entries(c.env ?? {})) {
    if (v === undefined) delete process.env[k];
    else process.env[k] = v;
  }

  const mock = createMockFetch(c.response);
  const ctx = { operation: undefined, input: undefined };
  let model;
  let result;
  let error;
  try {
    model = c.build(mock);
    const run = (operation, input) => {
      ctx.operation = operation;
      ctx.input = toPlain(input);
      return runOperation(operation, model, input, internal);
    };
    try {
      result = toPlain(await c.call(model, run));
    } catch (e) {
      error = e;
    }
  } finally {
    for (const k of Object.keys(process.env)) if (!(k in savedEnv)) delete process.env[k];
    Object.assign(process.env, savedEnv);
  }

  if (c.expectError && !error) throw new Error(`${pkg}/${c.name}: expected an error, got a result`);
  if (!c.expectError && error) throw new Error(`${pkg}/${c.name}: unexpected error: ${error.stack ?? error}`);
  const expected = c.expectRequests ?? 1;
  if (mock.requests.length !== expected) {
    throw new Error(`${pkg}/${c.name}: expected ${expected} request(s), mock saw ${mock.requests.length}`);
  }

  const raw = mock.requests[0];
  const observations = c.observe ? toPlain(c.observe({ request: raw, requests: mock.requests, result, error, model })) : undefined;

  const fixture = {
    package: pkg,
    version,
    case: c.name,
    sdk: { operation: ctx.operation, input: ctx.input },
    model: { modelId: model?.modelId, provider: model?.provider },
    request: raw ? { body: raw.body, headers: redactHeaders(raw.headers), method: raw.method, url: raw.url } : null,
    response: raw ? { body: c.response.body, headers: lowerSortedHeaders(Object.entries(raw.responseHeaders)), status: raw.responseStatus } : null,
    ...(error ? { error: { message: error.message, name: error.name } } : { result }),
    ...(observations ? { observations } : {}),
  };
  const text = JSON.stringify(sortKeys(JSON.parse(JSON.stringify(fixture, replacer))), null, 2) + '\n';
  for (const key of FAKE_KEYS) {
    if (key && text.includes(key)) throw new Error(`${pkg}/${c.name}: API key "${key}" leaked into fixture`);
  }
  return text;
}

// ---------------------------------------------------------------- main
const filters = process.argv.slice(2);
const wanted = (dir, name) => filters.length === 0 || filters.some((f) => f === dir || f === `${dir}/${name}`);

const skipped = [];
let recorded = 0;
for (const { dir, pkg } of PACKAGES) {
  const cases = (await import(`./cases/${dir}.mjs`)).default;
  const version = installedVersion(pkg);
  const pkgOut = join(OUT, dir);
  mkdirSync(pkgOut, { recursive: true });
  const names = new Set();
  for (const c of cases) {
    if (names.has(c.name)) throw new Error(`duplicate case ${dir}/${c.name}`);
    names.add(c.name);
    if (!wanted(dir, c.name)) continue;
    if (c.skip) {
      skipped.push({ dir, pkg, name: c.name, reason: c.skip });
      console.log(`skip   ${dir}/${c.name}: ${c.skip}`);
      continue;
    }
    const text = await recordCase({ pkg, version, c });
    writeFileSync(join(pkgOut, `${c.name}.json`), text);
    recorded++;
    console.log(`record ${dir}/${c.name}`);
  }
  if (filters.length === 0) {
    // Drop fixtures of cases that no longer exist.
    for (const f of readdirSync(pkgOut)) {
      if (f.endsWith('.json') && !names.has(f.slice(0, -5))) rmSync(join(pkgOut, f));
    }
  }
}

if (filters.length === 0) {
  const versions = Object.fromEntries(VERSIONED.map((p) => [p, installedVersion(p)]));
  writeFileSync(join(OUT, 'VERSIONS.json'), JSON.stringify(sortKeys(versions), null, 2) + '\n');
  const skippedPath = join(OUT, 'SKIPPED.md');
  if (skipped.length) {
    const lines = ['# Skipped cases', '', 'Generated by `scripts/aisdk-fixtures/record.mjs`. These cases could not be recorded against the pinned SDK versions.', ''];
    for (const s of skipped) lines.push(`- \`${s.dir}/${s.name}\` (${s.pkg}): ${s.reason}`);
    writeFileSync(skippedPath, lines.join('\n') + '\n');
  } else if (existsSync(skippedPath)) rmSync(skippedPath);
}
console.log(`done: ${recorded} recorded, ${skipped.length} skipped`);
