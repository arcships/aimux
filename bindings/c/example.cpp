/* Offline C++17 example: owned values are released on all exception paths. */
#include "aimux_raii.hpp"
#include <iostream>

int main() {
    try {
        char *raw = nullptr;
        aimux::check(aimux_manifest_json(&raw));
        aimux::String manifest(raw);
        std::cout << manifest.get() << '\n';
        raw = nullptr;
        aimux::check(aimux_call(0, "runtime.describe", "{}", 0, &raw));
        aimux::String description(raw);
        std::cout << description.get() << '\n';
        return 0;
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
