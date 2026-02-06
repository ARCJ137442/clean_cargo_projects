use anyhow::{anyhow, Context, Result};
use dialoguer::Input;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn get_parent_dir() -> Result<PathBuf> {
    // 获取当前可执行文件所在目录
    let exe_path = std::env::current_exe()
        .context("获取当前程序路径失败")?;
    let parent_dir = exe_path
        .parent()
        .ok_or_else(|| anyhow!("无法获取父目录"))?
        .to_path_buf();
    Ok(parent_dir)
}

fn ask_and_clean(cargo_dir: &Path) -> Result<String> {
    println!("\n{}", "─".repeat(50));
    println!("找到 Cargo 项目: {}", cargo_dir.display());

    loop {
        let response: String = Input::new()
            .with_prompt("是否执行 'cargo clean'? [y/n/s/q]")
            .interact_text()
            .context("获取用户输入失败")?;

        match response.trim().to_lowercase().as_str() {
            "y" => {
                println!("\n正在执行 cargo clean...");
                match execute_cargo_clean(cargo_dir) {
                    Ok(_) => {
                        println!("✓ 清理成功: {}", cargo_dir.display());
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    Err(e) => {
                        println!("✗ 清理失败: {}", e);
                        println!("  → 继续处理下一个...");
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                return Ok("cleaned".to_string());
            }
            "n" => {
                println!("  → 跳过: {}", cargo_dir.file_name().unwrap_or_default().to_string_lossy());
                return Ok("skipped".to_string());
            }
            "s" => {
                println!("\n[全部是模式] 正在执行 cargo clean...");
                match execute_cargo_clean(cargo_dir) {
                    Ok(_) => {
                        println!("✓ 清理成功: {}", cargo_dir.display());
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    Err(e) => {
                        println!("✗ 清理失败: {}", e);
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                return Ok("cleaned".to_string());
            }
            "q" => {
                println!("\n用户取消操作");
                return Ok("quit".to_string());
            }
            _ => {
                println!("无效输入，请输入 y / n / s / q");
            }
        }
    }
}

fn execute_cargo_clean(cargo_dir: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .args(&["clean"])
        .current_dir(cargo_dir)
        .status()
        .with_context(|| format!("执行 cargo clean 失败: {}", cargo_dir.display()))?;

    if !status.success() {
        return Err(anyhow!("cargo clean 返回非零状态"));
    }

    Ok(())
}

/// 计算目录的磁盘占用大小，返回可读格式的字符串（如 "20MB", "1.2GB"）
fn get_dir_size_str(path: &Path) -> String {
    fn dir_size_iter(path: &Path) -> std::io::Result<u64> {
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                total += if entry.path().is_dir() {
                    dir_size_iter(&entry.path())?
                } else {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                };
            }
        }
        Ok(total)
    }

    match dir_size_iter(path) {
        Ok(bytes) => {
            const KB: u64 = 1024;
            const MB: u64 = KB * 1024;
            const GB: u64 = MB * 1024;

            if bytes >= GB {
                format!("{:.1}GB", bytes as f64 / GB as f64)
            } else if bytes >= MB {
                format!("{:.1}MB", bytes as f64 / MB as f64)
            } else if bytes >= KB {
                format!("{:.1}KB", bytes as f64 / KB as f64)
            } else {
                format!("{}B", bytes)
            }
        }
        Err(_) => String::from("?"), // 无法计算时显示问号
    }
}

fn traverse_and_clean(parent_dir: &Path) -> Result<(usize, usize)> {
    let mut cleaned = 0;
    let mut skipped = 0;

    // 使用 VecDeque 作为队列实现BFS遍历
    let mut dir_queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    dir_queue.push_back((parent_dir.to_path_buf(), 0));

    while let Some((current_dir, depth)) = dir_queue.pop_front() {
        let indent = "  ".repeat(depth);

        // 打印当前正在遍历的目录
        if let Some(dir_name) = current_dir.file_name() {
            println!("{}⏳ [遍历] {}/", indent, dir_name.to_string_lossy());
        }

        // 检查是否有 Cargo.toml 且 target 目录存在
        let cargo_toml = current_dir.join("Cargo.toml");
        let target_dir = current_dir.join("target");
        if cargo_toml.exists() && target_dir.exists() {
            let target_size = get_dir_size_str(&target_dir);
            println!("{}  └── ✓ 找到 Cargo.toml + target/ ({})", indent, target_size);

            match ask_and_clean(&current_dir) {
                Ok(action) => {
                    if action == "cleaned" {
                        cleaned += 1;
                    } else if action == "skipped" {
                        skipped += 1;
                    } else if action == "quit" {
                        println!("\n[中止] 用户选择退出");
                        return Ok((cleaned, skipped));
                    }
                }
                Err(e) => {
                    println!("{}  └── ✗ 操作出错: {}", indent, e);
                    skipped += 1;
                }
            }
        }

        // 收集子目录
        match std::fs::read_dir(&current_dir) {
            Ok(entries) => {
                let sub_dirs: Vec<(PathBuf, usize)> = entries
                    .filter_map(|entry| entry.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| (e.path(), depth + 1))
                    .collect();

                // BFS：直接将子目录添加到队列末尾
                dir_queue.extend(sub_dirs);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                println!("{}⏭️ [跳过] 无权限访问: {}", indent, current_dir.display());
                skipped += 1;
            }
            Err(e) => {
                println!("{}⚠️ [警告] {}", indent, e);
            }
        }
    }

    Ok((cleaned, skipped))
}

fn main() -> Result<()> {
    let parent_dir = get_parent_dir()?;

    println!("遍历目录: {}", parent_dir.display());
    println!("{}", "=".repeat(60));
    println!("提示: y=执行 clean, n=跳过, s=全部执行, q=全部退出");
    println!("{}", "=".repeat(60));

    match traverse_and_clean(&parent_dir) {
        Ok((cleaned, skipped)) => {
            println!("\n{}", "=".repeat(60));
            println!("遍历完成!");
            println!("  ✓ 清理完成: {} 个项目", cleaned);
            println!("  ○ 跳过: {} 个项目", skipped);
            println!("{}", "=".repeat(60));
        }
        Err(e) => {
            eprintln!("\n错误: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
