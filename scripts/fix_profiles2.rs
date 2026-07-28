// 修复 with_profile 插入位置错误
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
        if !content.contains("OpenAICompatProfile") {
            continue;
        }

        let mut new_content = content.clone();

        // 修复模式 A: .with_provider(PROVIDER_NAME),\n        .with_profile(...)\n        )
        // 改成: .with_provider(PROVIDER_NAME)\n                .with_profile(...),\n        )
        new_content = new_content.replace(
            ".with_provider(PROVIDER_NAME),\n        .with_profile(OpenAICompatProfile::full())\n        )",
            ".with_provider(PROVIDER_NAME)\n                .with_profile(OpenAICompatProfile::full()),\n        )",
        );

        // 修复模式 B: .with_base_url(DEFAULT_BASE_URL),\n        .with_provider(PROVIDER_NAME)\n        .with_profile(...)\n        )
        // 改成: .with_base_url(DEFAULT_BASE_URL)\n                .with_provider(PROVIDER_NAME)\n                .with_profile(...),\n        )
        new_content = new_content.replace(
            ".with_base_url(DEFAULT_BASE_URL),\n        .with_provider(PROVIDER_NAME)\n        .with_profile(OpenAICompatProfile::full())\n        )",
            ".with_base_url(DEFAULT_BASE_URL)\n                .with_provider(PROVIDER_NAME)\n                .with_profile(OpenAICompatProfile::full()),\n        )",
        );

        // 通用修复：任何 .with_profile(OpenAICompatProfile::full())\n        ) 
        // 前面缺逗号且缩进不对的，修成正确格式
        // 模式: ,\n        .with_profile(OpenAICompatProfile::full())\n        )
        // 这种在 ) 前面没逗号的情况
        if new_content.contains(".with_profile(OpenAICompatProfile::full())\n        )") {
            // 检查是不是已经有了正确的逗号
            if !new_content.contains(".with_profile(OpenAICompatProfile::full()),\n        )") {
                new_content = new_content.replace(
                    ".with_profile(OpenAICompatProfile::full())\n        )",
                    ".with_profile(OpenAICompatProfile::full()),\n        )",
                );
            }
        }

        // 修复模式: .with_profile(...) 出现在 ) 之后（独立行以 . 开头）
        // 即: ),\n        .with_profile(...)
        // 这种应该是: .with_profile(...),\n        )
        if new_content.contains("),\n        .with_profile(OpenAICompatProfile::full())") {
            // 这种是错误的——需要把 .with_profile 移到 ) 前面
            // 但这个比较复杂，先标记
        }

        if new_content != content {
            fs::write(&path, new_content).unwrap();
            fixed += 1;
            println!("Fixed: {}", name);
        }
    }

    println!("\n总计: {} fixed", fixed);
}
