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
