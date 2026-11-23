#include "test_support.h"
#include "aimux_raii.hpp"
#include <cstring>
#include <future>
#include <type_traits>

using Completion = std::promise<std::pair<aimux::String, aimux::Error>>;

extern "C" void complete_cpp_call(char *result, aimux_error_t *error, void *data) noexcept {
    std::unique_ptr<Completion> completion(static_cast<Completion *>(data));
    assert((result == nullptr) != (error == nullptr));
    completion->set_value({aimux::String(result), aimux::Error(error)});
}

static void async_ownership() {
    for (const char *op : {"runtime.describe", "not.an.op"}) {
        auto completion = new Completion;
        auto future = completion->get_future();
        aimux::Error submission(aimux_call_async(0, op, "{}", 0, complete_cpp_call, completion));
        if (submission) {
            delete completion;
            assert(!submission);
        }
        assert(future.wait_for(std::chrono::seconds(5)) == std::future_status::ready);
        auto [result, error] = future.get();
        if (std::strcmp(op, "runtime.describe") == 0) {
            assert(result && !error && std::strstr(result.get(), "\"abiVersion\":65536"));
        } else {
            assert(!result && error && aimux_error_code(error.get()) == 300);
        }
    }
}

int main() {
    static_assert(!std::is_copy_constructible<aimux::Handle>::value, "unique handle ownership");
    static_assert(std::is_nothrow_move_constructible<aimux::Handle>::value, "noexcept move");
    static_assert(noexcept(aimux_cpp_event(nullptr, nullptr)), "callback cannot unwind");
    char *raw = nullptr;
    aimux::check(aimux_manifest_json(&raw));
    aimux::String manifest(raw);
    assert(manifest && std::strstr(manifest.get(), "opsVersion"));
    raw = nullptr;
    aimux::Error error(aimux_call(0, "not.an.op", "{}", 0, &raw));
    assert(raw == nullptr);
    check_error(error.release(), 300);
    bool caught = false;
    try { aimux::check(aimux_call(0, "not.an.op", "{}", 0, &raw)); }
    catch (const aimux::NativeError &e) {
        caught = e.code() == 300 && e.json().find("\"InvalidOp\"") != std::string::npos;
    }
    assert(caught);
    // The cause chain is in the envelope JSON: the serde error under
    // InvalidWireJson is `error.cause`, an underlying `{message}`.
    caught = false;
    try { aimux::check(aimux_call(0, "abort.create", "{", 0, &raw)); }
    catch (const aimux::NativeError &e) {
        const std::string &json = e.json();
        auto cause = json.find("\"cause\":{");
        caught = e.code() == 202 && json.find("\"InvalidWireJson\"") != std::string::npos &&
                 cause != std::string::npos &&
                 json.find("\"message\":\"EOF while parsing", cause) != std::string::npos;
    }
    assert(caught && raw == nullptr);

    aimux::Handle first(create_handle("abort.create", 0, "{}"));
    auto id = first.get();
    aimux::Handle second(std::move(first));
    assert(first.get() == 0 && second.get() == id);
    aimux::Handle third(create_handle("abort.create", 0, "{}"));
    auto old = third.get();
    third = std::move(second);
    assert(second.get() == 0 && third.get() == id);
    raw = nullptr;
    check_error(aimux_call(old, "abort.signal", "{}", 0, &raw), 203);
    assert(raw == nullptr);

    unsigned deliveries = 0;
    aimux::StreamContext ctx{[&deliveries](const std::string &) {
        ++deliveries;
        throw std::runtime_error("consumer failure");
    }, third.get(), nullptr};
    aimux_cpp_event("{\"type\":\"event\",\"seq\":\"1\",\"event\":{}}", &ctx);
    assert(ctx.failure && deliveries == 1);
    aimux_cpp_event("{\"type\":\"end\",\"seq\":\"2\",\"outcome\":\"cancelled\"}", &ctx);
    assert(deliveries == 1);
    caught = false;
    try { std::rethrow_exception(ctx.failure); }
    catch (const std::runtime_error &e) { caught = std::string(e.what()) == "consumer failure"; }
    assert(caught);
    async_ownership();
    puts("PASS C++17 RAII / moves / exception latch (direct trampoline test)");
    return 0;
}
