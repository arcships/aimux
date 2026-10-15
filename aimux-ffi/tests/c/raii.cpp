#include "test_support.h"
#include "aimux_raii.hpp"
#include <cstring>
#include <type_traits>

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
    catch (const std::runtime_error &) { caught = true; }
    assert(caught);

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
    puts("PASS C++17 RAII / moves / exception latch (direct trampoline test)");
    return 0;
}
