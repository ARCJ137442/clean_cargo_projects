use anyhow::{anyhow, Context, Result};
use dialoguer::Input;
use rayon::prelude::*;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;

lazy_static::lazy_static! {
    static ref PRINT_LOCK: Mutex<()> = Mutex::new(());
}

/// Cargo 项目信息
#[derive(Clone)]
struct CargoProject {
    path: PathBuf,
    target_size: String,
}

/// 扫描进度消息
enum ScanProgress {
    Visiting(PathBuf, usize), // 路径, 深度
    Found(CargoProject),
    Scanned(usize),           // 已扫描计数
    Done,
}

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

/// 阶段一：BFS 遍历收集所有 Cargo 项目
fn collect_cargo_projects(root: &Path, num_threads: usize) -> Vec<CargoProject> {
    // 创建同步通道，设置合适的缓冲区大小
    // sync_channel(0) = 无缓冲，发送者必须等待接收者
    // sync_channel(100) = 100 消息缓冲区
    let (progress_tx, progress_rx) = mpsc::sync_channel::<ScanProgress>(1000);

    // 启动进度显示线程
    let progress_handle = thread::spawn(move || {
        let mut total_scanned = 0;

        while let Ok(msg) = progress_rx.recv() {
            match msg {
                ScanProgress::Visiting(path, depth) => {
                    let _guard = PRINT_LOCK.lock().unwrap();
                    let indent = "  ".repeat(depth.saturating_sub(1));
                    let dir_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_else(|| path.to_string_lossy());
                    println!("{}⏳ [遍历] {}/", indent, dir_name);
                }
                ScanProgress::Found(project) => {
                    let _guard = PRINT_LOCK.lock().unwrap();
                    println!(
                        "      ✓ 找到 Cargo.toml + target/ ({})",
                        project.target_size
                    );
                }
                ScanProgress::Scanned(count) => {
                    total_scanned = count;
                    // 每扫描 100 个目录才更新计数，避免刷屏
                    if count % 100 == 0 {
                        let _guard = PRINT_LOCK.lock().unwrap();
                        print!("\r[进度] 已扫描 {} 个目录...", count);
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                    }
                }
                ScanProgress::Done => {
                    let _guard = PRINT_LOCK.lock().unwrap();
                    print!("\r"); // 清除进度行
                    println!("\n[扫描完成] 共扫描 {} 个目录\n", total_scanned);
                    break;
                }
            }
        }
    });

    // 使用 BFS 收集所有目录
    let mut all_dirs: Vec<PathBuf> = Vec::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(root.to_path_buf());

    while let Some(dir) = queue.pop_front() {
        all_dirs.push(dir.clone());

        // 读取目录内容并按字母顺序添加到队列
        let mut children: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // 跳过 target 目录（不进入其内部）
                    if path.file_name().map(|n| n == "target").unwrap_or(false) {
                        continue;
                    }
                    children.push(path);
                }
            }
            // 按字母顺序排序，保证 BFS 顺序的一致性
            children.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            for child in children {
                queue.push_back(child);
            }
        }
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .unwrap();

    let projects: Mutex<Vec<CargoProject>> = Mutex::new(Vec::new());
    let total_scanned: Mutex<usize> = Mutex::new(0);

    pool.install(|| {
        all_dirs.par_iter().enumerate().for_each(|(index, dir)| {
            let depth = calculate_depth(root, dir);
            let path = dir.clone();

            // 发送遍历进度
            let _ = progress_tx.send(ScanProgress::Visiting(path.clone(), depth));

            // 检查 Cargo 项目特征
            let cargo_toml = path.join("Cargo.toml");
            let target_dir = path.join("target");

            if cargo_toml.exists() && target_dir.exists() {
                // 计算 target 大小
                let target_size = get_dir_size_str(&target_dir);

                let project = CargoProject {
                    path: path.clone(),
                    target_size: target_size.clone(),
                };

                let _guard = PRINT_LOCK.lock().unwrap();
                println!(
                    "      ✓ 找到 Cargo.toml + target/ ({})",
                    target_size
                );
                drop(_guard);

                let mut projects = projects.lock().unwrap();
                projects.push(project.clone());

                // 发送找到的项目
                let _ = progress_tx.send(ScanProgress::Found(project));
            }

            // 更新扫描计数
            let mut total = total_scanned.lock().unwrap();
            *total += 1;

            // 每 10 个目录发送一次计数更新
            if *total % 10 == 0 {
                let _ = progress_tx.send(ScanProgress::Scanned(*total));
            }

            // 发送最终计数
            if index == all_dirs.len() - 1 {
                let _ = progress_tx.send(ScanProgress::Scanned(*total));
            }
        });

        // 发送完成信号
        let _ = progress_tx.send(ScanProgress::Done);
    });

    // 等待进度线程完成
    progress_handle.join().unwrap();

    let mut projects = projects.into_inner().unwrap();
    // 按深度和路径排序，使输出更整齐
    projects.sort_by(|a, b| {
        let depth_a = calculate_depth(root, &a.path);
        let depth_b = calculate_depth(root, &b.path);
        depth_a.cmp(&depth_b).then(a.path.cmp(&b.path))
    });

    projects
}

/// 计算目录相对于根目录的深度
fn calculate_depth(root: &Path, dir: &Path) -> usize {
    let mut depth: usize = 0;
    let mut current = dir;
    while let Some(parent) = current.parent() {
        if parent == root || parent.starts_with(root) {
            depth += 1;
        } else {
            break;
        }
        current = parent;
    }
    depth.saturating_sub(1) // 根目录深度为 0
}

/// 计算目录的磁盘占用大小，返回可读格式的字符串
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
        Err(_) => String::from("?"),
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

/// 阶段二：交互式询问用户选择
fn collect_user_selections(projects: &[CargoProject]) -> Result<Vec<PathBuf>> {
    let mut to_clean: Vec<PathBuf> = Vec::new();
    let mut cleaned = 0;
    let mut skipped = 0;

    for (i, project) in projects.iter().enumerate() {
        println!("\n{}", "─".repeat(50));
        println!(
            "{}. {} (target: {})",
            i + 1,
            project.path.display(),
            project.target_size
        );

        loop {
            let response: String = Input::new()
                .with_prompt("是否执行 'cargo clean'? [y/n/s/q]")
                .interact_text()
                .context("获取用户输入失败")?;

            match response.trim().to_lowercase().as_str() {
                "y" | "s" => {
                    // y 和 s 都执行 clean
                    to_clean.push(project.path.clone());
                    cleaned += 1;
                    if response.trim().to_lowercase() == "s" {
                        // 继续扫描剩余项目，但全部执行 clean
                        println!("[全部是模式] 已选择执行 clean，继续扫描...");
                    }
                    break;
                }
                "n" => {
                    println!("  → 跳过");
                    skipped += 1;
                    break;
                }
                "q" => {
                    println!("[中止] 用户选择退出");
                    return Ok(to_clean);
                }
                _ => {
                    println!("无效输入，请输入 y / n / s / q");
                }
            }
        }
    }

    println!("\n{}", "─".repeat(50));
    println!("选择完成: {} 个执行 clean, {} 个跳过", cleaned, skipped);

    Ok(to_clean)
}

/// 阶段三：并行执行 cargo clean（限制线程数避免 I/O 争用）
fn execute_cargo_clean_parallel(projects: &[PathBuf], num_threads: usize) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .unwrap();

    let results: Mutex<Vec<(PathBuf, Result<()>)>> = Mutex::new(Vec::new());

    pool.install(|| {
        projects.par_iter().for_each(|project| {
            let result = execute_cargo_clean(project);
            let mut results = results.lock().unwrap();
            results.push((project.clone(), result));
        });
    });

    // 主线程统一打印结果
    let mut success_count = 0;
    let mut fail_count = 0;

    println!("\n{}", "=".repeat(60));
    println!("清理结果:");
    println!("{}", "=".repeat(60));

    for (project, result) in results.into_inner().unwrap() {
        match result {
            Ok(_) => {
                println!("[OK]   {}", project.display());
                success_count += 1;
            }
            Err(e) => {
                println!("[FAIL] {} - {}", project.display(), e);
                fail_count += 1;
            }
        }
    }

    println!("\n清理完成: {} 个成功, {} 个失败", success_count, fail_count);
}

fn main() -> Result<()> {
    let parent_dir = get_parent_dir()?;

    println!("遍历目录: {}", parent_dir.display());
    println!("{}", "=".repeat(60));
    println!("提示: y=执行 clean, n=跳过, s=全部执行, q=全部退出");
    println!("{}", "=".repeat(60));

    // 计算可用线程数（扫描用全部，清理用 2-4 个）
    let total_threads = std::cmp::min(8, num_cpus::get());
    let clean_threads = std::cmp::max(2, std::cmp::min(4, total_threads));

    println!("\n[阶段一] 开始并行扫描 Cargo 项目 (使用 {} 个线程)", total_threads);

    // 阶段一：并行扫描收集项目
    let projects = collect_cargo_projects(&parent_dir, total_threads);

    if projects.is_empty() {
        println!("未找到任何 Cargo 项目");
        return Ok(());
    }

    println!("[扫描结果] 共找到 {} 个 Cargo 项目\n", projects.len());

    // 显示项目列表
    println!("项目列表:");
    println!("{}", "-".repeat(60));
    for (i, project) in projects.iter().enumerate() {
        println!(
            "{:3}. {} (target: {})",
            i + 1,
            project.path.display(),
            project.target_size
        );
    }
    println!("{}", "-".repeat(60));

    // 阶段二：交互式询问用户
    println!("\n[阶段二] 开始交互选择...");

    let to_clean = collect_user_selections(&projects)?;

    if to_clean.is_empty() {
        println!("\n没有选择任何项目进行清理");
        return Ok(());
    }

    // 阶段三：并行执行 clean
    println!(
        "\n[阶段三] 开始并行执行 cargo clean (使用 {} 个线程)",
        clean_threads
    );

    execute_cargo_clean_parallel(&to_clean, clean_threads);

    println!("\n{}", "=".repeat(60));
    println!("所有任务完成!");
    println!("{}", "=".repeat(60));

    Ok(())
}
