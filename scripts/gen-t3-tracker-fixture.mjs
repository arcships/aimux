// Regenerates fixtures/aisdk/provider-utils/t3-unattributable-deltas.json by
// replaying each case's deltas through the pinned @ai-sdk/provider-utils
// StreamingToolCallTracker with each case's typeValidation (default 'if-present').
// The pin is the "@ai-sdk/provider-utils" entry of fixtures/aisdk/VERSIONS.json.
// Usage: node scripts/gen-t3-tracker-fixture.mjs <node_modules dir with the pinned @ai-sdk/provider-utils>
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

const nm = process.argv[2];
if (!nm) throw new Error('usage: node scripts/gen-t3-tracker-fixture.mjs <node_modules dir>');
const versions = JSON.parse(
  fs.readFileSync(new URL('../fixtures/aisdk/VERSIONS.json', import.meta.url), 'utf8'),
);
const pinned = versions['@ai-sdk/provider-utils']?.version;
if (!pinned) throw new Error('VERSIONS.json has no @ai-sdk/provider-utils version');
const req = createRequire(path.join(path.resolve(nm), 'x.js'));
const pkgJson = req.resolve('@ai-sdk/provider-utils/package.json');
const pkg = req(pkgJson);
if (pkg.version !== pinned) {
  throw new Error(`expected provider-utils ${pinned} (VERSIONS.json), got ${pkg.version}`);
}
const trackerSource = fs.readFileSync(
  path.join(path.dirname(pkgJson), 'src/streaming-tool-call-tracker.ts'),
);
const { StreamingToolCallTracker } = req('@ai-sdk/provider-utils');
const out = new URL('../fixtures/aisdk/provider-utils/t3-unattributable-deltas.json', import.meta.url);
const fx = JSON.parse(fs.readFileSync(out, 'utf8'));
const sha256 = createHash('sha256').update(trackerSource).digest('hex');
fx.source = `@ai-sdk/provider-utils@${pkg.version} src/streaming-tool-call-tracker.ts (sha256 ${sha256})`;
for (const c of fx.cases) {
  const steps = [];
  let parts = [];
  let n = 0;
  const t = new StreamingToolCallTracker({ enqueue: p => parts.push(p) }, {
    typeValidation: c.typeValidation ?? 'if-present', generateId: () => `gen-${n++}`,
  });
  let error = null;
  for (const d of c.deltas) {
    parts = [];
    try { t.processDelta(d); } catch (e) { error = { name: e.name, message: e.message }; }
    steps.push({ parts });
    if (error) break;
  }
  let flush = null;
  if (!error) { parts = []; t.flush(); flush = parts; }
  c.aisdk = { steps, error, flush };
}
fs.writeFileSync(out, JSON.stringify(fx, null, 2) + '\n');
