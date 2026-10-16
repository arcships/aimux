// node --test scripts/fetch-aisdk.test.mjs
//
// Runs scripts/fetch-aisdk.sh against a throwaway npm registry served from
// this process, with a temporary npm cache and empty npm config files, so the
// tests use no network and leave target/aisdk alone.
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, 'fetch-aisdk.sh');

let root;
let server;
let registryUrl;
/** `name -> version -> tarball bytes` served by the registry. */
const published = new Map();
/** Paths the registry was asked for. */
const requests = [];

before(async () => {
  root = mkdtempSync(join(tmpdir(), 'fetch-aisdk-test-'));
  server = createServer((req, res) => {
    requests.push(req.url);
    const path = decodeURIComponent(req.url.split('?')[0].slice(1));
    const tarball = /^tarballs\/(.+)\/-\/([^/]+)\.tgz$/.exec(path);
    if (tarball) {
      const bytes = published.get(tarball[1])?.get(tarball[2]);
      if (!bytes) return res.writeHead(404).end();
      return res.writeHead(200, { 'content-type': 'application/octet-stream' }).end(bytes);
    }
    const versions = published.get(path);
    if (!versions) return res.writeHead(404, { 'content-type': 'application/json' }).end('{}');
    const doc = { name: path, 'dist-tags': {}, versions: {} };
    for (const [version, bytes] of versions) {
      doc['dist-tags'].latest = version;
      doc.versions[version] = {
        name: path,
        version,
        dist: { tarball: `${registryUrl}tarballs/${path}/-/${version}.tgz`, integrity: sri(bytes) },
      };
    }
    res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify(doc));
  });
  await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
  registryUrl = `http://127.0.0.1:${server.address().port}/`;
});

after(() => {
  server?.close();
  if (root) rmSync(root, { recursive: true, force: true });
});

function sri(bytes) {
  return `sha512-${createHash('sha512').update(bytes).digest('base64')}`;
}

/** A gzipped npm tarball whose package.json says `name@version`, plus `index.js`. */
function pack(name, version, body = `module.exports = ${JSON.stringify(`${name}@${version}`)};\n`) {
  const dir = mkdtempSync(join(root, 'src-'));
  mkdirSync(join(dir, 'package'));
  writeFileSync(join(dir, 'package', 'package.json'), JSON.stringify({ name, version }) + '\n');
  writeFileSync(join(dir, 'package', 'index.js'), body);
  execFileSync('tar', ['-czf', join(dir, 'out.tgz'), '-C', dir, 'package']);
  return readFileSync(join(dir, 'out.tgz'));
}

/** Serve `bytes` as `name@version`; returns its integrity. */
function publish(name, version, bytes) {
  if (!published.has(name)) published.set(name, new Map());
  published.get(name).set(version, bytes);
  return sri(bytes);
}

/** Run the script with `versions` (an object, or raw text) into a fresh destination. */
async function fetchInto(versions, dest = mkdtempSync(join(root, 'dest-'))) {
  const dir = mkdtempSync(join(root, 'run-'));
  const versionsPath = join(dir, 'VERSIONS.json');
  writeFileSync(versionsPath, typeof versions === 'string' ? versions : JSON.stringify(versions, null, 2));
  // Two files: npm refuses to load one file as both the user and the global config.
  writeFileSync(join(dir, 'user-npmrc'), '');
  writeFileSync(join(dir, 'global-npmrc'), '');
  // npm reads npm_config_* in any case, so an inherited NPM_CONFIG_USERCONFIG
  // would win over the settings below.
  const inherited = Object.entries(process.env).filter(([k]) => !/^npm_config_/i.test(k));
  const env = {
    ...Object.fromEntries(inherited),
    AISDK_VERSIONS: versionsPath,
    AISDK_DEST: dest,
    npm_config_registry: registryUrl,
    npm_config_cache: join(dir, 'npm-cache'),
    npm_config_userconfig: join(dir, 'user-npmrc'),
    npm_config_globalconfig: join(dir, 'global-npmrc'),
    npm_config_noproxy: '127.0.0.1,localhost',
    NO_PROXY: '127.0.0.1,localhost',
    no_proxy: '127.0.0.1,localhost',
    npm_config_update_notifier: 'false',
    npm_config_fund: 'false',
    npm_config_audit: 'false',
  };
  const child = spawn('bash', [script], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (d) => (stdout += d));
  child.stderr.on('data', (d) => (stderr += d));
  const code = await new Promise((ok) => child.on('close', ok));
  return { code, stdout, stderr, dest };
}

/** Every file under `dir` as `{relativePath: content}`. */
function tree(dir) {
  const out = {};
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else out[relative(dir, p)] = readFileSync(p, 'utf8');
    }
  };
  if (existsSync(dir)) walk(dir);
  return out;
}

test('places every pinned package, and a second run leaves the same trees', async () => {
  const versions = {
    '@aimux-test/alpha': { version: '1.0.0', integrity: publish('@aimux-test/alpha', '1.0.0', pack('@aimux-test/alpha', '1.0.0')) },
    'aimux-test-beta': { version: '2.1.0', integrity: publish('aimux-test-beta', '2.1.0', pack('aimux-test-beta', '2.1.0')) },
  };
  const first = await fetchInto(versions);
  assert.equal(first.code, 0, first.stderr);
  const placed = tree(first.dest);
  assert.deepEqual(Object.keys(placed).sort(), [
    '@aimux-test/alpha@1.0.0/index.js',
    '@aimux-test/alpha@1.0.0/package.json',
    'aimux-test-beta@2.1.0/index.js',
    'aimux-test-beta@2.1.0/package.json',
  ]);
  assert.equal(placed['@aimux-test/alpha@1.0.0/index.js'], 'module.exports = "@aimux-test/alpha@1.0.0";\n');

  const second = await fetchInto(versions, first.dest);
  assert.equal(second.code, 0, second.stderr);
  assert.deepEqual(tree(first.dest), placed);
});

test('a tarball whose sha512 differs from the pin is not placed', async () => {
  publish('@aimux-test/gamma', '1.0.0', pack('@aimux-test/gamma', '1.0.0'));
  const other = sri(pack('@aimux-test/gamma', '1.0.0', 'tampered\n'));
  const run = await fetchInto({ '@aimux-test/gamma': { version: '1.0.0', integrity: other } });
  assert.notEqual(run.code, 0);
  assert.match(run.stderr, /@aimux-test\/gamma@1\.0\.0: integrity mismatch/);
  assert.deepEqual(readdirSync(run.dest), []);
});

test('a tarball whose package.json names another version is not placed', async () => {
  const integrity = publish('aimux-test-delta', '1.0.0', pack('aimux-test-delta', '1.0.1'));
  const run = await fetchInto({ 'aimux-test-delta': { version: '1.0.0', integrity } });
  assert.notEqual(run.code, 0);
  assert.match(run.stderr, /version mismatch: wanted aimux-test-delta@1\.0\.0, tarball is aimux-test-delta@1\.0\.1/);
  assert.deepEqual(readdirSync(run.dest), []);
});

test('only the {version, integrity} object form is accepted, before any download', async () => {
  const integrity = publish('aimux-test-epsilon', '1.0.0', pack('aimux-test-epsilon', '1.0.0'));
  const sha1 = `sha1-${createHash('sha1').update('x').digest('base64')}`;
  const cases = {
    'bare version string': { 'aimux-test-epsilon': '1.0.0' },
    'no integrity': { 'aimux-test-epsilon': { version: '1.0.0' } },
    'sha1 integrity': { 'aimux-test-epsilon': { version: '1.0.0', integrity: sha1 } },
    'version range': { 'aimux-test-epsilon': { version: '^1.0.0', integrity } },
    'path in the package name': { '../aimux-test-epsilon': { version: '1.0.0', integrity } },
    'one bad entry among good ones': {
      'aimux-test-epsilon': { version: '1.0.0', integrity },
      'aimux-test-zeta': '1.0.0',
    },
    'empty object': {},
    'array': '[]',
    'not JSON': 'aimux-test-epsilon@1.0.0',
  };
  for (const [name, versions] of Object.entries(cases)) {
    const seen = requests.length;
    const dest = join(mkdtempSync(join(root, 'dest-')), 'aisdk');
    const run = await fetchInto(versions, dest);
    assert.notEqual(run.code, 0, name);
    assert.match(run.stderr, /^fetch-aisdk: /, name);
    assert.equal(requests.length, seen, `${name}: no request before the file is checked`);
    assert.equal(existsSync(dest), false, `${name}: destination not created`);
  }
});

test('a failing package leaves every destination as it was', async () => {
  // eta sorts and is fetched first and verifies; theta then fails its check.
  const eta = publish('@aimux-test/eta', '1.0.0', pack('@aimux-test/eta', '1.0.0'));
  publish('aimux-test-theta', '1.0.0', pack('aimux-test-theta', '1.0.0'));
  const thetaPin = sri(pack('aimux-test-theta', '1.0.0', 'tampered\n'));

  const dest = mkdtempSync(join(root, 'dest-'));
  for (const spec of ['@aimux-test/eta@1.0.0', 'aimux-test-theta@1.0.0']) {
    mkdirSync(join(dest, spec), { recursive: true });
    writeFileSync(join(dest, spec, 'index.js'), `old ${spec}\n`);
  }
  const old = tree(dest);

  const run = await fetchInto(
    {
      '@aimux-test/eta': { version: '1.0.0', integrity: eta },
      'aimux-test-theta': { version: '1.0.0', integrity: thetaPin },
    },
    dest,
  );
  assert.notEqual(run.code, 0);
  assert.match(run.stderr, /aimux-test-theta@1\.0\.0: integrity mismatch/);
  assert.match(run.stdout, /fetch-aisdk: @aimux-test\/eta@1\.0\.0\n/, 'eta was fetched before theta failed');
  assert.deepEqual(tree(dest), old);
  assert.deepEqual(readdirSync(dest).sort(), ['@aimux-test', 'aimux-test-theta@1.0.0']);
});
