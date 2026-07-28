// 修复残留的 with_profile 位置错误
use std::fs;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from("aimux-providers/src");
    let mut fixed = 0;

    for entry in fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        if !name.ends_with(".rs") || name == "lib.rs" {
            continue;
        }

        let content = fs::read_to_string(&path).unwrap();
        let mut new_content = content.clone();

        // 修复: .with_provider(PROVIDER_NAME),\n        .with_profile(OpenAICompatProfile::full()),\n        )
        // → .with_provider(PROVIDER_NAME)\n                .with_profile(OpenAICompatProfile::full()),\n        )
        new_content = new_content.replace(
            ".with_provider(PROVIDER_NAME),\n        .with_profile(OpenAICompatProfile::full()),\n        )",
            ".with_provider(PROVIDER_NAME)\n                .with_profile(OpenAICompatProfile::full()),\n        )",
        );

        // 修复: .with_base_url(DEFAULT_BASE_URL),\n        .with_provider(PROVIDER_NAME),\n        .with_profile(...)
        // → 正确格式
        new_content = new_content.replace(
            ".with_base_url(DEFAULT_BASE_URL),\n        .with_provider(PROVIDER_NAME),\n        .with_profile(OpenAICompatProfile::full()),\n        )",
            ".with_base_url(DEFAULT_BASE_URL)\n                .with_provider(PROVIDER_NAME)\n                .with_profile(OpenAICompatProfile::full()),\n        )",
        );

        if new_content != content {
            fs::write(&path, new_content).unwrap();
            fixed += 1;
            println!("Fixed: {}", name);
        }
    }

    println!("\n总计: {} fixed", fixed);
}
