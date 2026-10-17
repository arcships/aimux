# C and C++ primitives

The examples include the generated `aimux-ffi/include/aimux.h` and use only
ABI primitives. Both examples run offline and print the manifest and
runtime description. `aimux_raii.hpp` owns strings, errors, and handles;
its stream trampoline copies borrowed frame bytes, catches every C++ exception,
requests abort, and rethrows the first exception only after `aimux_stream` returns.
A stream wrapper caller must supply a live Abort handle for cancellation.
Callbacks must return promptly.

From the repository root:

```sh
scripts/test-c-abi.sh
scripts/test-c-abi.sh --asan
scripts/test-c-abi.sh --asan-lsan # Linux only
scripts/test-c-abi.sh --compile-only
scripts/test-c-abi.sh --asan-compile-only
```

`cargo build -p aimux-ffi` builds the cdylib; its artifact messages locate it
wherever `CARGO_TARGET_DIR` points. The script copies that library into its
temporary run directory before linking. All four
executables run and their exit statuses are reported, even if an earlier one
fails. To test a specific already built cdylib, set
`AIMUX_C_LIB_DIR` to its directory. `CC` and `CXX` select compilers;
`AIMUX_C_HEADER_DIR` explicitly selects a different generated header directory.
Compile-only mode neither links nor runs. Assertions remain enabled in every mode.

ASan instruments the C/C++ harnesses. A normal Rust cdylib is not itself
ASan-instrumented: full Rust bounds instrumentation requires a separately
instrumented Rust build, and passed through
`AIMUX_C_LIB_DIR`. LeakSanitizer is explicitly requested on Linux; macOS ASan
does not establish an LSan pass.

The C test exercises ownership on success/error paths, pointer and numeric output
sentinels, NULL arguments, invalid UTF-8, malformed JSON, unknown ops, oversized
binary lengths, an Abort-versus-model/provider/registry/operation wrong-kind
sample, and 100 concurrent drop/signal races. All returned errors and all owned
getter strings are freed. The oversized length test uses an invalid operation
handle, so either length rejection or handle rejection is acceptable; it does
not establish valid-operation backpressure or input-copy behavior. It never
allocates or pretends to supply SIZE_MAX bytes.

By default the accepted-stream test prints `SKIP`. To enable it, configure an
existing provider (preferably against a local deterministic fixture server):

```sh
export AIMUX_C_TEST_PROVIDER_PARAMS='<provider.create request JSON>'
export AIMUX_C_TEST_MODEL_PARAMS='<provider.model request JSON>'
export AIMUX_C_TEST_STREAM_PARAMS='<text.stream request JSON>'
scripts/test-c-abi.sh
```

The request DTOs come from the generated schema. Tests expect handle-producing
results to contain a decimal-string `handle` field. The stream test creates all
handles in its own process, copies frames while borrowed bytes are valid,
aborts inside the callback, requires exactly one final End frame, and checks
for late callbacks during a short observation interval. That interval is a
bounded check, not a proof against arbitrarily delayed callbacks. Rejection
before acceptance fails this configured test. Never put credentials in committed
fixture files.

The C++ test exercises move ownership, error unwinding, and the first-failure
latch by directly calling its `extern "C"` trampoline. It does not claim this
local trampoline test is an accepted native stream integration test. All Rust
error domains share one destructor; this suite checks reachable boundary and
protocol errors, not every AiMux/recording payload variant.
