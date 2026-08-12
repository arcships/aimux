// example.cpp — C++ RAII wrapper example using aimux-ffi
//
// Build: g++ -std=c++17 -o example_cpp example.cpp -I../../aimux-ffi -L../../target/release -laimux_ffi -lpthread -ldl -lm
// Run:   OPENAI_API_KEY=sk-... ./example_cpp

#include <cstdlib>
#include <cstring>
#include <functional>
#include <iostream>
#include <stdexcept>
#include <string>

#include "aimux-error.h"
#include "aimux-ffi.h"

// Thin exception owning a copy of the AimuxError fields.
class AimuxException : public std::runtime_error {
public:
    // Consumes e: copies the fields and releases e.message.
    AimuxException(const std::string &context, AimuxError &e)
        : std::runtime_error(context + ": " + (e.message ? e.message : "failed")),
          code_(e.code), status_(e.status), retry_ms_(e.retry_ms),
          error_value_(e.error_value ? e.error_value : "") {
        aimux_free_string(e.message);
        aimux_free_string(e.error_value);
        e.message = nullptr;
        e.error_value = nullptr;
    }

    // For failures synthesized on the C++ side (no C struct involved).
    AimuxException(const std::string &context, AimuxErrorCode code, const std::string &msg)
        : std::runtime_error(context + ": " + msg), code_(code) {}

    AimuxErrorCode code() const { return code_; }
    int status() const { return status_; }
    int64_t retryMs() const { return retry_ms_; }
    /** Lossless AiMuxError JSON, or "" for FFI-synthesized failures. */
    const std::string &errorValue() const { return error_value_; }

private:
    AimuxErrorCode code_ = AIMUX_E_UNKNOWN;
    int status_ = -1;
    int64_t retry_ms_ = -1;
    std::string error_value_;
};

static void throw_if_failed(bool failed, AimuxError &err, const std::string &what) {
    if (failed) {
        throw AimuxException(what, err);
    }
}

static uint64_t take_handle(uint64_t h, AimuxError &err, const std::string &what) {
    throw_if_failed(h == 0, err, what);
    return h;
}

// RAII wrapper for model handle
class AimuxModel {
public:
    static AimuxModel openai(const std::string &api_key, const std::string &model_id) {
        AimuxError err{};
        auto handle =
            take_handle(aimux_openai_new(api_key.c_str(), model_id.c_str(), &err), err, "openai");
        return AimuxModel(handle);
    }

    static AimuxModel anthropic(const std::string &api_key, const std::string &model_id) {
        AimuxError err{};
        auto handle = take_handle(
            aimux_anthropic_new(api_key.c_str(), model_id.c_str(), &err), err, "anthropic");
        return AimuxModel(handle);
    }

    static AimuxModel provider(const std::string &name, const std::string &model_id,
                               const char *api_key = nullptr,
                               const char *config_json = nullptr) {
        AimuxError err{};
        auto handle = take_handle(
            aimux_provider_new(name.c_str(), api_key, model_id.c_str(), config_json, &err), err,
            "provider '" + name + "'");
        return AimuxModel(handle);
    }

    ~AimuxModel() {
        if (handle_) {
            aimux_drop_handle(handle_);
        }
    }

    AimuxModel(const AimuxModel &) = delete;
    AimuxModel &operator=(const AimuxModel &) = delete;
    AimuxModel(AimuxModel &&other) noexcept : handle_(other.handle_) { other.handle_ = 0; }

    std::string generate_text(const std::string &prompt_json,
                              const std::string &opts_json = "") {
        require_open("generate_text");
        const char *opts = opts_json.empty() ? nullptr : opts_json.c_str();
        AimuxError err{};
        char *result = aimux_generate_text(handle_, prompt_json.c_str(), opts, &err);
        throw_if_failed(result == nullptr, err, "generate_text");
        std::string s(result);
        aimux_free_string(result);
        return s;
    }

    std::string generate_text_as_openai(const std::string &prompt_json,
                                        const std::string &opts_json = "") {
        require_open("generate_text_as_openai");
        const char *opts = opts_json.empty() ? nullptr : opts_json.c_str();
        AimuxError err{};
        char *result =
            aimux_generate_text_as_openai(handle_, prompt_json.c_str(), opts, &err);
        throw_if_failed(result == nullptr, err, "generate_text_as_openai");
        std::string s(result);
        aimux_free_string(result);
        return s;
    }

    void stream_text(const std::string &prompt_json,
                     std::function<void(const std::string &)> on_part,
                     const std::string &opts_json = "") {
        require_open("stream_text");
        const char *opts = opts_json.empty() ? nullptr : opts_json.c_str();
        AimuxError err{};
        auto *ctx = &on_part;
        auto part_cb = [](const char *json, void *stream_ctx) {
            auto *fn = static_cast<std::function<void(const std::string &)> *>(stream_ctx);
            (*fn)(json ? json : "");
        };
        auto done_cb = [](void *) {};
        int rc = aimux_stream_text(handle_, prompt_json.c_str(), opts, part_cb, done_cb, ctx,
                                   &err);
        throw_if_failed(rc == 0, err, "stream_text");
    }

private:
    explicit AimuxModel(uint64_t handle) : handle_(handle) {}

    void require_open(const char *what) {
        if (handle_ == 0) {
            throw AimuxException(what, AIMUX_E_INVALID_ARGUMENT, "handle is closed");
        }
    }

    uint64_t handle_ = 0;
};

int main() {
    try {
        const char *api_key = std::getenv("OPENAI_API_KEY");
        if (!api_key) {
            throw AimuxException("openai", AIMUX_E_INVALID_ARGUMENT,
                                 "OPENAI_API_KEY is not set");
        }

        AimuxModel model = AimuxModel::openai(api_key, "gpt-4o-mini");
        std::cout << model.generate_text("\"Explain Rust ownership in one sentence.\"")
                  << "\n";

        std::cout << "\n--- Streaming ---\n";
        model.stream_text("\"Write a haiku about Rust.\"",
                          [](const std::string &p) { std::cout << "PART: " << p << "\n"; });

    } catch (const AimuxException &e) {
        std::cerr << "Error: " << e.what() << "\n";
        // HTTP-shaped failures are all AIMUX_E_API_CALL; status is the classification.
        if (e.code() == AIMUX_E_API_CALL && e.status() == 429) {
            // retryMs() is -1 when the 429 carried no retry-after header —
            // fall back to your own exponential backoff.
            if (e.retryMs() >= 0) {
                std::cerr << "rate limited, retry_ms=" << e.retryMs() << "\n";
            } else {
                std::cerr << "rate limited, no retry-after hint, back off exponentially\n";
            }
        }
        return 1;
    } catch (const std::exception &e) {
        std::cerr << "Error: " << e.what() << "\n";
        return 1;
    }
    return 0;
}
