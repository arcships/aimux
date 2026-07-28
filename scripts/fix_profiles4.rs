// 修复残留的 with_profile 位置错误 - 更通用的匹配
use std::fs;
use std::path::PathBuf;
use regex::Regex;

fn main() {
    let dir = PathBuf::from("aimux-providers/src");
    let mut fixed = 0;

    // 匹配: .with_provider(PROVIDER_NAME),\n<ws>.with_profile(OpenAICompatProfile::full()),\n<ws>)
    let re = Regex::new(
        r"\.with_provider\(PROVIDER_NAME\),\n\s+\.with_profile\(OpenAICompatProfile::full\(\)\),\n\s+\)"
    ).unwrap();

    // 也匹配: .with_base_url(DEFAULT_BASE_URL),\n<ws>.with_provider(PROVIDER_NAME),\n<ws>.with_profile(...)
    let re2 = Regex::new(
        r"\.with_base_url\(DEFAULT_BASE_URL\),\n\s+\.with_provider\(PROVIDER_NAME\),\n\s+\.with_profile\(OpenAICompatProfile::full\(\)\),\n\s+\)"
    ).unwrap();

    for entry in fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        if !name.ends_with(".rs") || name == "lib.rs" {
            continue;
        }

        let content = fs::read_to_string(&path).unwrap();
        let mut new_content = content.clone();

        // 先修 re2（更长的模式）
        new_content = re2.replace_all(&new_content,
            ".with_base_url(DEFAULT_BASE_URL)\n                .with_provider(PROVIDER_NAME)\n                .with_profile(OpenAICompatProfile::full()),\n        )"
        ).to_string();

        // 再修 re
        new_content = re.replace_all(&new_content,
            ".with_provider(PROVIDER_NAME)\n                .with_profile(OpenAICompatProfile::full()),\n        )"
        ).to_string();

        if new_content != content {
            fs::write(&path, new_content).unwrap();
            fixed += 1;
            println!("Fixed: {}", name);
        }
    }

    println!("\n总计: {} fixed", fixed);
}
