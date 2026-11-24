# Benchmarks

`bench_profile.cjs` times one built binding against in-process localhost mocks
(six scenarios, no network) and prints a JSON line; it is the comparison behind
the binding's `[profile.release]` (RFC-0043 B-profile). Run two builds
alternately for several rounds and compare medians:

```sh
npm run build   # napi build --release -> ./aimux.<platform>.node and target/release/libaimux_node.so
node bench/bench_profile.cjs target/release/libaimux_node.so 300
```

The one-shot API benchmarks were retired by the ops binding. Their source remains in [git history](https://github.com/arcships/aimux/tree/96dba455d0bec2be322c361ff49334c0e88bcf32/bindings/node/bench).

## RFC-0049 / RFC-0050 data plane

From the repository root, build the native addon and public TypeScript facade
for each revision. Run the same copy of the harness against both builds:

```sh
npm ci --prefix bindings/node
npm --prefix bindings/node run build
npm --prefix bindings/node run build:typed
TOKIO_WORKER_THREADS=2 node bindings/node/bench/bench_data_plane.cjs bindings/node request-200k 100 4 20
TOKIO_WORKER_THREADS=2 node bindings/node/bench/bench_data_plane.cjs bindings/node stream-burst 10 1 2
```

Use `request-small` or `request-200k` for concurrency scans. `generate-burst`
measures small requests after a pause following warmup. For the connection
pool case, supply 64 concurrent requests and a warmup of at least 64.
`stream-burst` checks the sequence, 5,000 text deltas, usage, and terminal frame.
`stream-paced` spaces eight deltas by 20 ms and reports consumer-side gaps.
The mock is a separate loopback process. `delta_window_ns` covers the first
through last text delta, while `ttft_ns` starts at the SDK call. Return
size is sampled with a separate raw call and includes actual wire bytes plus
a size with variable step timing removed. On a build with the `include`
option, `request-200k-no-bodies` also checks the body fields are absent and
enforces the 360,000-byte phase 1 budget.

For direct Rust, copy the same example source to both revisions, build once
per revision, and keep each resulting executable separately:

```sh
cargo build --release -p aimux-providers --example perf_data_plane
target/release/examples/perf_data_plane request-small 100 20
target/release/examples/perf_data_plane request-200k 100 20
target/release/examples/perf_data_plane serialize-200k 100 20
target/release/examples/perf_data_plane gen-tools-loop 100 20
target/release/examples/perf_data_plane stream-1k-chunks 30 5
```

The Rust example runs on one current-thread Tokio runtime and uses a canned
in-memory Fetch. The tools fixture is built once and the resulting
`GenerateTextOptions` is cloned on every call, as in a Rust caller reusing its
configuration. Timing and allocation counting use separate passes. Counts
include runtime work during the operation, allocations and reallocations,
and result cleanup, with cumulative requested allocation bytes. For the
stream case counts cover a complete 1,000-delta operation including startup
and teardown. `serialize-200k` prepares the result before timing and measures
encoding and releasing the output buffer.

Run each recipe in fresh processes with seeded AB/BA ordering and keep the
raw samples and hashes. For example:

```sh
python3 scripts/run-perf-pairs.py \
  --command '{artifact} gen-tools-loop 100 20' \
  --base /tmp/aimux-base --base-sha "$BASE_SHA" --base-digest-file /tmp/aimux-base \
  --head /tmp/aimux-head --head-sha "$HEAD_SHA" --head-digest-file /tmp/aimux-head \
  --recipe-file aimux-providers/examples/perf_data_plane.rs \
  --recipe-file Cargo.lock --output /tmp/aimux-tools-pairs.jsonl
```

For Node, substitute each built package directory and hash its native addon,
the typed facade, the driver, and the dependency lockfile. The example sample
counts are for local smoke runs. RFC-0043 §7's registered isolated environment,
runtime versions, affinity, duration, sample counts, and anchor comparison are
required for release evidence.
