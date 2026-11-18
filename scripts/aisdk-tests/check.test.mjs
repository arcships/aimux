import assert from 'node:assert/strict';
import { test } from 'node:test';

import { autoCategory, check, keyCases, readLayouts, scanMarkers } from './check.mjs';
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

[nope]
package = "@ai-sdk/nope"
`;
  readLayouts([{ rel: 'layout.toml', src }], versions, errors);
  assert.deepEqual(errors, [
    'layout.toml:5 src/a.ts: a mapped entry needs a tests table (RFC-0045 §3.2)',
    'layout.toml:9 src/b.ts: a missing entry has no tests to exclude',
    'layout.toml:15 src/c.ts: unknown status "done"',
    'layout.toml:19 src/d.ts: version 4.0.51 is below the locked 4.0.52',
    'layout.toml:25 ts-only: exclusion without reason',
    'layout.toml:26 registered: exclusion without ref',
    'layout.toml:27 sub-feature: issue must be an issue number',
    'layout.toml:19 src/d.ts: unknown exclusion category other',
    'layout.toml:34 [nope]: package "@ai-sdk/nope" is not in VERSIONS.json',
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
