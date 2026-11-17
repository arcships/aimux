#!/usr/bin/env node
// Checks the upstream test markers and the `tests` tables of layout.toml
// against the upstream AI SDK test cases (RFC-0045 §7).
//
//   node scripts/aisdk-tests/check.mjs --repo <vercel/ai clone> [--summary <file>]
//   node scripts/aisdk-tests/check.mjs diff --repo <vercel/ai clone> --base <git rev>
//
// The cases come from inventory.mjs: package `<name>` is read from vercel/ai
// tag `<name>@<version>`, the version locked in fixtures/aisdk/VERSIONS.json
// unless the layout.toml entry of the source file sets `version` (§3.1).
//
// `check` fails when:
// 1. a marker (`/// upstream: <package> <test file> › <describe chain>` above
//    a Rust test, §4) is malformed, names a case that does not exist at that
//    version, or names a case another marker already names; or an entry's
//    `version` is below the locked version;
// 2. a layout.toml entry breaks §3.1 or §3.2: an unknown `status`, a `mapped`
//    or `elsewhere` entry without a `tests` table, a `tests` table on any
//    other entry, a category outside §3.2, an exclusion without `reason`
//    (`registered` without `ref`, `sub-feature` without `issue`), or an
//    exclusion whose `case` does not exist or is also marked.
// 3. It prints, for every selected file, the cases ported (marked), excluded
//    by the automatic rules, excluded by hand, and not handled yet, and lists
//    each automatic exclusion. This part only reports.
//
// `diff` lists the cases of the selected files that are new at the versions in
// the working tree's VERSIONS.json compared with the ones at `--base`, for the
// PR that bumps the lock (§7).

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { readPackages } from './inventory.mjs';
import { LINE, parseToml } from './toml.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const SEP = ' › ';

// §3.1: what an entry's `status` means for its tests. `aimux` marks a Rust
// file with no upstream source (RFC-0046 §4.4).
const SELECTED = new Set(['mapped', 'elsewhere']);
const STATUSES = new Set([...SELECTED, 'missing', 'not-applicable', 'aimux']);

// §3.2: the exclusion categories and the fields each one requires.
const CATEGORIES = {
  'single-step': [],
  telemetry: [],
  'ts-only': [],
  registered: ['ref'],
  'sub-feature': ['issue'],
};

// §3.2 automatic rules, for the files of the `ai` package only. A case falls
// in a category when its `describe` chain or its body uses one of these API
// names. Changing a list means changing the table in RFC-0045 §3.2.
export const AUTO_RULES = {
  'single-step': [
    /\b(?:stopWhen|stepCountIs|isStepCount|hasToolCall|prepareStep|needsApproval)\b/,
    /\btool-approval-(?:request|response)\b/,
    // a tool with `execute`
    /\bexecute\s*(?::|\()/,
  ],
  telemetry: [
    /\btelemetry\b/,
    /\bexperimental_telemetry\b/,
    // callbacks named on…Start, on…End, on…Finish
    /\bon[A-Za-z]*(?:Start|End|Finish)\b/,
  ],
};
const AUTO_PACKAGES = new Set(['ai']);

export function autoCategory(kase) {
  const text = kase.chain.join('\n') + '\n' + (kase.body ?? '');
  for (const [category, rules] of Object.entries(AUTO_RULES)) {
    if (rules.some((r) => r.test(text))) return category;
  }
  return null;
}

// Keys every case of a file by its full chain; the n-th case with the same
// chain (n ≥ 2) is keyed `<chain> #n` (§4). Spaces at the ends of a name are
// dropped: a marker cannot keep them (`it('should extract response id
// metadata ', …)` in @ai-sdk/openai).
export function keyCases(cases) {
  const seen = new Map();
  const keyed = new Map();
  for (const kase of cases) {
    const chain = kase.chain.map((s) => s.trim()).join(SEP);
    const n = (seen.get(chain) ?? 0) + 1;
    seen.set(chain, n);
    keyed.set(n === 1 ? chain : `${chain} #${n}`, kase);
  }
  return keyed;
}

// `/// upstream: <package> <test file> › <chain>`
const MARKER = /^\s*\/\/\/\s*upstream:(.*)$/;
const MARKER_BODY = /^ (@ai-sdk\/[a-z0-9-]+|ai) (src\/\S+\.test\.tsx?) › (.+)$/;

// Returns the markers of one Rust file and the problems with them.
export function scanMarkers(rel, src) {
  const markers = [];
  const errors = [];
  const lines = src.split('\n');
  lines.forEach((text, k) => {
    const m = MARKER.exec(text);
    if (!m) return;
    const where = `${rel}:${k + 1}`;
    const body = MARKER_BODY.exec(m[1].replace(/\s+$/, ''));
    if (!body) {
      errors.push(`${where}: malformed marker, expected "/// upstream: <package> src/<file>.test.ts › <describe chain>"`);
      return;
    }
    const [, pkg, file, chain] = body;
    if (chain.split(SEP).some((s) => s.trim() === '' || s !== s.trim())) {
      errors.push(`${where}: empty or padded describe level in the marker`);
      return;
    }
    // A marker sits above a test: only doc comments and attributes between.
    let j = k + 1;
    while (j < lines.length && /^\s*(\/\/\/|#\[)/.test(lines[j])) j++;
    if (!/^\s*(pub(\(\w+\))?\s+)?(async\s+)?fn\s/.test(lines[j] ?? '')) {
      errors.push(`${where}: marker is not above a test function`);
      return;
    }
    markers.push({ where, pkg, file, chain });
  });
  return { markers, errors };
}

function parseSemver(v) {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(v);
  return m ? m.slice(1).map(Number) : null;
}
function compareSemver(a, b) {
  for (let k = 0; k < 3; k++) if (a[k] !== b[k]) return a[k] - b[k];
  return 0;
}

const testFileOf = (upstream) => upstream.replace(/\.(tsx?)$/, '.test.$1');
const testFilesOf = (upstream) => [testFileOf(upstream), testFileOf(upstream).replace(/\.ts$/, '.tsx')];

// Reads every `<crate>/layout.toml` and checks its entries (§3.1, §3.2).
export function readLayouts(layoutFiles, versions, errors) {
  const tables = [];
  for (const { rel, src } of layoutFiles) {
    let doc;
    try {
      doc = parseToml(src);
    } catch (e) {
      errors.push(`${rel}: ${e.message}`);
      continue;
    }
    for (const [key, table] of Object.entries(doc)) {
      const at = `${rel}:${table[LINE]} [${key}]`;
      if (typeof table !== 'object' || Array.isArray(table)) {
        errors.push(`${rel}: top-level key ${key} is not a vendor table`);
        continue;
      }
      if (!versions[table.package]) {
        errors.push(`${at}: package ${JSON.stringify(table.package)} is not in VERSIONS.json`);
        continue;
      }
      const locked = versions[table.package].version;
      const files = [];
      const seen = new Set();
      for (const entry of table.files ?? []) {
        const where = `${rel}:${entry[LINE]} ${entry.upstream ?? entry.rust}`;
        const e = { ...entry, where, package: table.package, exclusions: [] };
        if (!STATUSES.has(entry.status)) {
          errors.push(`${where}: unknown status ${JSON.stringify(entry.status)}`);
          continue;
        }
        if (entry.status !== 'aimux') {
          if (typeof entry.upstream !== 'string') errors.push(`${where}: no upstream path`);
          else if (seen.has(entry.upstream)) errors.push(`${where}: upstream listed twice`);
          else seen.add(entry.upstream);
        }
        e.version = locked;
        if (entry.version !== undefined) {
          const v = parseSemver(String(entry.version));
          if (!v) errors.push(`${where}: version ${JSON.stringify(entry.version)} is not x.y.z`);
          else if (compareSemver(v, parseSemver(locked)) < 0)
            errors.push(`${where}: version ${entry.version} is below the locked ${locked}`);
          else e.version = entry.version;
        }
        if (SELECTED.has(entry.status) && entry.tests === undefined)
          errors.push(`${where}: a ${entry.status} entry needs a tests table (RFC-0045 §3.2)`);
        if (!SELECTED.has(entry.status) && entry.tests !== undefined)
          errors.push(`${where}: a ${entry.status} entry has no tests to exclude`);
        for (const [category, items] of Object.entries(entry.tests ?? {})) {
          if (!CATEGORIES[category]) {
            errors.push(`${where}: unknown exclusion category ${category}`);
            continue;
          }
          if (!Array.isArray(items)) {
            errors.push(`${where}: tests.${category} is not an array`);
            continue;
          }
          for (const item of items) {
            const at = `${rel}:${item[LINE] ?? entry[LINE]} ${category}`;
            const required = ['case', 'reason', ...CATEGORIES[category]];
            const missing = required.filter((f) => item[f] === undefined || item[f] === '');
            const extra = Object.keys(item).filter((f) => !required.includes(f));
            if (missing.length) errors.push(`${at}: exclusion without ${missing.join(', ')}`);
            if (extra.length) errors.push(`${at}: unknown field ${extra.join(', ')}`);
            if (category === 'sub-feature' && item.issue !== undefined && !Number.isInteger(item.issue))
              errors.push(`${at}: issue must be an issue number`);
            if (typeof item.case === 'string') e.exclusions.push({ category, case: item.case, where: at });
          }
        }
        files.push(e);
      }
      tables.push({ key, package: table.package, files });
    }
  }
  return tables;
}

// Checks the markers and exclusions against the upstream cases, and returns
// the progress of every selected test file.
export function check({ versions, tables, markers, inventory }) {
  const errors = [];
  const warnings = [];
  const cases = new Map(); // `${package}@${version}` → Map(test file → Map(key → case))
  for (const p of inventory) {
    const dir = `packages/${p.package.replace(/^@ai-sdk\//, '')}/`;
    cases.set(`${p.package}@${p.version}`,
      new Map(p.files.map((f) => [f.file.slice(dir.length), keyCases(f.cases)])));
  }
  // Owner entry of a test file: the source file it is named after (§3.1).
  const owners = new Map(); // `${package} ${test file}` → entry
  for (const t of tables)
    for (const e of t.files)
      if (e.upstream) for (const f of testFilesOf(e.upstream)) owners.set(`${t.package} ${f}`, e);
  const versionOf = (pkg, file) => owners.get(`${pkg} ${file}`)?.version ?? versions[pkg]?.version;

  // Handled cases: `${package}@${version} ${file} ${key}` → how.
  const handled = new Map();

  for (const m of markers) {
    if (!versions[m.pkg]) {
      errors.push(`${m.where}: package ${m.pkg} is not in VERSIONS.json`);
      continue;
    }
    const version = versionOf(m.pkg, m.file);
    const file = cases.get(`${m.pkg}@${version}`)?.get(m.file);
    if (!file) {
      errors.push(`${m.where}: no test file ${m.file} in ${m.pkg}@${version}`);
      continue;
    }
    if (!file.has(m.chain)) {
      errors.push(`${m.where}: no case "${m.chain}" in ${m.pkg}@${version} ${m.file}`);
      continue;
    }
    const id = `${m.pkg}@${version} ${m.file} ${m.chain}`;
    const prior = handled.get(id);
    if (prior) {
      errors.push(`${m.where}: case already marked at ${prior.where}`);
      continue;
    }
    handled.set(id, { how: 'ported', where: m.where });
    const owner = owners.get(`${m.pkg} ${m.file}`);
    if (owner && !SELECTED.has(owner.status))
      warnings.push(`${m.where}: marks a case of ${m.file}, whose source entry is ${owner.status}`);
  }

  for (const t of tables) {
    const files = cases.get(`${t.package}@${versions[t.package].version}`);
    // Test files with no source file of their own name: their exclusions sit
    // under the entry of the source file they test.
    const orphans = [...(files?.keys() ?? [])].filter((f) => !owners.has(`${t.package} ${f}`));
    for (const e of t.files) {
      if (!e.exclusions.length) continue;
      const pkgCases = cases.get(`${t.package}@${e.version}`);
      const own = testFilesOf(e.upstream).filter((f) => pkgCases?.has(f));
      for (const x of e.exclusions) {
        const hits = [...own, ...orphans].filter((f) => pkgCases?.get(f)?.has(x.case));
        // A case in the entry's own test file wins over the same chain elsewhere.
        const inOwn = hits.filter((f) => own.includes(f));
        const chosen = inOwn.length ? inOwn : hits;
        if (!chosen.length) {
          errors.push(`${x.where}: no case "${x.case}" in ${own[0] ?? testFileOf(e.upstream)} or the test files with no source of their own name`);
          continue;
        }
        if (chosen.length > 1) {
          errors.push(`${x.where}: "${x.case}" is a case of ${chosen.join(', ')}; ambiguous`);
          continue;
        }
        const id = `${t.package}@${e.version} ${chosen[0]} ${x.case}`;
        const prior = handled.get(id);
        if (prior) {
          errors.push(prior.how === 'ported'
            ? `${x.where}: "${x.case}" is excluded but marked at ${prior.where}`
            : `${x.where}: "${x.case}" is already excluded at ${prior.where}`);
          continue;
        }
        handled.set(id, { how: x.category, where: x.where });
      }
    }
  }

  // Progress (§7.3). A test file is selected when its source entry is mapped
  // or elsewhere, or, with no source of its own name, when a marker or an
  // exclusion points into it.
  const progress = [];
  const untracked = [];
  for (const t of tables) {
    const locked = versions[t.package].version;
    const rows = [];
    const versionsUsed = new Set([locked, ...t.files.map((e) => e.version)]);
    for (const version of versionsUsed) {
      const files = cases.get(`${t.package}@${version}`);
      if (!files) continue;
      for (const [file, keyed] of files) {
        const owner = owners.get(`${t.package} ${file}`);
        if ((owner?.version ?? locked) !== version) continue;
        const prefix = `${t.package}@${version} ${file} `;
        const touched = [...keyed.keys()].some((k) => handled.has(prefix + k));
        if (owner ? !SELECTED.has(owner.status) : !touched) {
          if (!owner) untracked.push(`${t.package} ${file}`);
          continue;
        }
        const row = { file, version, total: keyed.size, ported: 0, auto: [], manual: 0, unhandled: [] };
        for (const [key, kase] of keyed) {
          const how = handled.get(prefix + key)?.how;
          if (how === 'ported') row.ported++;
          else if (how) row.manual++;
          else {
            const auto = AUTO_PACKAGES.has(t.package) ? autoCategory(kase) : null;
            if (auto) row.auto.push({ key, category: auto });
            else row.unhandled.push(key);
          }
        }
        rows.push(row);
      }
    }
    rows.sort((a, b) => (a.file < b.file ? -1 : 1));
    progress.push({ package: t.package, rows });
  }
  // Markers of packages with no layout.toml table yet: checked, not reported
  // per file.
  const listed = new Set(tables.map((t) => t.package));
  const unlisted = {};
  for (const m of markers) if (versions[m.pkg] && !listed.has(m.pkg)) unlisted[m.pkg] = (unlisted[m.pkg] ?? 0) + 1;
  return { errors, warnings, progress, untracked, unlisted };
}

function sum(rows) {
  const t = { files: rows.length, total: 0, ported: 0, auto: 0, manual: 0, unhandled: 0 };
  for (const r of rows) {
    t.total += r.total;
    t.ported += r.ported;
    t.auto += r.auto.length;
    t.manual += r.manual;
    t.unhandled += r.unhandled.length;
  }
  return t;
}

export function renderSummary({ errors, warnings, progress, untracked, unlisted, markerCount }) {
  const out = ['## Upstream test parity (RFC-0045 §7)', ''];
  out.push(`${markerCount} markers; ${errors.length} errors; ${warnings.length} warnings.`, '');
  if (errors.length) out.push('### Errors', '', ...errors.map((e) => `- ${e}`), '');
  if (warnings.length) out.push('### Warnings', '', ...warnings.map((e) => `- ${e}`), '');
  out.push('### Per package', '', '| package | files | cases | ported | auto-excluded | hand-excluded | not handled |', '|---|---:|---:|---:|---:|---:|---:|');
  const all = [];
  for (const p of progress) {
    const t = sum(p.rows);
    all.push(...p.rows);
    out.push(`| ${p.package} | ${t.files} | ${t.total} | ${t.ported} | ${t.auto} | ${t.manual} | ${t.unhandled} |`);
  }
  const t = sum(all);
  out.push(`| **total** | ${t.files} | ${t.total} | ${t.ported} | ${t.auto} | ${t.manual} | ${t.unhandled} |`, '');
  for (const [pkg, n] of Object.entries(unlisted))
    out.push(`- ${pkg}: ${n} markers, no layout.toml table yet`);
  if (Object.keys(unlisted).length) out.push('');
  for (const p of progress) {
    if (!p.rows.length) continue;
    out.push(`<details><summary>${p.package}: per file</summary>`, '', '| file | cases | ported | auto-excluded | hand-excluded | not handled |', '|---|---:|---:|---:|---:|---:|');
    for (const r of p.rows)
      out.push(`| ${r.file} | ${r.total} | ${r.ported} | ${r.auto.length} | ${r.manual} | ${r.unhandled.length} |`);
    out.push('', '</details>', '');
  }
  const autos = progress.flatMap((p) => p.rows.flatMap((r) => r.auto.map((a) => `- \`${a.category}\` ${p.package} ${r.file}${SEP}${a.key}`)));
  if (autos.length) out.push('<details><summary>Automatic exclusions</summary>', '', ...autos, '', '</details>', '');
  const open = progress.flatMap((p) => p.rows.flatMap((r) => r.unhandled.map((k) => `- ${p.package} ${r.file}${SEP}${k}`)));
  if (open.length) out.push('<details><summary>Not handled</summary>', '', ...open, '', '</details>', '');
  if (untracked.length)
    out.push('<details><summary>Test files with no layout.toml entry of their own name and no marker</summary>', '', ...untracked.map((f) => `- ${f}`), '', '</details>', '');
  return out.join('\n');
}

// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const args = { command: 'check', versions: path.join(root, 'fixtures/aisdk/VERSIONS.json') };
  if (['check', 'diff', 'tags'].includes(argv[0])) args.command = argv.shift();
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i].replace(/^--/, '');
    if (!['repo', 'versions', 'summary', 'base'].includes(key) || argv[i + 1] === undefined)
      throw new Error(`unknown or incomplete argument: ${argv[i]}`);
    args[key] = argv[i + 1];
  }
  if (!args.repo && args.command !== 'tags') throw new Error('--repo <vercel/ai clone> is required');
  if (args.command === 'diff' && !args.base) throw new Error('diff needs --base <git rev>');
  return args;
}

const gitFiles = (...patterns) =>
  execFileSync('git', ['-C', root, 'ls-files', '-z', '--', ...patterns], { encoding: 'utf8', maxBuffer: 1 << 26 })
    .split('\0')
    .filter(Boolean);

function loadRepo(versions, errors) {
  const layoutFiles = gitFiles(':(glob)*/layout.toml').map((rel) => ({ rel, src: fs.readFileSync(path.join(root, rel), 'utf8') }));
  const tables = readLayouts(layoutFiles, versions, errors);
  const markers = [];
  for (const rel of gitFiles(':(glob)**/*.rs')) {
    const src = fs.readFileSync(path.join(root, rel), 'utf8');
    if (!src.includes('upstream:')) continue;
    const r = scanMarkers(rel, src);
    markers.push(...r.markers);
    errors.push(...r.errors);
  }
  return { tables, markers };
}

// The (package, version) pairs whose cases the markers and entries need.
function neededPackages(versions, tables, markers) {
  const want = new Map();
  const add = (name, version) => want.set(`${name}@${version}`, { name, version });
  for (const m of markers) if (versions[m.pkg]) add(m.pkg, versions[m.pkg].version);
  for (const t of tables) for (const e of t.files) add(t.package, e.version ?? versions[t.package].version);
  for (const t of tables) add(t.package, versions[t.package].version);
  return [...want.values()];
}

function runCheck(args) {
  const versions = JSON.parse(fs.readFileSync(args.versions, 'utf8'));
  const loadErrors = [];
  const { tables, markers } = loadRepo(versions, loadErrors);
  const inventory = readPackages(args.repo, neededPackages(versions, tables, markers));
  const result = check({ versions, tables, markers, inventory });
  result.errors.unshift(...loadErrors);
  const summary = renderSummary({ ...result, markerCount: markers.length });
  const target = args.summary ?? process.env.GITHUB_STEP_SUMMARY;
  if (target) fs.appendFileSync(target, summary + '\n');
  for (const p of result.progress) {
    const t = sum(p.rows);
    console.log(`${p.package}\tfiles ${t.files}\tcases ${t.total}\tported ${t.ported}\tauto ${t.auto}\thand ${t.manual}\tnot handled ${t.unhandled}`);
  }
  for (const [pkg, n] of Object.entries(result.unlisted)) console.log(`${pkg}\t${n} markers, no layout.toml table yet`);
  console.log(`${markers.length} markers`);
  for (const w of result.warnings) console.log(`warning: ${w}`);
  for (const e of result.errors) console.error(`error: ${e}`);
  if (result.errors.length) {
    console.error(`${result.errors.length} errors`);
    process.exitCode = 1;
  }
}

// Prints the vercel/ai tags `check` reads, one per line, for a CI job to fetch.
function runTags(args) {
  const versions = JSON.parse(fs.readFileSync(args.versions, 'utf8'));
  const errors = [];
  const { tables, markers } = loadRepo(versions, errors);
  for (const p of neededPackages(versions, tables, markers)) console.log(`${p.name}@${p.version}`);
}

function runDiff(args) {
  const versions = JSON.parse(fs.readFileSync(args.versions, 'utf8'));
  const base = JSON.parse(execFileSync('git', ['-C', root, 'show', `${args.base}:fixtures/aisdk/VERSIONS.json`], { encoding: 'utf8' }));
  const errors = [];
  const { tables, markers } = loadRepo(versions, errors);
  const bumped = tables.filter((t) => base[t.package] && base[t.package].version !== versions[t.package].version);
  if (!bumped.length) {
    console.log('no package with a layout.toml table changed version');
    return;
  }
  const pairs = bumped.flatMap((t) => [
    { name: t.package, version: base[t.package].version },
    { name: t.package, version: versions[t.package].version },
  ]);
  const inventory = readPackages(args.repo, pairs);
  const progress = check({ versions, tables, markers, inventory: inventory.filter((p) => p.version === versions[p.package].version) }).progress;
  for (const t of bumped) {
    const dir = `packages/${t.package.replace(/^@ai-sdk\//, '')}/`;
    const at = (v) => new Map(inventory.find((p) => p.package === t.package && p.version === v).files.map((f) => [f.file.slice(dir.length), keyCases(f.cases)]));
    const before = at(base[t.package].version);
    const after = at(versions[t.package].version);
    console.log(`${t.package} ${base[t.package].version} → ${versions[t.package].version}`);
    for (const row of progress.find((p) => p.package === t.package)?.rows ?? []) {
      const old = before.get(row.file) ?? new Map();
      const added = [...after.get(row.file).keys()].filter((k) => !old.has(k));
      for (const k of added) console.log(`  new: ${row.file}${SEP}${k}`);
    }
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = parseArgs(process.argv.slice(2));
  if (args.command === 'diff') runDiff(args);
  else if (args.command === 'tags') runTags(args);
  else runCheck(args);
}
