// example.cpp — C++ RAII wrapper example using aimux-ffi
//
// Build: g++ -std=c++17 -o example_cpp example.cpp -L../../target/debug -laimux_ffi -lpthread -ldl -lm
// Run:   OPENAI_API_KEY=sk-... ./example_cpp

#include <iostream>
#include <memory>
#include <string>
#include <functional>

#include "aimux-ffi.h"

// RAII wrapper for model handle
class AimuxModel {
public:
    static AimuxModel openai(const std::string &api_key, const std::string &model_id) {
        auto handle = aimux_openai_new(api_key.c_str(), model_id.c_str());
        if (handle == 0) {
            throw std::runtime_error("Failed to create OpenAI model");
        }
        return AimuxModel(handle);
    }

    static AimuxModel anthropic(const std::string &api_key, const std::string &model_id) {
        auto handle = aimux_anthropic_new(api_key.c_str(), model_id.c_str());
        if (handle == 0) {
            throw std::runtime_error("Failed to create Anthropic model");
        }
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

        // Wrap lambdas into C function pointers via static trampolines.
        // (In production, use a thread-local or context-pointer pattern.)
        auto part_cb = [](const char *json) {
            printf("PART: %s\n", json);
        };
        auto done_cb = []() {
            printf("[done]\n");
        };
        auto err_cb = [](const char *json) {
            fprintf(stderr, "ERROR: %s\n", json);
        };

        aimux_stream_text(handle_, prompt_json.c_str(), opts,
                          part_cb, done_cb, err_cb);
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
