use anyhow::{anyhow, Context, Result};
use clap::Parser;
use dialoguer::Input;
use glob::Pattern;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;

mod cli;

lazy_static::lazy_static! {
    static ref PRINT_LOCK: Mutex<()> = Mutex::new(());
}

/// Cargo 项目信息
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CargoProject {
    path: PathBuf,
    target_size: String,
}

/// 扫描进度消息
enum ScanProgress {
    Visiting(PathBuf, usize), // 路径, 深度
    Found(CargoProject),
    Scanned(usize), // 已扫描计数
    Done,
}

/// 全局配置
struct Config {
    strategy: String,
    threshold_bytes: Option<u64>,
    ask_mode: String,
    parallel_scan: usize,
    parallel_clean: usize,
    excludes: Vec<String>,
    dry_run: bool,
    json: bool,
    max_depth: Option<u32>,
}

/// 解析 "100MB", "1GB" 为字节数
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_uppercase();
    let mut num_str = String::new();
    let mut unit = String::new();

    // 分割数字和单位
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            num_str.push(c);
        } else if c.is_ascii_alphabetic() {
            unit.push(c);
        }
    }

    let num: f64 = num_str.parse().ok()?;
    let bytes = match unit.as_str() {
        "B" => num as u64,
        "KB" => (num * 1024.0) as u64,
        "MB" => (num * 1024.0 * 1024.0) as u64,
        "GB" => (num * 1024.0 * 1024.0 * 1024.0) as u64,
        "TB" => (num * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64,
        _ => return None,
    };
    Some(bytes)
}

/// glob 模式匹配排除
fn is_excluded(path: &Path, patterns: &[String]) -> bool {
    for pattern in patterns {
        if let Ok(glob_pattern) = Pattern::new(pattern) {
            if glob_pattern.matches_path(path) {
                return true;
            }
        }
    }
    false
}

/// BFS 收集所有目录
fn bfs_collect_dirs(root: &Path) -> Vec<PathBuf> {
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
    all_dirs
}

/// DFS 收集所有目录
fn dfs_collect_dirs(root: &Path) -> Vec<PathBuf> {
    fn inner(dir: &Path, all_dirs: &mut Vec<PathBuf>) {
        all_dirs.push(dir.to_path_buf());

        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut children: Vec<PathBuf> = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // 跳过 target 目录
                    if path.file_name().map(|n| n == "target").unwrap_or(false) {
                        continue;
                    }
                    children.push(path);
                }
            }
            // 按字母顺序排序
            children.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            for child in children {
                inner(&child, all_dirs);
            }
        }
    }

    let mut all_dirs = Vec::new();
    inner(root, &mut all_dirs);
    all_dirs
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

/// 阶段一：遍历收集所有 Cargo 项目
fn collect_cargo_projects(root: &Path, config: &Config) -> Vec<CargoProject> {
    // 创建同步通道
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
                    // 每扫描 100 个目录才更新计数
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

    // 根据策略收集目录
    let all_dirs = match config.strategy.as_str() {
        "bfs" => bfs_collect_dirs(root),
        "dfs" => dfs_collect_dirs(root),
        _ => bfs_collect_dirs(root),
    };

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.parallel_scan)
        .build()
        .unwrap();

    let projects: Mutex<Vec<CargoProject>> = Mutex::new(Vec::new());
    let total_scanned: Mutex<usize> = Mutex::new(0);

    pool.install(|| {
        all_dirs.par_iter().enumerate().for_each(|(index, dir)| {
            let depth = calculate_depth(root, dir);
            let path = dir.clone();

            // 检查深度限制
            if let Some(max_depth) = config.max_depth {
                if depth > max_depth as usize {
                    return;
                }
            }

            // 检查排除
            if is_excluded(&path, &config.excludes) {
                return;
            }

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
    // 按深度和路径排序
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

/// 实时询问模式
fn real_time_ask(projects: &[CargoProject]) -> Result<Vec<PathBuf>> {
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
                .with_prompt("是否执行 'cargo clean'? [y/n]")
                .interact_text()
                .context("获取用户输入失败")?;

            match response.trim().to_lowercase().as_str() {
                "y" => {
                    to_clean.push(project.path.clone());
                    cleaned += 1;
                    break;
                }
                "n" => {
                    println!("  → 跳过");
                    skipped += 1;
                    break;
                }
                _ => {
                    println!("无效输入，请输入 y / n");
                }
            }
        }
    }

    println!("\n{}", "─".repeat(50));
    println!("选择完成: {} 个执行 clean, {} 个跳过", cleaned, skipped);

    Ok(to_clean)
}

/// 扫描后询问模式
fn after_scan_ask(projects: &[CargoProject]) -> Result<Vec<PathBuf>> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }

    println!("\n[阶段二] 请选择要清理的项目:");
    println!("{}", "-".repeat(60));

    // 显示项目列表
    for (i, p) in projects.iter().enumerate() {
        println!(
            "{:3}. [ ] {} (target: {})",
            i + 1,
            p.path.display(),
            p.target_size
        );
    }

    println!("{}", "-".repeat(60));
    println!("提示: 输入 'all' 全部选择，'none' 全部跳过，或范围如 1-5");

    loop {
        let response: String = Input::new()
            .with_prompt("选择项目")
            .interact_text()
            .context("获取用户输入失败")?;

        match response.trim().to_lowercase().as_str() {
            "all" => {
                return Ok(projects.iter().map(|p| p.path.clone()).collect());
            }
            "none" => {
                println!("已跳过所有项目");
                return Ok(Vec::new());
            }
            s if s.contains('-') => {
                let parts: Vec<&str> = s.split('-').collect();
                if parts.len() == 2 {
                    if let (Ok(start), Ok(end)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
                        let start = start.saturating_sub(1);
                        let end = std::cmp::min(end, projects.len());
                        let selected: Vec<PathBuf> = projects[start..end]
                            .iter()
                            .map(|p| p.path.clone())
                            .collect();
                        println!("已选择 {} 个项目", selected.len());
                        return Ok(selected);
                    }
                }
                println!("无效格式，请使用 1-5 格式");
            }
            _ => {
                println!("请输入 'all', 'none' 或范围如 1-5");
            }
        }
    }
}

/// 自动模式（根据阈值）
fn auto_ask(projects: &[CargoProject], threshold_bytes: Option<u64>) -> Vec<PathBuf> {
    if threshold_bytes.is_none() {
        println!("[警告] auto 模式需要 --threshold 参数，默认跳过所有项目");
        return Vec::new();
    }

    let threshold = threshold_bytes.unwrap();
    let mut to_clean: Vec<PathBuf> = Vec::new();
    let mut below_threshold = 0;

    for project in projects.iter() {
        let size_bytes = parse_size(&project.target_size).unwrap_or(0);
        if size_bytes >= threshold {
            to_clean.push(project.path.clone());
        } else {
            below_threshold += 1;
        }
    }

    println!("\n[自动模式] 阈值: {} bytes", threshold);
    println!("  将清理 {} 个项目（超过阈值）", to_clean.len());
    println!("  跳过 {} 个项目（低于阈值）", below_threshold);

    to_clean
}

/// 无询问模式（全部清理）
fn none_ask(projects: &[CargoProject]) -> Vec<PathBuf> {
    let count = projects.len();
    let result: Vec<PathBuf> = projects.iter().map(|p| p.path.clone()).collect();
    println!("\n[无询问模式] 将清理全部 {} 个项目", count);
    result
}

/// 询问模式处理函数
fn ask_mode_handler(projects: &[CargoProject], mode: &str, threshold: Option<u64>) -> Result<Vec<PathBuf>> {
    match mode {
        "real-time" => real_time_ask(projects),
        "after-scan" => after_scan_ask(projects),
        "auto" => Ok(auto_ask(projects, threshold)),
        "none" => Ok(none_ask(projects)),
        _ => {
            println!("[警告] 未知询问模式 '{}'，使用 real-time", mode);
            real_time_ask(projects)
        }
    }
}

/// JSON 输出
fn json_output(projects: &[CargoProject], to_clean: &[PathBuf], results: &[(PathBuf, Result<()>)] ) {
    #[derive(Serialize)]
    struct Output {
        total_projects: usize,
        to_clean_count: usize,
        projects: Vec<serde_json::Value>,
        results: Vec<serde_json::Value>,
    }

    let project_list: Vec<serde_json::Value> = projects
        .iter()
        .map(|p| serde_json::json!({
            "path": p.path.display().to_string(),
            "target_size": p.target_size,
            "selected": to_clean.iter().any(|tp| tp == &p.path)
        }))
        .collect();

    let result_list: Vec<serde_json::Value> = results
        .iter()
        .map(|(path, result)| {
            serde_json::json!({
                "path": path.display().to_string(),
                "success": result.is_ok(),
                "error": result.as_ref().err().map(|e| e.to_string())
            })
        })
        .collect();

    let output = Output {
        total_projects: projects.len(),
        to_clean_count: to_clean.len(),
        projects: project_list,
        results: result_list,
    };

    println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
}

fn main() -> Result<()> {
    // 解析命令行参数
    let args = cli::Args::parse();

    // 确定扫描路径
    let scan_path = match &args.path {
        Some(p) => p.clone(),
        None => get_parent_dir()?,
    };

    // 解析阈值
    let threshold_bytes = args.threshold.as_ref().and_then(|s| parse_size(s));

    // 构建配置
    let config = Config {
        strategy: args.strategy,
        threshold_bytes,
        ask_mode: args.ask_mode,
        parallel_scan: args.parallel_scan,
        parallel_clean: args.parallel_clean,
        excludes: args.exclude,
        dry_run: args.dry_run,
        json: args.json,
        max_depth: args.max_depth,
    };

    println!("遍历目录: {}", scan_path.display());
    println!("策略: {} | 询问模式: {}", config.strategy, config.ask_mode);
    if let Some(t) = config.threshold_bytes {
        println!("阈值: {} bytes", t);
    }
    if !config.excludes.is_empty() {
        println!("排除: {:?}", config.excludes);
    }
    if config.dry_run {
        println!("[预览模式]");
    }
    println!("{}", "=".repeat(60));

    println!(
        "\n[阶段一] 开始并行扫描 Cargo 项目 (使用 {} 个线程)",
        config.parallel_scan
    );

    // 阶段一：并行扫描收集项目
    let projects = collect_cargo_projects(&scan_path, &config);

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

    // 阶段二：询问用户选择
    println!("\n[阶段二] 开始选择...");

    let to_clean = ask_mode_handler(&projects, &config.ask_mode, config.threshold_bytes)?;

    if to_clean.is_empty() {
        println!("\n没有选择任何项目进行清理");
        return Ok(());
    }

    // 阶段三：并行执行 clean
    println!(
        "\n[阶段三] 开始执行 cargo clean (使用 {} 个线程)",
        config.parallel_clean
    );

    let results: Vec<(PathBuf, Result<()> )> = if config.dry_run || config.json {
        to_clean.iter()
            .map(|p| (p.clone(), Ok(())))
            .collect()
    } else {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(config.parallel_clean)
            .build()
            .unwrap();

        let results: Mutex<Vec<(PathBuf, Result<()> )>> = Mutex::new(Vec::new());

        pool.install(|| {
            to_clean.par_iter().for_each(|project| {
                let result = execute_cargo_clean(project);
                let mut results = results.lock().unwrap();
                results.push((project.clone(), result));
            });
        });

        results.into_inner().unwrap()
    };

    // 输出结果
    if config.json {
        json_output(&projects, &to_clean, &results);
    } else {
        println!("\n{}", "=".repeat(60));
        println!("所有任务完成!");
        println!("{}", "=".repeat(60));
    }

    Ok(())
}
