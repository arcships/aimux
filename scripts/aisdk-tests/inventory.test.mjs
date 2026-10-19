import assert from 'node:assert/strict';
import { test } from 'node:test';

import { listCases } from './inventory.mjs';

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
