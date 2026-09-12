// example.cpp — C++ RAII wrapper example using aimux-ffi
//
// Build: g++ -std=c++17 -o example_cpp example.cpp -I../../aimux-ffi -L../../target/release -laimux_ffi -lpthread -ldl -lm
// Run:   OPENAI_API_KEY=sk-... ./example_cpp

#include <cstdlib>
#include <cstring>
#include <exception>
#include <functional>
#include <iostream>
#include <stdexcept>
#include <string>

#include "aimux-error.h"
#include "aimux-ffi.h"

// Own a getter's string (or "" for NULL) and release it.
static std::string take(char *s) {
    std::string out = s ? s : "";
    aimux_free_string(s);
    return out;
}

// Thin exception owning a copy of an AiMuxError's facts.
class AimuxException : public std::runtime_error {
public:
    // Consumes the returned error: copies every getter, then frees it.
    AimuxException(const std::string &context, aimux_error_t *e)
        : std::runtime_error(context + ": " + take(aimux_error_message(e))),
          code_(static_cast<aimux_error_code_t>(aimux_error_code(e))),
          status_(aimux_error_status(e)), retry_ms_(aimux_error_retry_ms(e)),
          retryable_(aimux_error_retryable(e) != 0),
          provider_code_(take(aimux_error_provider_code(e))),
          provider_message_(take(aimux_error_provider_message(e))),
          response_body_(take(aimux_error_response_body(e))),
          model_id_(take(aimux_error_model_id(e))), model_type_(take(aimux_error_model_type(e))),
          provider_id_(take(aimux_error_provider_id(e))) {
        aimux_error_free(e);
    }

    aimux_error_code_t code() const { return code_; }
    int status() const { return status_; }
    int64_t retryMs() const { return retry_ms_; }
    /** The callee's verdict: true when retrying may help. Not derivable from
     *  status() — a transport failure and a missing API key both report -1
     *  and disagree here. */
    bool retryable() const { return retryable_; }
    // Per-code payload; "" when the code does not carry the field.
    const std::string &providerCode() const { return provider_code_; }   // API_CALL
    const std::string &providerMessage() const { return provider_message_; } // API_CALL
    const std::string &responseBody() const { return response_body_; }   // API_CALL
    const std::string &modelId() const { return model_id_; }             // NO_SUCH_MODEL
    const std::string &modelType() const { return model_type_; }         // NO_SUCH_MODEL
    const std::string &providerId() const { return provider_id_; }       // NO_SUCH_PROVIDER

private:
    aimux_error_code_t code_ = AIMUX_E_OTHER;
    int status_ = -1;
    int64_t retry_ms_ = -1;
    bool retryable_ = false;
    std::string provider_code_, provider_message_, response_body_, model_id_,
        model_type_, provider_id_;
};

// An AiMuxError code (1..14) becomes AimuxException. Codes 200..206 mean this
// program made a bad call (NULL argument, malformed JSON, dead handle).
static void throw_if_failed(aimux_error_t *err, const std::string &what) {
    if (!err) return;
    int32_t code = aimux_error_code(err);
    if (code >= AIMUX_E_OTHER && code <= AIMUX_E_RETRY) {
        throw AimuxException(what, err);
    }
    std::string msg = take(aimux_error_message(err));
    aimux_error_free(err);
    throw std::logic_error(what + ": bad call (fix the caller): " + msg);
}

// RAII wrapper for model handle
class AimuxModel {
public:
    static AimuxModel openai(const std::string &api_key, const std::string &model_id) {
        uint64_t h = 0;
        throw_if_failed(aimux_openai_new(api_key.c_str(), model_id.c_str(), &h), "openai");
        return AimuxModel(h);
    }

    static AimuxModel anthropic(const std::string &api_key, const std::string &model_id) {
        uint64_t h = 0;
        throw_if_failed(aimux_anthropic_new(api_key.c_str(), model_id.c_str(), &h), "anthropic");
        return AimuxModel(h);
    }

    static AimuxModel provider(const std::string &name, const std::string &model_id,
                               const char *api_key = nullptr,
                               const char *config_json = nullptr) {
        uint64_t h = 0;
        throw_if_failed(aimux_provider_new(name.c_str(), api_key, model_id.c_str(), config_json, &h),
                        "provider '" + name + "'");
        return AimuxModel(h);
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
        char *result = nullptr;
        throw_if_failed(aimux_generate_text(handle_, prompt_json.c_str(), opts, &result),
                        "generate_text");
        return take(result);
    }

    std::string generate_text_as_openai(const std::string &prompt_json,
                                        const std::string &opts_json = "") {
        require_open("generate_text_as_openai");
        const char *opts = opts_json.empty() ? nullptr : opts_json.c_str();
        char *result = nullptr;
        throw_if_failed(aimux_generate_text_as_openai(handle_, prompt_json.c_str(), opts, &result),
                        "generate_text_as_openai");
        return take(result);
    }

    // Callbacks must not unwind across the Aimux C ABI (Rust cannot reliably
    // catch a C++ exception; the process may abort). The trampoline therefore
    // catches everything the user's on_part throws, parks it, and rethrows
    // after aimux_stream_text has returned. Once an exception is parked the
    // remaining parts are dropped on the floor — the stream still runs to its
    // end (there is no cancel-from-callback in this ABI); use an abort signal
    // if you need to stop early.
    struct StreamCtx {
        std::function<void(const std::string &)> on_part;
        std::exception_ptr pending;
    };

    void stream_text(const std::string &prompt_json,
                     std::function<void(const std::string &)> on_part,
                     const std::string &opts_json = "") {
        require_open("stream_text");
        if (!on_part) {
            throw std::logic_error("stream_text: on_part must not be empty");
        }
        const char *opts = opts_json.empty() ? nullptr : opts_json.c_str();
        StreamCtx ctx{std::move(on_part), nullptr};
        auto part_cb = [](const char *json, void *stream_ctx) noexcept {
            auto *c = static_cast<StreamCtx *>(stream_ctx);
            if (c->pending) return; // already failed; drain silently
            try {
                c->on_part(json ? json : "");
            } catch (...) {
                c->pending = std::current_exception();
            }
        };
        auto done_cb = [](void *) noexcept {};
        throw_if_failed(aimux_stream_text(handle_, prompt_json.c_str(), opts, part_cb, done_cb, &ctx),
                        "stream_text");
        if (ctx.pending) std::rethrow_exception(ctx.pending);
    }

private:
    explicit AimuxModel(uint64_t handle) : handle_(handle) {}

    void require_open(const char *what) {
        if (handle_ == 0) {
            throw std::logic_error(std::string(what) + ": model handle is closed");
        }
    }

    uint64_t handle_ = 0;
};

int main() {
    try {
        const char *api_key = std::getenv("OPENAI_API_KEY");
        if (!api_key) {
            std::cerr << "OPENAI_API_KEY is not set\n";
            return 1;
        }

        AimuxModel model = AimuxModel::openai(api_key, "gpt-4o-mini");
        std::cout << model.generate_text("\"Explain Rust ownership in one sentence.\"")
                  << "\n";

        std::cout << "\n--- Streaming ---\n";
        model.stream_text("\"Write a haiku about Rust.\"",
                          [](const std::string &p) { std::cout << "PART: " << p << "\n"; });

    } catch (const AimuxException &e) {
        std::cerr << "Error: " << e.what() << "\n";
        if (e.code() == AIMUX_E_NO_SUCH_PROVIDER) {
            std::cerr << "unknown provider: " << e.providerId() << "\n";
        }
        if (!e.providerCode().empty()) {
            std::cerr << "provider code: " << e.providerCode() << "\n";
        }
        // Whether to retry is the callee's verdict, not a guess from status:
        // a statusless failure is a transport blip (retryable) or a missing
        // API key (not), and both report status() == -1.
        if (e.retryable()) {
            // retryMs() is -1 when no retry-after hint arrived — fall back to
            // your own exponential backoff.
            if (e.retryMs() >= 0) {
                std::cerr << "retryable, retry_ms=" << e.retryMs() << "\n";
            } else {
                std::cerr << "retryable, no retry-after hint, back off exponentially\n";
            }
        }
        return 1;
    } catch (const std::exception &e) {
        std::cerr << "Error: " << e.what() << "\n";
        return 1;
    }
    return 0;
}
