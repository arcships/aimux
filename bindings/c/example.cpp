// example.cpp — C++ RAII wrapper example using aimux-ffi
//
// Build: g++ -std=c++17 -o example_cpp example.cpp -L../../target/debug -laimux_ffi -lpthread -ldl -lm
// Run:   OPENAI_API_KEY=sk-... ./example_cpp

#include <cstdlib>
#include <cstring>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <functional>

#include "aimux-ffi.h"

namespace {
thread_local const std::function<void(const std::string &)> *current_on_part = nullptr;
thread_local const std::function<void()> *current_on_done = nullptr;
thread_local const std::function<void(const std::string &)> *current_on_error = nullptr;
}

// Parse a constructor's JSON result (`{"handle":<u64>}` on success,
// `{"error":"..."}` on failure). Frees the string via aimux_free_string and
// throws std::runtime_error carrying the detailed engine error on failure
// (a crude substring scan, not a JSON parse -- avoids a JSON dependency for
// this demo; production bindings use a real JSON library).
static uint64_t extract_handle(char *json, const std::string &what) {
    if (!json) {
        throw std::runtime_error(what + ": constructor returned null");
    }
    std::string result(json);
    aimux_free_string(json);
    if (result.find("\"error\":") != std::string::npos) {
        throw std::runtime_error(what + ": " + result);
    }
    auto h_pos = result.find("\"handle\":");
    if (h_pos == std::string::npos) {
        throw std::runtime_error(what + ": invalid constructor response: " + result);
    }
    return std::strtoull(result.c_str() + h_pos + std::strlen("\"handle\":"), nullptr, 10);
}

// RAII wrapper for model handle
class AimuxModel {
public:
    static AimuxModel openai(const std::string &api_key, const std::string &model_id) {
        auto handle = extract_handle(aimux_openai_new(api_key.c_str(), model_id.c_str()), "openai");
        return AimuxModel(handle);
    }

    static AimuxModel anthropic(const std::string &api_key, const std::string &model_id) {
        auto handle = extract_handle(aimux_anthropic_new(api_key.c_str(), model_id.c_str()), "anthropic");
        return AimuxModel(handle);
    }

    // Registry-backed provider (RFC-0017 phase 4). api_key == nullptr reads the
    // provider's env var from the registry entry; config_json == nullptr uses
    // default ProviderOptions.
    static AimuxModel provider(const std::string &name, const std::string &model_id,
                               const char *api_key = nullptr,
                               const char *config_json = nullptr) {
        auto handle = extract_handle(
            aimux_provider_new(name.c_str(), api_key, model_id.c_str(), config_json), "provider '" + name + "'");
        return AimuxModel(handle);
    }

    ~AimuxModel() {
        if (handle_) {
            aimux_drop_handle(handle_);
        }
    }

    // Non-copyable, movable
    AimuxModel(const AimuxModel &) = delete;
    AimuxModel &operator=(const AimuxModel &) = delete;
    AimuxModel(AimuxModel &&other) noexcept : handle_(other.handle_) {
        other.handle_ = 0;
    }

    // Generate text (non-streaming)
    std::string generate_text(const std::string &prompt_json,
                              const std::string &opts_json = "") {
        const char *opts = opts_json.empty() ? nullptr : opts_json.c_str();
        char *result = aimux_generate_text(handle_, prompt_json.c_str(), opts);
        if (!result) {
            throw std::runtime_error("generate_text returned null");
        }
        std::string s(result);
        aimux_free_string(result);
        return s;
    }

    // Stream text with callbacks
    void stream_text(const std::string &prompt_json,
                     std::function<void(const std::string &)> on_part,
                     std::function<void()> on_done = nullptr,
                     std::function<void(const std::string &)> on_error = nullptr,
                     const std::string &opts_json = "") {
        const char *opts = opts_json.empty() ? nullptr : opts_json.c_str();

        // The C ABI has no user-data pointer. Since the call is synchronous and
        // callbacks run on the invoking thread, thread-local trampolines safely
        // isolate concurrent streams while forwarding the caller's callbacks.
        current_on_part = &on_part;
        current_on_done = &on_done;
        current_on_error = &on_error;

        aimux_stream_text(
            handle_, prompt_json.c_str(), opts,
            [](const char *json) {
                if (current_on_part && *current_on_part) (*current_on_part)(json);
            },
            []() {
                if (current_on_done && *current_on_done) (*current_on_done)();
            },
            [](const char *json) {
                if (current_on_error && *current_on_error) (*current_on_error)(json);
            });

        current_on_part = nullptr;
        current_on_done = nullptr;
        current_on_error = nullptr;
    }

private:
    explicit AimuxModel(uint64_t handle) : handle_(handle) {}
    uint64_t handle_ = 0;
};

int main() {
    const char *api_key = std::getenv("OPENAI_API_KEY");
    if (!api_key) {
        std::cerr << "Please set OPENAI_API_KEY\n";
        return 1;
    }

    try {
        auto model = AimuxModel::openai(api_key, "gpt-4o-mini");

        // Generate
        auto result = model.generate_text("\"Hello, world!\"");
        std::cout << "Result: " << result << "\n";

        // Stream
        std::cout << "\n--- Streaming ---\n";
        model.stream_text("\"Write a haiku about Rust.\"",
                          [](const std::string &part) {
                              std::cout << "PART: " << part << "\n";
                          });

    } catch (const std::exception &e) {
        std::cerr << "Error: " << e.what() << "\n";
        return 1;
    }

    return 0;
}
