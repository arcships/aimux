#!/usr/bin/env node
// Lists the upstream AI SDK test cases at the versions pinned in
// fixtures/aisdk/VERSIONS.json (RFC-0045 §2).
//
// The published npm tarballs leave the tests out, so the cases come from a
// vercel/ai git clone: package `<name>` at version `<v>` is read from tag
// `<name>@<v>`, directory `packages/<name without @ai-sdk/>`. A blobless clone
// is enough (`git clone --filter=blob:none https://github.com/vercel/ai`).
//
// A test file is `*.test.ts` or `*.test.tsx` (not the `*.test-d.ts` type
// tests). A case is one `it(...)` or `test(...)` call, including `.skip`,
// `.only`, `.todo` and `.each`; an `.each` table counts once. Its id is the
// `describe` chain plus the case name, which RFC-0045 §4 uses as the marker.
//
// Usage: node scripts/aisdk-tests/inventory.mjs --repo <vercel/ai clone>
//          [--versions <file>] [--json <out.json>]

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

function parseArgs(argv) {
  const args = { versions: path.join(root, 'fixtures/aisdk/VERSIONS.json') };
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i].replace(/^--/, '');
    if (!['repo', 'versions', 'json'].includes(key) || argv[i + 1] === undefined) {
      throw new Error(`unknown or incomplete argument: ${argv[i]}`);
    }
    args[key] = argv[i + 1];
  }
  if (!args.repo) throw new Error('--repo <vercel/ai clone> is required');
  return args;
}

const git = (repo, ...args) =>
  execFileSync('git', ['-C', repo, ...args], { encoding: 'utf8', maxBuffer: 1 << 28 });

function readBlobs(repo, specs) {
  const out = execFileSync('git', ['-C', repo, 'cat-file', '--batch'], {
    input: specs.join('\n') + '\n',
    maxBuffer: 1 << 30,
  });
  const blobs = [];
  let at = 0;
  for (const spec of specs) {
    const nl = out.indexOf(10, at);
    const header = out.subarray(at, nl).toString();
    const size = Number(header.split(' ')[2]);
    if (!Number.isFinite(size)) throw new Error(`git cat-file ${spec}: ${header}`);
    blobs.push(out.subarray(nl + 1, nl + 1 + size).toString('utf8'));
    at = nl + 1 + size + 1;
  }
  return blobs;
}

const CALLEES = new Set(['describe', 'it', 'test']);
const MODIFIERS = new Set(['skip', 'only', 'todo', 'concurrent', 'each', 'sequential', 'fails']);
// A `/` after one of these starts a regular expression, otherwise it divides.
const REGEX_BEFORE = new Set(['', '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+', '-', '*', '%', '<', '>', '~', '^']);

// Returns `{ line, chain }` for every case in a TypeScript test file. A small
// lexer skips comments, strings, template literals and regular expressions,
// so brackets inside them do not move the `describe` scope.
export function listCases(src) {
  const cases = [];
  const scopes = []; // open `describe` calls: { name, depth }
  const templates = []; // bracket depth of each open `${`
  let depth = 0;
  let line = 1;
  let prev = '';
  let i = 0;

  const skipString = (quote) => {
    let text = '';
    for (i++; i < src.length && src[i] !== quote; i++) {
      if (src[i] === '\\') text += src[++i];
      else text += src[i];
      if (src[i] === '\n') line++;
    }
    i++;
    return text;
  };
  // Skips a template literal from its opening backtick (or a `}` closing a
  // substitution) and returns its raw text; stops early at a `${`.
  const skipTemplate = () => {
    let text = '';
    for (i++; i < src.length; i++) {
      const c = src[i];
      if (c === '\\') { text += src[++i]; continue; }
      if (c === '\n') line++;
      if (c === '`') { i++; return { text, open: false }; }
      if (c === '$' && src[i + 1] === '{') { i += 2; templates.push(depth); depth++; return { text, open: true }; }
      text += c;
    }
    return { text, open: false };
  };
  const skipSpace = () => {
    for (;;) {
      if (src[i] === '\n') { line++; i++; }
      else if (/\s/.test(src[i] ?? '')) i++;
      else if (src.startsWith('//', i)) while (i < src.length && src[i] !== '\n') i++;
      else if (src.startsWith('/*', i)) {
        const end = src.indexOf('*/', i + 2);
        for (const c of src.slice(i, end)) if (c === '\n') line++;
        i = end + 2;
      } else return;
    }
  };
  // Reads the first argument when it is a plain string literal.
  const readName = () => {
    skipSpace();
    const q = src[i];
    if (q === '"' || q === "'") return skipString(q);
    if (q === '`') {
      const { text, open } = skipTemplate();
      return open ? text + '${…}' : text;
    }
    return null;
  };
  const skipWholeTemplate = () => {
    const open = templates.length;
    skipTemplate();
    while (i < src.length && templates.length > open) step();
  };
  const skipBalanced = () => {
    // From an opening `(`, skip to just after its matching `)`.
    const start = depth;
    depth++;
    i++;
    while (i < src.length && depth > start) step();
  };

  function step() {
    const c = src[i];
    if (c === '\n') { line++; i++; return; }
    if (/\s/.test(c)) { i++; return; }
    if (src.startsWith('//', i) || src.startsWith('/*', i)) { skipSpace(); return; }
    if (c === '"' || c === "'") { skipString(c); prev = 'str'; return; }
    if (c === '`') { skipTemplate(); prev = 'str'; return; }
    if (c === '/' && REGEX_BEFORE.has(prev)) {
      let inClass = false;
      for (i++; i < src.length; i++) {
        if (src[i] === '\\') i++;
        else if (src[i] === '[') inClass = true;
        else if (src[i] === ']') inClass = false;
        else if (src[i] === '/' && !inClass) break;
      }
      for (i++; /[a-z]/.test(src[i] ?? ''); i++);
      prev = 'regex';
      return;
    }
    if (c === '(' || c === '[' || c === '{') { depth++; prev = c; i++; return; }
    if (c === ')' || c === ']' || c === '}') {
      if (c === '}' && templates.length && templates[templates.length - 1] === depth - 1) {
        templates.pop();
        depth--;
        const { open } = skipTemplate();
        if (!open) prev = 'str';
        return;
      }
      depth--;
      while (scopes.length && scopes[scopes.length - 1].depth > depth) scopes.pop();
      prev = c;
      i++;
      return;
    }
    if (/[A-Za-z_$]/.test(c)) {
      const m = /^[A-Za-z_$][\w$]*/.exec(src.slice(i, i + 64));
      const word = m[0];
      const afterDot = prev === '.';
      i += word.length;
      prev = word;
      if (afterDot || !CALLEES.has(word)) return;
      const at = line;
      let each = false;
      for (;;) {
        skipSpace();
        if (src[i] !== '.') break;
        i++;
        skipSpace();
        const mod = /^[A-Za-z]+/.exec(src.slice(i, i + 16))?.[0];
        if (!MODIFIERS.has(mod)) return;
        if (mod === 'each') each = true;
        i += mod.length;
      }
      skipSpace();
      if (each) {
        if (src[i] === '`') skipWholeTemplate();
        else if (src[i] === '(') skipBalanced();
        else return;
        skipSpace();
      }
      if (src[i] !== '(') return;
      depth++;
      i++;
      const callDepth = depth;
      const name = readName() ?? '<computed>';
      if (word === 'describe') scopes.push({ name, depth: callDepth });
      else cases.push({ line: at, chain: [...scopes.map((s) => s.name), name] });
      prev = 'str';
      return;
    }
    prev = c;
    i++;
  }

  while (i < src.length) step();
  return cases;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const versions = JSON.parse(fs.readFileSync(args.versions, 'utf8'));
  const report = [];
  for (const [name, { version }] of Object.entries(versions)) {
    const tag = `${name}@${version}`;
    const dir = `packages/${name.replace(/^@ai-sdk\//, '')}`;
    const files = git(args.repo, 'ls-tree', '-r', '--name-only', tag, '--', dir)
      .split('\n')
      .filter((f) => /\.test\.tsx?$/.test(f))
      .sort();
    const blobs = readBlobs(args.repo, files.map((f) => `${tag}:${f}`));
    const entries = files.map((file, k) => ({ file, cases: listCases(blobs[k]) }));
    report.push({ package: name, version, tag, files: entries });
  }

  let totalFiles = 0;
  let totalCases = 0;
  console.log('package\tversion\tfiles\tcases');
  for (const p of report) {
    const cases = p.files.reduce((n, f) => n + f.cases.length, 0);
    totalFiles += p.files.length;
    totalCases += cases;
    console.log(`${p.package}\t${p.version}\t${p.files.length}\t${cases}`);
  }
  console.log(`total\t\t${totalFiles}\t${totalCases}`);
  if (args.json) fs.writeFileSync(args.json, JSON.stringify(report, null, 1) + '\n');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
