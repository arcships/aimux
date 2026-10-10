// Mock transport. `createMockFetch(canned)` returns a fetch-compatible function that
// records every outgoing request and answers with the canned response. It never touches
// the network: a request to any URL other than `canned.url`, or a second request, throws.

function lowerSortedHeaders(entries) {
  const out = {};
  for (const [k, v] of entries.map(([k, v]) => [String(k).toLowerCase(), String(v)]).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
    out[k] = v;
  }
  return out;
}

function headerEntries(headers) {
  if (headers == null) return [];
  if (typeof Headers !== 'undefined' && headers instanceof Headers) return [...headers.entries()];
  if (Array.isArray(headers)) return headers;
  return Object.entries(headers);
}

export function createMockFetch(canned) {
  const requests = [];

  async function mockFetch(input, init = {}) {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const method = (init.method ?? (typeof input === 'object' && input.method) ?? 'GET').toUpperCase();
    const rawEntries = headerEntries(init.headers);
    const headers = lowerSortedHeaders(rawEntries);

    let text;
    const b = init.body;
    if (b == null) text = undefined;
    else if (typeof b === 'string') text = b;
    else if (b instanceof Uint8Array || b instanceof ArrayBuffer) text = new TextDecoder().decode(b);
    else throw new Error(`mock fetch: unsupported request body type ${Object.prototype.toString.call(b)}`);

    let body = text;
    if (text !== undefined && (headers['content-type'] ?? '').includes('json')) {
      try {
        body = JSON.parse(text);
      } catch {
        body = text;
      }
    }
    requests.push({ method, url, headers, body, rawHeaderNames: rawEntries.map(([k]) => String(k)), rawHeaders: Object.fromEntries(rawEntries.map(([k, v]) => [String(k), String(v)])) });

    if (requests.length > 1 || !canned || url !== canned.url) {
      throw new Error(`mock fetch: unmocked request #${requests.length} ${method} ${url} (expected ${canned?.url ?? '<none>'})`);
    }

    const status = canned.status ?? 200;
    const respHeaders = { ...(canned.headers ?? {}) };
    const isSse = Object.entries(respHeaders).some(([k, v]) => k.toLowerCase() === 'content-type' && /text\/event-stream/i.test(v));
    let payload;
    if (typeof canned.body === 'string') {
      if (isSse) {
        const enc = new TextEncoder();
        const events = canned.body.split('\n\n').filter((e) => e.length > 0).map((e) => enc.encode(e + '\n\n'));
        payload = new ReadableStream({
          start(controller) {
            for (const e of events) controller.enqueue(e);
            controller.close();
          },
        });
      } else payload = canned.body;
    } else {
      payload = JSON.stringify(canned.body);
      if (!Object.keys(respHeaders).some((k) => k.toLowerCase() === 'content-type')) respHeaders['content-type'] = 'application/json';
    }
    const response = new Response(payload, { status, headers: respHeaders });
    const last = requests[requests.length - 1];
    last.responseStatus = response.status;
    last.responseHeaders = Object.fromEntries(response.headers.entries());
    return response;
  }

  mockFetch.requests = requests;
  mockFetch.canned = canned;
  return mockFetch;
}

export { lowerSortedHeaders };
