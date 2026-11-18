// A TOML reader for the subset the `layout.toml` files use (RFC-0046 §4.4):
// `[table]` and `[[array.of.tables]]` headers with bare keys, `key = value`
// pairs, basic and literal strings, integers, booleans, arrays (over several
// lines, trailing comma allowed) and inline tables. Anything else is an error
// naming the line, so a new construct fails loudly instead of being misread.
//
// Every table records the line of its header (or inline table) under `LINE`.

export const LINE = Symbol('line');

export function parseToml(src) {
  const root = {};
  let current = root;
  let line = 1;
  let i = 0;

  const fail = (msg) => {
    throw new Error(`line ${line}: ${msg}`);
  };
  // Skips spaces, and with `newlines` also line breaks and comments.
  const skip = (newlines) => {
    for (;;) {
      const c = src[i];
      if (c === ' ' || c === '\t' || c === '\r') i++;
      else if (c === '#') while (i < src.length && src[i] !== '\n') i++;
      else if (newlines && c === '\n') { line++; i++; }
      else return;
    }
  };
  const bareKey = () => {
    const m = /^[A-Za-z0-9_-]+/.exec(src.slice(i, i + 128));
    if (!m) fail(`expected a key, found ${JSON.stringify(src.slice(i, i + 10))}`);
    i += m[0].length;
    return m[0];
  };
  const dottedKey = () => {
    const keys = [bareKey()];
    for (skip(false); src[i] === '.'; skip(false)) {
      i++;
      skip(false);
      keys.push(bareKey());
    }
    return keys;
  };
  const basicString = () => {
    let out = '';
    for (i++; src[i] !== '"'; i++) {
      if (i >= src.length || src[i] === '\n') fail('unterminated string');
      if (src[i] !== '\\') { out += src[i]; continue; }
      const e = src[++i];
      const simple = { b: '\b', t: '\t', n: '\n', f: '\f', r: '\r', '"': '"', '\\': '\\' };
      if (Object.hasOwn(simple, e)) out += simple[e];
      else if (e === 'u' || e === 'U') {
        const n = e === 'u' ? 4 : 8;
        const hex = src.slice(i + 1, i + 1 + n);
        const cp = hex.length === n && /^[0-9A-Fa-f]+$/.test(hex) ? parseInt(hex, 16) : -1;
        if (cp < 0 || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) fail(`invalid unicode escape \\${e}${hex}`);
        out += String.fromCodePoint(cp);
        i += n;
      } else fail(`unknown escape \\${e}`);
    }
    i++;
    return out;
  };
  const value = () => {
    const c = src[i];
    if (c === '"') {
      if (src.startsWith('"""', i)) fail('multi-line strings are not supported');
      return basicString();
    }
    if (c === "'") {
      if (src.startsWith("'''", i)) fail('multi-line strings are not supported');
      const end = src.indexOf("'", i + 1);
      const nl = src.indexOf('\n', i + 1);
      if (end < 0 || (nl >= 0 && nl < end)) fail('unterminated string');
      const out = src.slice(i + 1, end);
      i = end + 1;
      return out;
    }
    if (c === '[') {
      const out = [];
      i++;
      for (;;) {
        skip(true);
        if (src[i] === ']') { i++; return out; }
        out.push(value());
        skip(true);
        if (src[i] === ',') i++;
        else if (src[i] !== ']') fail('expected , or ] in an array');
      }
    }
    if (c === '{') {
      const out = {};
      Object.defineProperty(out, LINE, { value: line });
      i++;
      skip(false);
      if (src[i] === '}') { i++; return out; }
      for (;;) {
        skip(false);
        const keys = dottedKey();
        skip(false);
        if (src[i] !== '=') fail('expected = in an inline table');
        i++;
        skip(false);
        assign(out, keys, value());
        skip(false);
        if (src[i] === '}') { i++; return out; }
        if (src[i] !== ',') fail('expected , or } in an inline table');
        i++;
      }
    }
    const m = /^(true|false|[+-]?\d[\d_]*)(?=[\s,\]}#]|$)/.exec(src.slice(i, i + 64));
    if (!m) fail(`unsupported value ${JSON.stringify(src.slice(i, i + 20))}`);
    i += m[0].length;
    if (m[0] === 'true' || m[0] === 'false') return m[0] === 'true';
    return Number(m[0].replace(/_/g, ''));
  };
  function assign(table, keys, v) {
    let t = table;
    for (const k of keys.slice(0, -1)) {
      if (t[k] === undefined) t[k] = {};
      t = t[k];
      if (typeof t !== 'object' || Array.isArray(t)) fail(`${keys.join('.')}: ${k} is not a table`);
    }
    const last = keys[keys.length - 1];
    if (Object.hasOwn(t, last)) fail(`duplicate key ${keys.join('.')}`);
    t[last] = v;
  }
  // Walks a header path; an array of tables on the way means its last table.
  const walk = (keys) => {
    let t = root;
    for (const k of keys) {
      if (t[k] === undefined) {
        t[k] = {};
        Object.defineProperty(t[k], LINE, { value: line });
      }
      t = Array.isArray(t[k]) ? t[k][t[k].length - 1] : t[k];
      if (typeof t !== 'object' || t === null) fail(`${keys.join('.')}: ${k} is not a table`);
    }
    return t;
  };

  for (;;) {
    skip(true);
    if (i >= src.length) return root;
    if (src[i] === '[') {
      const array = src[i + 1] === '[';
      i += array ? 2 : 1;
      skip(false);
      const keys = dottedKey();
      if (!src.startsWith(array ? ']]' : ']', i)) fail('unterminated table header');
      i += array ? 2 : 1;
      if (array) {
        const parent = walk(keys.slice(0, -1));
        const last = keys[keys.length - 1];
        if (parent[last] === undefined) parent[last] = [];
        if (!Array.isArray(parent[last])) fail(`${keys.join('.')} is not an array of tables`);
        current = {};
        Object.defineProperty(current, LINE, { value: line });
        parent[last].push(current);
      } else {
        current = walk(keys);
      }
    } else {
      const keys = dottedKey();
      skip(false);
      if (src[i] !== '=') fail('expected =');
      i++;
      skip(false);
      assign(current, keys, value());
    }
    skip(false);
    if (i < src.length && src[i] !== '\n') fail(`unexpected ${JSON.stringify(src.slice(i, i + 10))}`);
  }
}
