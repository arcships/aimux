// 修复残留的 with_profile 位置错误 - 纯字符串匹配
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
        if !content.contains("with_profile") {
            continue;
        }

        let mut new_content = content.clone();

        // 通用修复：找到 .with_provider(PROVIDER_NAME), 后面跟着换行+空格+.with_profile 的模式
        // 把它改成 .with_provider(PROVIDER_NAME)\n                .with_profile(...),\n        )
        // 通过逐行处理
        let lines: Vec<&str> = new_content.lines().collect();
        let mut result = String::new();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            // 检查当前行是否以 .with_provider(PROVIDER_NAME), 结尾
            if line.trim_end().ends_with(".with_provider(PROVIDER_NAME),") {
                // 去掉末尾逗号
                let trimmed = line.trim_end();
                let without_comma = &trimmed[..trimmed.len() - 1];
                result.push_str(without_comma);
                result.push('\n');
                i += 1;
                // 检查下一行是否是 .with_profile(OpenAICompatProfile::full()),
                if i < lines.len() {
                    let next = lines[i].trim();
                    if next.starts_with(".with_profile(OpenAICompatProfile::full())") {
                        // 加正确的缩进（16空格）
                        result.push_str("                .with_profile(OpenAICompatProfile::full()),\n");
                        i += 1;
                        // 跳过下一个 ) 行，加正确的 )
                        if i < lines.len() && lines[i].trim() == ")" {
                            result.push_str("        )\n");
                            i += 1;
                        }
                        continue;
                    }
                }
                // 如果不是预期模式，恢复逗号
                result = result.trim_end_matches(without_comma).to_string();
                result.push_str(line);
                result.push('\n');
            } else {
                result.push_str(line);
                result.push('\n');
                i += 1;
            }
        }

        // 同样修复 .with_base_url(DEFAULT_BASE_URL), 后跟 .with_provider 的模式
        // 但这更复杂，先只处理上面的

        if new_content.trim_end_matches('\n') != result.trim_end_matches('\n') && result.contains(".with_profile(OpenAICompatProfile::full()),\n        )") {
            new_content = result;
        }

        if new_content != content {
            fs::write(&path, &new_content).unwrap();
            fixed += 1;
            println!("Fixed: {}", name);
        }
    }

    println!("\n总计: {} fixed", fixed);
}
