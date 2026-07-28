// 批量给薄封装加 OpenAICompatProfile::full()
// 用法: cargo run --release -p fix-profiles
// 或者直接用 rustc 跑
use std::fs;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from("aimux-providers/src");
    let mut fixed = 0;
    let mut skipped = 0;

    for entry in fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();

        // 跳过非 .rs 文件和已处理的文件
        if !name.ends_with(".rs") || name == "lib.rs" || name == "open_responses.rs" {
            continue;
        }
        if name == "groq.rs" || name == "alibaba.rs" {
            continue;
        }

        let content = fs::read_to_string(&path).unwrap();

        // 只处理薄封装（包含 OpenAIProvider::new 但不含 OpenAICompatProfile）
        if !content.contains("OpenAIProvider::new") || content.contains("OpenAICompatProfile") {
            skipped += 1;
            continue;
        }

        let mut new_content = content.clone();

        // 1. 加 import：把 use crate::openai::{OpenAIConfig, ...} 改成包含 OpenAICompatProfile
        // 情况 A: use crate::openai::{OpenAIConfig, OpenAIModel, OpenAIProvider};
        if new_content.contains("use crate::openai::{OpenAIConfig") {
            new_content = new_content.replace(
                "use crate::openai::{OpenAIConfig",
                "use crate::openai::{OpenAICompatProfile, OpenAIConfig",
            );
        }
        // 情况 B: use crate::openai::OpenAIConfig; （单独一行）
        else if new_content.contains("use crate::openai::OpenAIConfig;") {
            new_content = new_content.replace(
                "use crate::openai::OpenAIConfig;",
                "use crate::openai::{OpenAICompatProfile, OpenAIConfig};",
            );
        }

        // 2. 在 OpenAIConfig::new(api_key) 链的末尾加 .with_profile(OpenAICompatProfile::full())
        // 模式 A: .with_provider(PROVIDER_NAME)\n        )  → 加 .with_profile 在 ) 前
        // 模式 B: .with_base_url(DEFAULT_BASE_URL)\n        )  → 加 .with_provider + .with_profile

        // 先试模式 A：已有 .with_provider(PROVIDER_NAME)
        if new_content.contains(".with_provider(PROVIDER_NAME)") {
            // 在 .with_provider(PROVIDER_NAME) 后面加 .with_profile
            // 需要找缩进
            if let Some(pos) = new_content.find(".with_provider(PROVIDER_NAME)") {
                let line_start = &new_content[pos..];
                // 找这行结尾的换行
                if let Some(nl) = line_start.find('\n') {
                    let after = &line_start[nl+1..];
                    // 找下一个非空白行
                    let indent: String = after.chars().take_while(|c| c.is_whitespace() && *c != '\n').collect();
                    let insert = format!("\n{}.with_profile(OpenAICompatProfile::full())", indent);
                    // 只在后面是 ) 时插入
                    let rest = after.trim_start();
                    if rest.starts_with(')') {
                        let split = pos + line_start[..nl].len();
                        new_content = format!("{}{}{}", &new_content[..split], insert, &new_content[split..]);
                    }
                }
            }
        }
        // 模式 B：只有 .with_base_url(DEFAULT_BASE_URL)，没有 .with_provider
        else if new_content.contains(".with_base_url(DEFAULT_BASE_URL)") {
            // 在 .with_base_url(DEFAULT_BASE_URL) 后加 .with_provider(PROVIDER_NAME) 和 .with_profile
            if let Some(pos) = new_content.find(".with_base_url(DEFAULT_BASE_URL)") {
                let line_start = &new_content[pos..];
                if let Some(nl) = line_start.find('\n') {
                    let after = &line_start[nl+1..];
                    let indent: String = after.chars().take_while(|c| c.is_whitespace() && *c != '\n').collect();
                    let rest = after.trim_start();
                    if rest.starts_with(')') {
                        let insert = format!("\n{}.with_provider(PROVIDER_NAME)\n{}.with_profile(OpenAICompatProfile::full())", indent, indent);
                        let split = pos + line_start[..nl].len();
                        new_content = format!("{}{}{}", &new_content[..split], insert, &new_content[split..]);
                    }
                }
            }
        }

        if new_content != content {
            fs::write(&path, new_content).unwrap();
            fixed += 1;
            println!("Fixed: {}", name);
        } else {
            println!("SKIP (no pattern matched): {}", name);
            skipped += 1;
        }
    }

    println!("\n总计: {} fixed, {} skipped", fixed, skipped);
}
