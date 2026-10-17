#ifndef AIMUX_RAII_HPP
#define AIMUX_RAII_HPP
#include "aimux.h"
#include <cstdint>
#include <exception>
#include <functional>
#include <memory>
#include <stdexcept>
#include <string>
#include <utility>

namespace aimux {
struct StringDeleter {
    void operator()(char *value) const noexcept { aimux_free_string(value); }
};
struct ErrorDeleter {
    void operator()(aimux_error_t *value) const noexcept { if (value) aimux_error_free(value); }
};
using String = std::unique_ptr<char, StringDeleter>;
using Error = std::unique_ptr<aimux_error_t, ErrorDeleter>;

inline void check(aimux_error_t *raw) {
    Error error(raw);
    if (!error) return;
    String message(aimux_error_message(error.get()));
    throw std::runtime_error(message ? message.get() : "aimux call failed");
}
class Handle {
public:
    explicit Handle(std::uint64_t value = 0) noexcept : value_(value) {}
    ~Handle() { aimux_drop(value_); }
    Handle(const Handle &) = delete;
    Handle &operator=(const Handle &) = delete;
    Handle(Handle &&other) noexcept : value_(std::exchange(other.value_, 0)) {}
    Handle &operator=(Handle &&other) noexcept {
        if (this != &other) {
            aimux_drop(value_);
            value_ = std::exchange(other.value_, 0);
        }
        return *this;
    }
    std::uint64_t get() const noexcept { return value_; }
private:
    std::uint64_t value_;
};

struct StreamContext {
    std::function<void(const std::string &)> consume;
    std::uint64_t abort_handle;
    std::exception_ptr failure;
};
} // namespace aimux

/* Catch allocation and consumer exceptions before returning to the C ABI. */
extern "C" inline void aimux_cpp_event(const char *frame, void *opaque) noexcept {
    auto &ctx = *static_cast<aimux::StreamContext *>(opaque);
    if (ctx.failure) return;
    try {
        ctx.consume(std::string(frame));
    } catch (...) {
        ctx.failure = std::current_exception();
        aimux_abort(ctx.abort_handle);
    }
}
namespace aimux {
inline void stream(std::uint64_t model, const char *op, const char *params,
                   std::uint64_t abort_handle,
                   std::function<void(const std::string &)> consume) {
    StreamContext context{std::move(consume), abort_handle, nullptr};
    Error error(aimux_stream(model, op, params, abort_handle, aimux_cpp_event, &context));
    if (context.failure) std::rethrow_exception(context.failure);
    check(error.release());
}
} // namespace aimux
#endif
