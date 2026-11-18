import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { listCases, readPackages } from './inventory.mjs';

const chains = (src) => listCases(src).map((c) => `${c.line} ${c.chain.join(' › ')}`);

test('nests cases under every describe level', () => {
  const src = `
describe('Model', () => {
  describe('doGenerate', () => {
    describe('code interpreter tool', () => {
      it('should send request body with include and tool', async () => {});
    });
    describe('web search tool', () => {
      it('should send request body with include and tool', async () => {});
    });
  });
  it.skip('top level', () => {});
});
test('outside', () => {});
`;
  assert.deepEqual(chains(src), [
    '5 Model › doGenerate › code interpreter tool › should send request body with include and tool',
    '8 Model › doGenerate › web search tool › should send request body with include and tool',
    '11 Model › top level',
    '13 outside',
  ]);
});

test('ignores brackets and calls inside strings, comments, templates and regexes', () => {
  const src = `
describe('a', () => {
  const s = '}) it("fake", () => {';
  // it('commented', () => {});
  /* describe('b', () => { */
  const t = \`\${JSON.stringify({ a: '}' })} it('x')\`;
  const r = /\\}\\)[)}]/g;
  it('real', () => { expect(\`{\${1}}\`).toBe('{1}'); });
});
it('after', () => {});
`;
  assert.deepEqual(chains(src), ['8 a › real', '10 after']);
});

test('counts an each table once and skips member calls', () => {
  const src = `
describe('d', () => {
  it.each([[1], [2]])('case %s', (n) => {});
  test.each\`
    a | b
    \${1} | \${2}
  \`('table $a', () => {});
  helper.it('not a case', () => {});
  it.todo('later');
});
`;
  assert.deepEqual(chains(src), ['3 d › case %s', '4 d › table $a', '9 d › later']);
});

test('handles skipIf and runIf modifiers on describe and it/test', () => {
  const src = `
describe('outer', () => {
  describe.skipIf(cond)('skipScope', () => {
    it('a', () => {});
  });
  describe.runIf(cond)('runScope', () => {
    test('b', () => {});
  });
  it.skipIf(cond)('skippedIt', () => {});
  it.runIf(cond).each([[1]])('chained %s', () => {});
  it('kept', () => {});
});
`;
  assert.deepEqual(chains(src), [
    '4 outer › skipScope › a',
    '7 outer › runScope › b',
    '9 outer › skippedIt',
    '10 outer › chained %s',
    '11 outer › kept',
  ]);
});

test('handles TypeScript type arguments on each and regex literals in predicates and return', () => {
  const src = `
describe('Azure', () => {
  it.each<{ name: string; check: (x: number) => boolean }>([
    { name: 'a', check: (x) => x > 0 },
  ])('supports $name on Azure messages', () => {
    return /\\)/;
  });
  test.runIf(/[(]/.test(v))('after regex predicate', () => {});
});
`;
  const parsed = listCases(src);
  assert.deepEqual(parsed.map((c) => `${c.line} ${c.chain.join(' › ')}`), [
    '3 Azure › supports $name on Azure messages',
    '8 Azure › after regex predicate',
  ]);
  assert.ok(src.slice(parsed[0].start, parsed[0].end).endsWith('})'));
});

test('readPackages works on a full (non-promisor) git repo and with empty test lists', () => {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'aisdk-inv-'));
  try {
    const git = (...args) => execFileSync('git', ['-C', repo, ...args], { encoding: 'utf8' });
    git('init', '-q');
    git('config', 'user.email', 'test@example.com');
    git('config', 'user.name', 'Test');
    fs.mkdirSync(path.join(repo, 'packages/groq/src'), { recursive: true });
    fs.writeFileSync(path.join(repo, 'packages/groq/src/a.test.ts'), "it('x', () => {});\n");
    git('add', '.');
    git('commit', '-q', '-m', 'init');
    git('tag', '@ai-sdk/groq@4.0.52');
    git('tag', '@ai-sdk/empty@1.0.0');

    const res = readPackages(repo, [
      { name: '@ai-sdk/groq', version: '4.0.52' },
      { name: '@ai-sdk/empty', version: '1.0.0' },
    ]);
    assert.equal(res.length, 2);
    assert.deepEqual(res[0].files.map((f) => [f.file, f.cases.map((c) => c.chain)]), [
      ['packages/groq/src/a.test.ts', [['x']]],
    ]);
    assert.deepEqual(res[1].files, []);

    const emptyOnly = readPackages(repo, [{ name: '@ai-sdk/empty', version: '1.0.0' }]);
    assert.deepEqual(emptyOnly[0].files, []);
  } finally {
    fs.rmSync(repo, { recursive: true, force: true });
  }
});
