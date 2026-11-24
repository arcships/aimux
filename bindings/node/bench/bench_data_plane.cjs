// Run the same harness against the built package of each revision.
const assert = require('node:assert/strict');
const { fork } = require('node:child_process');
const { once } = require('node:events');
const http = require('node:http');
const path = require('node:path');

const chunkCount = 5000;
const chunkText = 'word\n';
const pacedChunkCount = 8;
const operationTimeoutMs = 15000;

function response(text) {
  return {
    id: 'resp-bench', object: 'response', model: 'gpt-4o', status: 'completed',
    output: [{
      id: 'msg-bench', type: 'message', role: 'assistant', status: 'completed',
      content: [{ type: 'output_text', text, annotations: [] }],
    }],
    usage: { input_tokens: 10, output_tokens: 8, total_tokens: 18 },
  };
}

function sse(type, fields) {
  return `event: ${type}\ndata: ${JSON.stringify({ type, ...fields })}\n\n`;
}

async function serve() {
  const small = JSON.stringify(response('hello'));
  const large = JSON.stringify(response('b'.repeat(50000)));
  const completed = response(chunkText.repeat(chunkCount));
  const paced = response(chunkText.repeat(pacedChunkCount));
  const start = sse('response.created', {
    response: { id: 'resp-bench', model: 'gpt-4o', created_at: 1234 },
  }) + sse('response.output_item.added', {
    output_index: 0, item: { ...completed.output[0], status: 'in_progress', content: [] },
  });
  const delta = sse('response.output_text.delta', {
    output_index: 0, item_id: 'msg-bench', content_index: 0, delta: chunkText,
  });
  const burst = delta.repeat(32);
  const finish = message => sse('response.output_item.done', { output_index: 0, item: message.output[0] })
    + sse('response.completed', { response: message });

  const server = http.createServer(async (req, res) => {
    try {
      const chunks = [];
      for await (const bytes of req) chunks.push(bytes);
      const request = JSON.parse(Buffer.concat(chunks));
      assert.equal(req.method, 'POST');
      assert.match(req.url, /\/responses$/);
      const input = JSON.stringify(request.input);
      if (request.stream === true) {
        res.writeHead(200, { 'content-type': 'text/event-stream' });
        res.write(start);
        if (input.includes('bench-paced')) {
          for (let i = 0; i < pacedChunkCount; i++) {
            await new Promise(resolve => setTimeout(resolve, 20));
            res.write(delta);
          }
          res.end(finish(paced));
          return;
        }
        // Each write contains at most 32 complete SSE events. The TCP layer
        // may coalesce writes; the same fixture and write policy serve A/B.
        let i = 0;
        for (; i + 32 <= chunkCount; i += 32) {
          if (!res.write(burst)) await once(res, 'drain');
        }
        res.end(delta.repeat(chunkCount - i) + finish(completed));
      } else {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(input.length >= 200000 ? large : small);
      }
    } catch (error) {
      res.destroy(error);
      throw error;
    }
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  process.send(server.address().port);
  process.on('disconnect', () => {
    server.closeAllConnections();
    server.close();
  });
}

async function measure() {
  const [packageDir, scenario, sampleArg, concurrencyArg = '1', warmupArg = '20'] = process.argv.slice(2);
  const samples = Number(sampleArg);
  const concurrency = Number(concurrencyArg);
  const warmup = Number(warmupArg);
  assert.ok(packageDir && ['request-small', 'request-200k', 'request-200k-no-bodies', 'generate-burst', 'stream-burst', 'stream-paced'].includes(scenario),
    'usage: node bench_data_plane.cjs <built package dir> <scenario> <samples> [concurrency] [warmup]');
  for (const n of [samples, concurrency]) assert.ok(Number.isSafeInteger(n) && n > 0);
  assert.ok(Number.isSafeInteger(warmup) && warmup >= 0);
  assert.ok(concurrency <= samples);

  const mock = fork(__filename, ['mock'], { stdio: ['ignore', 'ignore', 'inherit', 'ipc'] });
  const failure = once(mock, 'exit').then(([code]) => {
    throw new Error(`benchmark mock exited early (code ${code})`);
  });
  try {
    const [port] = await Promise.race([once(mock, 'message'), failure]);
    const { Client } = require(path.resolve(packageDir, 'dist', 'index.js'));
    const client = new Client();
    const provider = await client.provider({
      package: 'openai', settings: { apiKey: 'benchmark', baseUrl: `http://127.0.0.1:${port}/v1` },
    });
    const model = await provider.model('gpt-4o', 'languageModel');
    try {
      const suppressBodies = scenario === 'request-200k-no-bodies';
      const large = scenario === 'request-200k' || suppressBodies;
      const paced = scenario === 'stream-paced';
      const prompt = large ? 'a'.repeat(200000) : paced ? 'bench-paced' : 'hi';
      const expected = large ? 'b'.repeat(50000) : 'hello';
      const streaming = paced || scenario === 'stream-burst';
      const options = { maxRetries: 0, timeout: { totalMs: operationTimeoutMs } };
      if (suppressBodies) options.include = { requestBody: false, responseBody: false };
      const generate = async () => {
        const result = await model.generate(prompt, options);
        assert.equal(result.text, expected);
        assert.equal(result.steps.length, 1);
        assert.equal(result.totalUsage.totalTokens, 18);
      };
      const stream = async started => {
        let deltas = 0;
        let frames = 0;
        let ended = false;
        let finished = false;
        let first;
        let last;
        const deltaGaps = [];
        for await (const frame of await model.stream(prompt, options)) {
          assert.ok(!ended, 'received a frame after End');
          assert.equal(frame.seq, String(frames++));
          if (frame.type === 'end') {
            assert.equal(frame.outcome, 'success');
            ended = true;
          } else {
            const { event } = frame;
            assert.notEqual(event.type, 'error');
            if (event.type === 'finish') {
              assert.equal(event.totalUsage.totalTokens, 18);
              finished = true;
            }
            if (event.type === 'text-delta') {
              assert.equal(event.text, chunkText);
              deltas++;
              const received = process.hrtime.bigint();
              if (paced && last !== undefined) deltaGaps.push(Number(received - last));
              last = received;
              first ??= last;
            }
          }
        }
        assert.equal(deltas, paced ? pacedChunkCount : chunkCount);
        assert.ok(ended && finished);
        return {
          frames, text_deltas: deltas, ttft_ns: Number(first - started),
          delta_window_ns: Number(last - first), ...(paced && { delta_gaps_ns: deltaGaps }),
        };
      };
      const run = async () => {
        const start = process.hrtime.bigint();
        const details = streaming ? await stream(start) : await generate();
        return { elapsed_ns: Number(process.hrtime.bigint() - start), ...details };
      };
      const workload = async n => {
        const result = new Array(n);
        let next = 0;
        await Promise.all(Array.from({ length: concurrency }, async () => {
          while (next < n) {
            const i = next++;
            result[i] = await run();
          }
        }));
        return result;
      };

      await Promise.race([workload(warmup), failure]);
      if (scenario === 'generate-burst') await new Promise(resolve => setTimeout(resolve, 250));
      const began = process.hrtime.bigint();
      const cpu = process.cpuUsage();
      const observations = await Promise.race([workload(samples), failure]);
      const elapsed = Number(process.hrtime.bigint() - began);
      const usage = process.cpuUsage(cpu);
      // Count actual wire bytes in an additional, untimed raw call. Payload
      // includes any varying timing metadata carried by this revision.
      let wireBytes;
      let normalizedWireBytes;
      const framesPerSecond = streaming
        ? observations.reduce((sum, observation) => sum + observation.frames, 0) * 1e9 / elapsed
        : undefined;
      if (!streaming) {
        const raw = await client.raw.call(model.handle, 'text.generate', JSON.stringify({ prompt, options }));
        wireBytes = Buffer.byteLength(raw, 'utf8');
        const result = JSON.parse(raw);
        if (suppressBodies) {
          for (const step of result.steps) {
            assert.ok(!Object.hasOwn(step.request, 'body'));
            assert.ok(!Object.hasOwn(step.response, 'body'));
          }
          assert.ok(wireBytes <= 360000, `unexpected return size: ${wireBytes} bytes`);
        }
        if (process.env.AIMUX_STAGE2_GATE === '1' && scenario === 'request-200k') {
          assert.deepEqual(Object.keys(result).sort(), ['steps', 'totalUsage']);
          for (const step of result.steps) {
            assert.ok(!Object.hasOwn(step.request, 'body'));
            assert.ok(!Object.hasOwn(step.response, 'body'));
          }
        }
        for (const step of result.steps) delete step.performance;
        normalizedWireBytes = Buffer.byteLength(JSON.stringify(result), 'utf8');
      }
      if (process.env.AIMUX_STAGE2_GATE === '1') {
        if (scenario === 'request-200k') {
          assert.ok(wireBytes <= 110000, `Stage 2 JSON size exceeds 110,000 B: ${wireBytes} B`);
        }
        if (scenario === 'stream-burst') {
          assert.equal(process.env.TOKIO_WORKER_THREADS, '2');
          assert.ok(framesPerSecond >= 150000, `Stage 2 stream is below 150,000 frames/s: ${framesPerSecond}`);
        }
      }
      console.log(JSON.stringify({
        scenario, samples, concurrency, warmup, observations, elapsed_ns: elapsed,
        process_cpu_us: usage.user + usage.system, requests_per_s: samples * 1e9 / elapsed,
        process_peak_rss_kib: process.resourceUsage().maxRSS, wire_bytes: wireBytes, wire_bytes_without_step_timing: normalizedWireBytes,
        frames_per_s: framesPerSecond, operation_timeout_ms: operationTimeoutMs,
      }));
    } finally {
      model.close();
      provider.close();
    }
  } finally {
    if (mock.connected) mock.disconnect();
    if (mock.exitCode === null && mock.signalCode === null) await once(mock, 'exit');
  }
}

(process.argv[2] === 'mock' ? serve() : measure()).catch(error => {
  console.error(error);
  process.exitCode = 1;
});
