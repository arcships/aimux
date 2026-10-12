// Shared constants for the fixture recorder. Keys are obviously fake; they are
// redacted from every fixture and the recorder fails if one ever leaks.
export const API_KEY = 'sk-test-fixture';
export const ENV_KEY = 'sk-env-should-not-be-used';
export const FAKE_KEYS = [API_KEY, ENV_KEY];

// Deterministic clock: every `new Date()` / `Date.now()` returns this instant.
export const FIXED_NOW_MS = Date.UTC(2025, 0, 1, 0, 0, 0);

/** Build an SSE text body from event objects (JSON `data:` lines) or raw strings. */
export function sse(events) {
  return (
    events
      .map((e) => {
        if (typeof e === 'string') return e;
        const { event, data } = e;
        return `${event ? `event: ${event}\n` : ''}data: ${typeof data === 'string' ? data : JSON.stringify(data)}`;
      })
      .join('\n\n') + '\n\n'
  );
}

export const JSON_HEADERS = { 'content-type': 'application/json' };
export const SSE_HEADERS = {
  'cache-control': 'no-cache',
  'content-type': 'text/event-stream',
};
