import assert from 'node:assert/strict';
import { test } from 'node:test';

import { autoCategory, check, diffCases, keyCases, readLayouts, scanMarkers } from './check.mjs';
import { listCases } from './inventory.mjs';
import { parseToml } from './toml.mjs';

const versions = {
  '@ai-sdk/groq': { version: '4.0.52' },
  ai: { version: '7.0.127' },
};

// An inventory entry as readPackages returns it, from TypeScript sources.
const pkg = (name, version, files) => ({
  package: name,
  version,
  tag: `${name}@${version}`,
  files: Object.entries(files).map(([file, src]) => {
    const dir = `packages/${name.replace(/^@ai-sdk\//, '')}/`;
    return { file: dir + file, cases: listCases(src).map((c) => ({ ...c, body: src.slice(c.start, c.end) })) };
  }),
});

const marker = (rest) => `/// upstream: ${rest}\n#[tokio::test]\nasync fn t() {}\n`;

test('parseToml reads the layout.toml subset', () => {
  const doc = parseToml(`
# comment
[groq]
package = "@ai-sdk/groq"

[[groq.files]]
upstream = "src/a.ts" # trailing comment
status = "mapped"

[groq.files.tests]
sub-feature = [
  { case = "a › b", issue = 275, reason = "say \\"why\\"" },
]

[[groq.files]]
upstream = 'src/b.ts'
status = "missing"
`);
  assert.deepEqual(JSON.parse(JSON.stringify(doc)), {
    groq: {
      package: '@ai-sdk/groq',
      files: [
        { upstream: 'src/a.ts', status: 'mapped', tests: { 'sub-feature': [{ case: 'a › b', issue: 275, reason: 'say "why"' }] } },
        { upstream: 'src/b.ts', status: 'missing' },
      ],
    },
  });
  assert.throws(() => parseToml('a = 1.5\n'), /line 1: unsupported value/);
  assert.throws(() => parseToml('a = 1\na = 2\n'), /line 2: duplicate key a/);
  assert.throws(() => parseToml('a = "\\u16ZZ"\n'), /line 1: invalid unicode escape/);
});

test('listCases gives each case its source span', () => {
  const src = "describe('d', () => {\n  it('x', () => { stopWhen(1); });\n  it.todo('y');\n});\n";
  const cases = listCases(src).map((c) => src.slice(c.start, c.end));
  assert.deepEqual(cases, ["it('x', () => { stopWhen(1); })", "it.todo('y')"]);
});

test('keyCases numbers repeated chains and trims names', () => {
  const keys = [...keyCases([{ chain: ['a', 'b'] }, { chain: ['a', 'b '] }, { chain: ['a', 'c'] }]).keys()];
  assert.deepEqual(keys, ['a › b', 'a › b #2', 'a › c']);
});

test('scanMarkers checks the marker syntax and placement', () => {
  const src = [
    marker('@ai-sdk/groq src/a.test.ts › d › x'),
    marker('@ai-sdk/groq src/a.test.ts › d › x #2'),
    marker('@ai-sdk/groq a.test.ts › d › x'),
    marker('@ai-sdk/groq src/a.test.ts › d ›  x'),
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › y\nconst X: u8 = 1;\n',
    '    /// upstream: ai src/generate-text/generate-text.test.ts › generateText › text\n    /// More words.\n    #[test]\n    #[should_panic]\n    fn t() {}\n',
  ].join('');
  const { markers, errors } = scanMarkers('x.rs', src);
  assert.deepEqual(markers.map((m) => [m.where, m.pkg, m.file, m.chain]), [
    ['x.rs:1', '@ai-sdk/groq', 'src/a.test.ts', 'd › x'],
    ['x.rs:4', '@ai-sdk/groq', 'src/a.test.ts', 'd › x #2'],
    ['x.rs:15', 'ai', 'src/generate-text/generate-text.test.ts', 'generateText › text'],
  ]);
  assert.deepEqual(errors.map((e) => e.split(': ')[0] + ': ' + e.split(': ')[1].split(',')[0]), [
    'x.rs:7: malformed marker',
    'x.rs:10: empty or padded describe level in the marker',
    'x.rs:13: marker is not above a test function',
  ]);
});

test('autoCategory applies the RFC-0045 §3.2 lists', () => {
  const cat = (chain, body = '') => autoCategory({ chain, body });
  assert.equal(cat(['generateText', 'options.stopWhen', 'x']), 'single-step');
  assert.equal(cat(['x'], 'tools: { t: tool({ execute: async () => 1 }) }'), 'single-step');
  assert.equal(cat(['x'], "{ type: 'tool-approval-request' }"), 'single-step');
  assert.equal(cat(['x'], 'experimental_telemetry: { isEnabled: true }'), 'telemetry');
  assert.equal(cat(['x'], 'onStepFinish: () => {}'), 'telemetry');
  assert.equal(cat(['x'], 'onFinish: () => {}'), 'telemetry');
  assert.equal(cat(['x'], 'onError: () => {}, onChunk: () => {}, onAbort: () => {}'), null);
  assert.equal(cat(['text', 'should return text']), null);
});

test('readLayouts checks statuses, versions and the tests tables', () => {
  const errors = [];
  const src = `
[groq]
package = "@ai-sdk/groq"

[[groq.files]]
upstream = "src/a.ts"
status = "mapped"

[[groq.files]]
upstream = "src/b.ts"
status = "missing"
issue = 275

[groq.files.tests]

[[groq.files]]
upstream = "src/c.ts"
status = "done"

[[groq.files]]
upstream = "src/d.ts"
status = "mapped"
version = "4.0.51"

[groq.files.tests]
ts-only = [{ case = "x" }]
registered = [{ case = "x", reason = "r" }]
sub-feature = [{ case = "x", reason = "r", issue = "275" }]
other = [{ case = "x", reason = "r" }]

[[groq.files]]
rust = "groq/extra.rs"
status = "aimux"
reason = "extra"

[nope]
package = "@ai-sdk/nope"
`;
  readLayouts([{ rel: 'layout.toml', src }], versions, errors);
  assert.deepEqual(errors, [
    'layout.toml:5 src/a.ts: a mapped entry needs a tests table (RFC-0045 §3.2)',
    'layout.toml:9 src/b.ts: a missing entry has no tests to exclude',
    'layout.toml:16 src/c.ts: unknown status "done"',
    'layout.toml:20 src/d.ts: version 4.0.51 is below the locked 4.0.52',
    'layout.toml:26 ts-only: exclusion without reason',
    'layout.toml:27 registered: exclusion without ref',
    'layout.toml:28 sub-feature: issue must be an issue number',
    'layout.toml:20 src/d.ts: unknown exclusion category other',
    'layout.toml:36 [nope]: package "@ai-sdk/nope" is not in VERSIONS.json',
  ]);
});

test('check resolves markers and exclusions and reports progress', () => {
  const layout = `
[groq]
package = "@ai-sdk/groq"

[[groq.files]]
upstream = "src/a.ts"
status = "mapped"

[groq.files.tests]
ts-only = [
  { case = "d › z", reason = "r" },
  { case = "o › w", reason = "r" },
  { case = "d › missing", reason = "r" },
  { case = "d › x", reason = "r" },
]

[[groq.files]]
upstream = "src/b.ts"
status = "missing"
issue = 275

[ai]
package = "ai"

[[ai.files]]
upstream = "src/decide/decide.ts"
status = "mapped"
version = "7.0.128"

[ai.files.tests]
`;
  const errors = [];
  const tables = readLayouts([{ rel: 'layout.toml', src: layout }], versions, errors);
  assert.deepEqual(errors, []);
  const inventory = [
    pkg('@ai-sdk/groq', '4.0.52', {
      'src/a.test.ts': "describe('d', () => { it('x', f); it('x', f); it('y', f); it('z', f); it('u', f); });",
      'src/b.test.ts': "it('q', f);",
      'src/orphan.test.ts': "describe('o', () => { it('w', f); });",
      'src/lonely.test.ts': "it('l', f);",
    }),
    pkg('ai', '7.0.127', { 'src/decide/decide.test.ts': "it('old', f);" }),
    pkg('ai', '7.0.128', {
      'src/decide/decide.test.ts': "it('new', f); it('loops', () => { stopWhen(stepCountIs(2)); });",
    }),
  ];
  const markers = [
    ['g:1', '@ai-sdk/groq', 'src/a.test.ts', 'd › x'],
    ['g:2', '@ai-sdk/groq', 'src/a.test.ts', 'd › x #2'],
    ['g:3', '@ai-sdk/groq', 'src/a.test.ts', 'd › x'],
    ['g:4', '@ai-sdk/groq', 'src/a.test.ts', 'd › nope'],
    ['g:5', '@ai-sdk/groq', 'src/none.test.ts', 'q'],
    ['g:6', '@ai-sdk/groq', 'src/b.test.ts', 'q'],
    ['g:7', '@ai-sdk/other', 'src/b.test.ts', 'q'],
    ['a:1', 'ai', 'src/decide/decide.test.ts', 'new'],
    ['a:2', 'ai', 'src/decide/decide.test.ts', 'old'],
  ].map(([where, pkg, file, chain]) => ({ where, pkg, file, chain }));
  const r = check({ versions, tables, markers, inventory });
  assert.deepEqual(r.errors, [
    'g:3: case already marked at g:1',
    'g:4: no case "d › nope" in @ai-sdk/groq@4.0.52 src/a.test.ts',
    'g:5: no test file src/none.test.ts in @ai-sdk/groq@4.0.52',
    'g:7: package @ai-sdk/other is not in VERSIONS.json',
    'a:2: no case "old" in ai@7.0.128 src/decide/decide.test.ts',
    'layout.toml:13 ts-only: no case "d › missing" in src/a.test.ts or the test files with no source of their own name',
    'layout.toml:14 ts-only: "d › x" is excluded but marked at g:1',
  ]);
  assert.deepEqual(r.warnings, ['g:6: marks a case of src/b.test.ts, whose source entry is missing']);
  const rows = Object.fromEntries(r.progress.flatMap((p) =>
    p.rows.map((row) => [`${p.package} ${row.file}`, { ...row, auto: row.auto.map((a) => `${a.category} ${a.key}`) }])));
  assert.deepEqual(rows, {
    '@ai-sdk/groq src/a.test.ts': { file: 'src/a.test.ts', version: '4.0.52', total: 5, ported: 2, auto: [], manual: 1, unhandled: ['d › y', 'd › u'] },
    '@ai-sdk/groq src/orphan.test.ts': { file: 'src/orphan.test.ts', version: '4.0.52', total: 1, ported: 0, auto: [], manual: 1, unhandled: [] },
    'ai src/decide/decide.test.ts': { file: 'src/decide/decide.test.ts', version: '7.0.128', total: 2, ported: 1, auto: ['single-step loops'], manual: 0, unhandled: [] },
  });
  assert.deepEqual(r.untracked, ['@ai-sdk/groq src/lonely.test.ts']);
  assert.deepEqual(r.unlisted, {});
});

test('scanMarkers rejects #[ignore] and non-test helper functions', () => {
  const src = [
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › x\n#[ignore]\n#[test]\nfn ignored() {}\n',
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › y\n#[ignore = "wip"]\n#[tokio::test]\nasync fn ignored_msg() {}\n',
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › z\nfn helper() {}\n',
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › ok\n#[tokio::test(flavor = "multi_thread")]\nasync fn ok() {}\n',
  ].join('');
  const { markers, errors } = scanMarkers('x.rs', src);
  assert.deepEqual(markers.map((m) => [m.where, m.chain]), [['x.rs:11', 'd › ok']]);
  assert.deepEqual(errors, [
    'x.rs:1: marker sits on an #[ignore] test (RFC-0045 §5)',
    'x.rs:5: marker sits on an #[ignore] test (RFC-0045 §5)',
    'x.rs:9: marker is not above a test function',
  ]);
});

test('readLayouts and check handle malformed files/upstream, tsx entries, and version-overridden orphan exclusions', () => {
  const badErrors = [];
  const badTables = readLayouts([{
    rel: 'layout.toml',
    src: `
[groq]
package = "@ai-sdk/groq"
[groq.files]
upstream = "src/a.ts"
status = "mapped"

[ai]
package = "ai"
[[ai.files]]
status = "mapped"
[ai.files.tests]
ts-only = [{ case = "x", reason = "r" }]
`,
  }], versions, badErrors);
  assert.deepEqual(badErrors, [
    'layout.toml:2 [groq]: files must be an array of tables',
    'layout.toml:10 undefined: no upstream path',
  ]);
  const badCheck = check({ versions, tables: badTables, markers: [], inventory: [] });
  assert.deepEqual(badCheck.errors, []);

  const layout = `
[groq]
package = "@ai-sdk/groq"

[[groq.files]]
upstream = "src/comp.tsx"
status = "mapped"

[groq.files.tests]
ts-only = [{ case = "c", reason = "r" }]

[ai]
package = "ai"

[[ai.files]]
upstream = "src/decide/decide.ts"
status = "mapped"
version = "7.0.128"

[ai.files.tests]
ts-only = [
  { case = "locked orphan case", reason = "r" },
  { case = "override orphan case", reason = "r" },
]
`;
  const layoutErrors = [];
  const tables = readLayouts([{ rel: 'layout.toml', src: layout }], versions, layoutErrors);
  assert.deepEqual(layoutErrors, []);
  const inventory = [
    pkg('@ai-sdk/groq', '4.0.52', {
      'src/comp.test.tsx': "it('c', f);",
    }),
    pkg('ai', '7.0.127', {
      'src/orphan-locked.test.ts': "it('locked orphan case', f);",
    }),
    pkg('ai', '7.0.128', {
      'src/decide/decide.test.ts': "it('d', f);",
      'src/orphan-locked.test.ts': "it('locked orphan case', f);",
      'src/orphan-new.test.ts': "it('override orphan case', f);",
    }),
  ];
  const r = check({ versions, tables, markers: [], inventory });
  assert.deepEqual(r.errors, []);
  assert.deepEqual(r.untracked, []);
  const rows = Object.fromEntries(r.progress.flatMap((p) =>
    p.rows.map((row) => [`${p.package}@${row.version} ${row.file}`, { manual: row.manual, unhandled: row.unhandled }])));
  assert.deepEqual(rows, {
    '@ai-sdk/groq@4.0.52 src/comp.test.tsx': { manual: 1, unhandled: [] },
    'ai@7.0.127 src/orphan-locked.test.ts': { manual: 1, unhandled: [] },
    'ai@7.0.128 src/decide/decide.test.ts': { manual: 0, unhandled: ['d'] },
    'ai@7.0.128 src/orphan-new.test.ts': { manual: 1, unhandled: [] },
  });
});

test('scanMarkers rejects upstream + partial on the same test while allowing standalone partial', () => {
  const src = [
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › x\n/// partial: missing edge case\n#[test]\nfn both_after() {}\n',
    '/// partial: missing edge case\n/// upstream: @ai-sdk/groq src/a.test.ts › d › y\n#[test]\nfn both_before() {}\n',
    '/// partial: @ai-sdk/groq src/a.test.ts › d › z\n#[test]\nfn partial_only() {}\n',
  ].join('');
  const { markers, partials, errors } = scanMarkers('x.rs', src);
  assert.deepEqual(markers, []);
  assert.equal(partials.length, 3);
  assert.deepEqual(errors, [
    'x.rs:1: marker sits on a partial test (remove upstream: or complete the port)',
    'x.rs:6: marker sits on a partial test (remove upstream: or complete the port)',
  ]);
});

test('readLayouts requires issue on missing and reason on not-applicable', () => {
  const errors = [];
  readLayouts([{
    rel: 'aimux-providers/layout.toml',
    src: `
[groq]
package = "@ai-sdk/groq"

[[groq.files]]
upstream = "src/a.ts"
status = "missing"

[[groq.files]]
upstream = "src/b.ts"
status = "missing"
issue = "275"

[[groq.files]]
upstream = "src/c.ts"
status = "not-applicable"

[[groq.files]]
upstream = "src/d.ts"
status = "not-applicable"
reason = "   "

[[groq.files]]
upstream = "src/e.ts"
status = "missing"
issue = 275

[[groq.files]]
upstream = "src/f.ts"
status = "not-applicable"
reason = "crate version used"
`,
  }], versions, errors);
  assert.deepEqual(errors, [
    'aimux-providers/layout.toml:5 src/a.ts: a missing entry needs an issue number',
    'aimux-providers/layout.toml:9 src/b.ts: a missing entry needs an issue number',
    'aimux-providers/layout.toml:14 src/c.ts: a not-applicable entry needs a reason',
    'aimux-providers/layout.toml:18 src/d.ts: a not-applicable entry needs a reason',
  ]);
});

test('check validates marker ownership against layout.toml crate and vendor directory', () => {
  const layout = `
[groq]
package = "@ai-sdk/groq"

[[groq.files]]
upstream = "src/a.ts"
rust = "groq/a.rs"
status = "mapped"

[groq.files.tests]

[[groq.files]]
upstream = "src/shared.ts"
rust = "shared/shared.rs"
status = "mapped"

[groq.files.tests]
`;
  const errors = [];
  const tables = readLayouts([{ rel: 'aimux-providers/layout.toml', src: layout }], versions, errors);
  assert.deepEqual(errors, []);
  const inventory = [
    pkg('@ai-sdk/groq', '4.0.52', {
      'src/a.test.ts': "it('ok', f); it('wrong_vendor', f); it('legacy_dir', f);",
      'src/shared.test.ts': "it('shared_ok', f);",
      'src/orphan.test.ts': "it('orphan_ok', f); it('orphan_bad', f);",
    }),
  ];
  const make = (rel, file, chain, pkgName = '@ai-sdk/groq') =>
    scanMarkers(rel, marker(`${pkgName} ${file} › ${chain}`)).markers[0];
  const markers = [
    make('aimux-providers/src/groq/tests/a.rs', 'src/a.test.ts', 'ok'),
    make('aimux-providers/src/shared/tests/shared.rs', 'src/shared.test.ts', 'shared_ok'),
    make('aimux-providers/src/groq/tests/orphan.rs', 'src/orphan.test.ts', 'orphan_ok'),
    make('aimux-providers/src/openai/tests/a.rs', 'src/a.test.ts', 'wrong_vendor'),
    make('aimux-providers/tests/groq_test.rs', 'src/a.test.ts', 'legacy_dir'),
    make('aimux-providers/src/openai/tests/orphan.rs', 'src/orphan.test.ts', 'orphan_bad'),
    make('aimux-providers/src/groq/tests/u.rs', 'src/u.test.ts', 'unlisted_in_vendor', '@ai-sdk/other'),
  ];
  const r = check({ versions, tables, markers, inventory });
  assert.deepEqual(r.errors, [
    'aimux-providers/src/openai/tests/a.rs:1: marker belongs to @ai-sdk/groq (expected under aimux-providers/src/groq/tests/), found in aimux-providers/src/openai/tests/a.rs',
    'aimux-providers/tests/groq_test.rs:1: marker belongs to @ai-sdk/groq (expected under aimux-providers/src/groq/tests/), found in aimux-providers/tests/groq_test.rs',
    'aimux-providers/src/openai/tests/orphan.rs:1: marker belongs to @ai-sdk/groq (expected under aimux-providers/src/groq/tests/), found in aimux-providers/src/openai/tests/orphan.rs',
    'aimux-providers/src/groq/tests/u.rs:1: package @ai-sdk/other is not in VERSIONS.json',
  ]);
  // Unlisted package that IS in VERSIONS.json sitting inside a listed vendor dir:
  const rUnlisted = check({
    versions: { ...versions, '@ai-sdk/provider-utils': { version: '5.0.55' } },
    tables,
    markers: [make('aimux-providers/src/groq/tests/u.rs', 'src/u.test.ts', 'x', '@ai-sdk/provider-utils')],
    inventory,
  });
  assert.deepEqual(rUnlisted.errors, [
    'aimux-providers/src/groq/tests/u.rs:1: marker for @ai-sdk/provider-utils sits in @ai-sdk/groq directory aimux-providers/src/groq/tests/u.rs',
  ]);
});

test('scanMarkers handles pre-marker #[ignore]/#[test], same-line #[test] #[ignore], multiline attributes, and quoted strings', () => {
  const src = [
    '#[ignore]\n/// upstream: @ai-sdk/groq src/a.test.ts › d › pre_ignore\n#[test]\nfn f1() {}\n',
    '#[test] #[ignore]\n/// upstream: @ai-sdk/groq src/a.test.ts › d › same_line\nfn f2() {}\n',
    '#[test]\n/// upstream: @ai-sdk/groq src/a.test.ts › d › pre_test\nfn f3() {}\n',
    '#[tokio::test(\n  flavor = "multi_thread"\n)]\n/// upstream: @ai-sdk/groq src/a.test.ts › d › multi_attr\nasync fn f4() {}\n',
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › quoted_bracket\n#[test]\n#[should_panic(expected = "missing [")]\nfn f5() {}\n',
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › doc_fake_test\n#[doc = "Shows #[test] syntax."]\nfn helper() {}\n',
    '#[doc = "two\nlines"]\nfn h2() {}\n/// upstream: @ai-sdk/groq src/a.test.ts › d › after_multiline_doc\n#[test]\nfn f6() {}\n',
    '/// upstream: @ai-sdk/groq src/a.test.ts › d › block_comment_test\n#[doc = /* #[test] */ "helper"]\nfn h3() {}\n',
  ].join('');
  const { markers, errors } = scanMarkers('x.rs', src);
  assert.deepEqual(markers.map((m) => m.chain), ['d › pre_test', 'd › multi_attr', 'd › quoted_bracket', 'd › after_multiline_doc']);
  assert.deepEqual(errors, [
    'x.rs:2: marker sits on an #[ignore] test (RFC-0045 §5)',
    'x.rs:6: marker sits on an #[ignore] test (RFC-0045 §5)',
    'x.rs:20: marker is not above a test function',
    'x.rs:29: marker is not above a test function',
  ]);
});

test('readLayouts rejects non-table tests, non-string/negative fields, aimux/elsewhere omissions, and array versions', () => {
  const errors = [];
  readLayouts([{
    rel: 'layout.toml',
    src: `
[groq]
package = "@ai-sdk/groq"

[[groq.files]]
upstream = "src/a.ts"
status = "mapped"
tests = true

[[groq.files]]
upstream = "src/b.ts"
status = "elsewhere"

[groq.files.tests]
constructor = []
ts-only = [{ case = "c", reason = true }]
sub-feature = [{ case = "c", reason = "r", issue = -1 }]

[[groq.files]]
rust = "groq/x.rs"
upstream = 42
status = "aimux"

[[groq.files]]
upstream = "src/v.ts"
status = "missing"
issue = 275
version = ["4.0.53"]
`,
  }], versions, errors);
  assert.deepEqual(errors, [
    'layout.toml:5 src/a.ts: tests must be a table',
    'layout.toml:10 src/b.ts: an elsewhere entry needs a rust path',
    'layout.toml:10 src/b.ts: unknown exclusion category constructor',
    'layout.toml:16 ts-only: exclusion without reason',
    'layout.toml:17 sub-feature: issue must be an issue number',
    'layout.toml:19 42: an aimux entry has no upstream path',
    'layout.toml:19 42: an aimux entry needs a reason',
    'layout.toml:24 src/v.ts: version ["4.0.53"] is not x.y.z',
  ]);
  const malformedTables = readLayouts([{
    rel: 'layout.toml',
    src: '[groq]\npackage = "@ai-sdk/groq"\n[[groq.files]]\nrust = "groq/x.rs"\nupstream = 42\nstatus = "aimux"\nreason = "r"\n',
  }], versions, []);
  assert.deepEqual(check({ versions, tables: malformedTables, markers: [], inventory: [] }).errors, []);
});

test('check resolves cross-crate elsewhere ownership and multi-table missing-only vendors', () => {
  const layout = `
[groq]
package = "@ai-sdk/groq"

[[groq.files]]
upstream = "src/sigv4.ts"
rust = "../aimux-provider-utils/src/sigv4.rs"
status = "elsewhere"

[groq.files.tests]

[fal]
package = "@ai-sdk/fal"

[[fal.files]]
upstream = "src/fal.ts"
status = "missing"
issue = 275
`;
  const v = { ...versions, '@ai-sdk/fal': { version: '3.0.54' }, '@ai-sdk/provider-utils': { version: '5.0.55' } };
  const errors = [];
  const tables = readLayouts([{ rel: 'aimux-providers/layout.toml', src: layout }], v, errors);
  assert.deepEqual(errors, []);
  const inventory = [pkg('@ai-sdk/groq', '4.0.52', { 'src/sigv4.test.ts': "it('s', f);" })];
  const mCross = scanMarkers('aimux-provider-utils/src/tests/sigv4.rs', marker('@ai-sdk/groq src/sigv4.test.ts › s')).markers[0];
  const mUnlisted = scanMarkers('aimux-providers/src/other_unlisted.rs', marker('@ai-sdk/provider-utils src/u.test.ts › u')).markers[0];
  const r = check({
    versions: v,
    tables,
    markers: [mCross, mUnlisted],
    inventory: [...inventory, pkg('@ai-sdk/provider-utils', '5.0.55', { 'src/u.test.ts': "it('u', f);" })],
  });
  assert.deepEqual(r.errors, []);

  // Cross-crate elsewhere targeting a single-vendor-scoped crate restricts to that vendor's tests dir:
  const targetTables = readLayouts([
    {
      rel: 'aimux-providers/layout.toml',
      src: '[groq]\npackage = "@ai-sdk/groq"\n[[groq.files]]\nupstream = "src/a.ts"\nrust = "../target-crate/src/anthropic/a.rs"\nstatus = "elsewhere"\n[groq.files.tests]\n',
    },
    {
      rel: 'target-crate/layout.toml',
      src: '[anthropic]\npackage = "@ai-sdk/anthropic"\n[[anthropic.files]]\nupstream = "src/x.ts"\nrust = "anthropic/x.rs"\nstatus = "mapped"\n[anthropic.files.tests]\n',
    },
  ], { ...v, '@ai-sdk/anthropic': { version: '4.0.71' } }, []);
  const mTargetOk = scanMarkers('target-crate/src/anthropic/tests/a.rs', marker('@ai-sdk/groq src/a.test.ts › ok')).markers[0];
  const mTargetBad = scanMarkers('target-crate/src/shared/a.rs', marker('@ai-sdk/groq src/a.test.ts › bad')).markers[0];
  const rTarget = check({
    versions: { ...v, '@ai-sdk/anthropic': { version: '4.0.71' } },
    tables: targetTables,
    markers: [mTargetOk, mTargetBad],
    inventory: [pkg('@ai-sdk/groq', '4.0.52', { 'src/a.test.ts': "it('ok', f); it('bad', f);" })],
  });
  assert.deepEqual(rTarget.errors, [
    'target-crate/src/shared/a.rs:1: marker belongs to @ai-sdk/groq (expected under target-crate/src/anthropic/tests/), found in target-crate/src/shared/a.rs',
  ]);

  // Root-level elsewhere target in a vendor-scoped crate restricts to src/tests/, not src/shared/:
  const rootTargetTables = readLayouts([
    {
      rel: 'aimux-providers/layout.toml',
      src: '[groq]\npackage = "@ai-sdk/groq"\n[[groq.files]]\nupstream = "src/a.ts"\nrust = "../target-crate/src/a.rs"\nstatus = "elsewhere"\n[groq.files.tests]\n',
    },
    {
      rel: 'target-crate/layout.toml',
      src: '[anthropic]\npackage = "@ai-sdk/anthropic"\n[[anthropic.files]]\nupstream = "src/x.ts"\nrust = "anthropic/x.rs"\nstatus = "mapped"\n[anthropic.files.tests]\n',
    },
  ], { ...v, '@ai-sdk/anthropic': { version: '4.0.71' } }, []);
  const mRootOk = scanMarkers('target-crate/src/tests/a.rs', marker('@ai-sdk/groq src/a.test.ts › ok')).markers[0];
  const mRootBad = scanMarkers('target-crate/src/shared/a.rs', marker('@ai-sdk/groq src/a.test.ts › bad')).markers[0];
  const rRoot = check({
    versions: { ...v, '@ai-sdk/anthropic': { version: '4.0.71' } },
    tables: rootTargetTables,
    markers: [mRootOk, mRootBad],
    inventory: [pkg('@ai-sdk/groq', '4.0.52', { 'src/a.test.ts': "it('ok', f); it('bad', f);" })],
  });
  assert.deepEqual(rRoot.errors, [
    'target-crate/src/shared/a.rs:1: marker belongs to @ai-sdk/groq (expected under target-crate/src/tests/), found in target-crate/src/shared/a.rs',
  ]);
});

test('diffCases compares per-file effective versions across lock catchup and override bumps', () => {
  const baseVersions = { ai: { version: '7.0.127' } };
  const baseTables = readLayouts([{
    rel: 'aimux-core/layout.toml',
    src: `
[ai]
package = "ai"
[[ai.files]]
upstream = "src/decide/decide.ts"
status = "mapped"
version = "7.0.128"
[ai.files.tests]
[[ai.files]]
upstream = "src/text.ts"
status = "mapped"
[ai.files.tests]
`,
  }], baseVersions, []);

  // Lock catches up to 7.0.128 and removes decide's version override: decide has 0 new cases, text has 1 new case.
  const headVersions = { ai: { version: '7.0.128' } };
  const headTables = readLayouts([{
    rel: 'aimux-core/layout.toml',
    src: `
[ai]
package = "ai"
[[ai.files]]
upstream = "src/decide/decide.ts"
status = "mapped"
[ai.files.tests]
[[ai.files]]
upstream = "src/text.ts"
status = "mapped"
[ai.files.tests]
`,
  }], headVersions, []);

  const inventory = [
    pkg('ai', '7.0.127', { 'src/text.test.ts': "it('t1', f);" }),
    pkg('ai', '7.0.128', {
      'src/decide/decide.test.ts': "it('d1', f); it('d2', f);",
      'src/text.test.ts': "it('t1', f); it('t2', f);",
    }),
  ];
  const catchup = diffCases({ baseVersions, baseTables, versions: headVersions, tables: headTables, markers: [], inventory });
  assert.deepEqual(catchup, [{
    package: 'ai',
    rows: [{ file: 'src/text.test.ts', beforeVer: '7.0.127', afterVer: '7.0.128', added: ['t2'] }],
  }]);

  // Selected orphan at override 7.0.128 in base and 7.0.129 in head diffs 7.0.128 -> 7.0.129:
  const oBaseTables = readLayouts([{
    rel: 'aimux-core/layout.toml',
    src: '[ai]\npackage = "ai"\n[[ai.files]]\nupstream = "src/decide/decide.ts"\nstatus = "mapped"\nversion = "7.0.128"\n[ai.files.tests]\nts-only = [{ case = "o1", reason = "r" }]\n',
  }], baseVersions, []);
  const oHeadTables = readLayouts([{
    rel: 'aimux-core/layout.toml',
    src: '[ai]\npackage = "ai"\n[[ai.files]]\nupstream = "src/decide/decide.ts"\nstatus = "mapped"\nversion = "7.0.129"\n[ai.files.tests]\nts-only = [{ case = "o1", reason = "r" }]\n',
  }], baseVersions, []);
  const oInv = [
    pkg('ai', '7.0.127', {}),
    pkg('ai', '7.0.128', { 'src/decide/decide.test.ts': "it('d1', f);", 'src/orphan.test.ts': "it('o1', f);" }),
    pkg('ai', '7.0.129', { 'src/decide/decide.test.ts': "it('d1', f);", 'src/orphan.test.ts': "it('o1', f); it('o2', f);" }),
  ];
  const oDiff = diffCases({ baseVersions, baseTables: oBaseTables, versions: baseVersions, tables: oHeadTables, markers: [], inventory: oInv });
  assert.deepEqual(oDiff, [{
    package: 'ai',
    rows: [{ file: 'src/orphan.test.ts', beforeVer: '7.0.128', afterVer: '7.0.129', added: ['o2'] }],
  }]);

  // Multi-version orphan selected at 7.0.128 and 7.0.129 in base, with 7.0.129 bumped to 7.0.130 in head:
  const mBase = readLayouts([{
    rel: 'aimux-core/layout.toml',
    src: '[ai]\npackage = "ai"\n[[ai.files]]\nupstream = "src/a.ts"\nstatus = "mapped"\nversion = "7.0.128"\n[ai.files.tests]\nts-only = [{ case = "o1", reason = "r" }]\n[[ai.files]]\nupstream = "src/b.ts"\nstatus = "mapped"\nversion = "7.0.129"\n[ai.files.tests]\nts-only = [{ case = "o2", reason = "r" }]\n',
  }], baseVersions, []);
  const mHead = readLayouts([{
    rel: 'aimux-core/layout.toml',
    src: '[ai]\npackage = "ai"\n[[ai.files]]\nupstream = "src/a.ts"\nstatus = "mapped"\nversion = "7.0.128"\n[ai.files.tests]\nts-only = [{ case = "o1", reason = "r" }]\n[[ai.files]]\nupstream = "src/b.ts"\nstatus = "mapped"\nversion = "7.0.130"\n[ai.files.tests]\nts-only = [{ case = "o2", reason = "r" }]\n',
  }], baseVersions, []);
  const mInv = [
    pkg('ai', '7.0.127', {}),
    pkg('ai', '7.0.128', { 'src/orphan.test.ts': "it('o1', f);" }),
    pkg('ai', '7.0.129', { 'src/orphan.test.ts': "it('o1', f); it('o2', f);" }),
    pkg('ai', '7.0.130', { 'src/orphan.test.ts': "it('o1', f); it('o2', f); it('o3', f);" }),
  ];
  const mDiff = diffCases({ baseVersions, baseTables: mBase, versions: baseVersions, tables: mHead, markers: [], inventory: mInv });
  assert.deepEqual(mDiff, [{
    package: 'ai',
    rows: [{ file: 'src/orphan.test.ts', beforeVer: '7.0.128,7.0.129', afterVer: '7.0.130', added: ['o3'] }],
  }]);
});
