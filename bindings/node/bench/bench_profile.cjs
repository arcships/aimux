// Paired micro-benchmark of one built Node binding (RFC-0043 B-profile).
//
// Usage: node bench/bench_profile.cjs /path/to/libaimux_node.so [COUNT]
//
// Loads the given cdylib through index.js (NAPI_RS_NATIVE_LIBRARY_PATH), then
// times six scenarios against in-process localhost mocks (no network):
// text.generate and text.stream over small and large bodies, model.info and
// provider.create+drop with no HTTP at all. Prints one JSON line with the
// median and p90 in microseconds per scenario and the library's byte size.
// Run two builds alternately, several rounds each, and compare medians.
const fs = require('fs'), os = require('os'), path = require('path'), http = require('http');
const so = process.argv[2], N = Number(process.argv[3] || 300);
if (!so) { console.error('usage: node bench/bench_profile.cjs <libaimux_node.so> [count]'); process.exit(2); }
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'aimux-bench-'));
const nodeFile = path.join(tmp, 'aimux.node');
fs.copyFileSync(so, nodeFile);
process.env.NAPI_RS_NATIVE_LIBRARY_PATH = nodeFile;
const { Runtime } = require(path.join(__dirname, '..', 'index.js'));

// Same bodies as bindings/python/tests/ops_fixtures.py.
const RESPONSE = { id: 'resp-test', object: 'response', model: 'gpt-4o', status: 'completed',
  output: [{ id: 'msg-test', type: 'message', role: 'assistant', status: 'completed',
    content: [{ type: 'output_text', text: 'Rust is a systems programming language.', annotations: [] }] }],
  usage: { input_tokens: 10, output_tokens: 8, total_tokens: 18 } };
const sse = (type, data) => `event: ${type}\ndata: ${JSON.stringify({ type, ...data })}\n\n`;
const delta = text => sse('response.output_text.delta', { output_index: 0, item_id: 'msg-test', content_index: 0, delta: text });
const streamBody = deltas => sse('response.created', { response: { id: 'resp-test', model: 'gpt-4o', created_at: 1234 } })
  + sse('response.output_item.added', { output_index: 0, item: { id: 'msg-test', type: 'message', role: 'assistant', content: [] } })
  + deltas.map(delta).join('')
  + sse('response.output_item.done', { output_index: 0, item: { id: 'msg-test', type: 'message', role: 'assistant', status: 'completed',
      content: [{ type: 'output_text', text: deltas.join(''), annotations: [] }] } })
  + sse('response.completed', { response: { id: 'resp-test', status: 'completed', usage: { input_tokens: 5, output_tokens: 2, total_tokens: 7 } } });
const heavy = JSON.parse(JSON.stringify(RESPONSE));
heavy.output[0].content[0].text = 'Rust is a systems programming language. '.repeat(1600); // ~64 KB

function serve(body, type) {
  return new Promise(resolve => {
    const server = http.createServer((req, res) => { req.resume(); req.on('end', () => { res.writeHead(200, { 'content-type': type }); res.end(body); }); });
    server.listen(0, '127.0.0.1', () => resolve([server, `http://127.0.0.1:${server.address().port}`]));
  });
}
const stats = t => { t.sort((a, b) => a - b); return { median_us: +(t[t.length >> 1] * 1e3).toFixed(1), p90_us: +(t[Math.floor(t.length * 0.9)] * 1e3).toFixed(1) }; };
async function timed(fn, warmup = 20) {
  for (let i = 0; i < warmup; i++) await fn();
  const t = [];
  for (let i = 0; i < N; i++) { const a = process.hrtime.bigint(); await fn(); t.push(Number(process.hrtime.bigint() - a) / 1e6); }
  return stats(t);
}
(async () => {
  const rt = new Runtime();
  const out = { library: so, bytes: fs.statSync(so).size };
  const opts = { maxRetries: 0, timeout: { totalMs: 5000 } };
  const open = async url => {
    const provider = JSON.parse(await rt.call('0', 'provider.create', JSON.stringify({ package: 'openai', settings: { apiKey: 'k', baseUrl: url } }))).handle;
    const model = JSON.parse(await rt.call(provider, 'provider.model', JSON.stringify({ method: 'languageModel', modelId: 'gpt-4o' }))).handle;
    return [provider, model];
  };
  const generate = async (body, label) => {
    const [server, url] = await serve(JSON.stringify(body), 'application/json'); const [provider, model] = await open(url);
    out[label] = await timed(() => rt.call(model, 'text.generate', JSON.stringify({ prompt: 'hi', options: opts })));
    if (label === 'text.generate') out['model.info'] = await timed(() => rt.call(model, 'model.info', '{}'));
    rt.dropHandle(model); rt.dropHandle(provider); server.close();
  };
  const stream = async (body, label) => {
    const [server, url] = await serve(body, 'text/event-stream'); const [provider, model] = await open(url);
    out[label] = await timed(async () => { for await (const _ of await rt.stream(model, 'text.stream', JSON.stringify({ prompt: 'hi', options: opts }))) {} });
    rt.dropHandle(model); rt.dropHandle(provider); server.close();
  };
  await generate(RESPONSE, 'text.generate');
  await stream(streamBody(['Hello', ' world']), 'text.stream');
  await generate(heavy, 'text.generate(64KB)');
  await stream(streamBody(Array(400).fill('word ')), 'text.stream(400 deltas)');
  out['provider.create+drop'] = await timed(async () => {
    const handle = JSON.parse(await rt.call('0', 'provider.create', JSON.stringify({ package: 'openai', settings: { apiKey: 'k' } }))).handle;
    rt.dropHandle(handle);
  });
  console.log(JSON.stringify(out));
})().catch(error => { console.error(error); process.exit(1); });
