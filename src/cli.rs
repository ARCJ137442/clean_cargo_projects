use clap::Parser;
use std::path::PathBuf;

/// clean_cargo_projects 命令行参数
#[derive(Parser, Debug)]
#[command(name = "clean_cargo_projects")]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// 扫描的起始路径（可选，默认：exe 所在目录）
    #[arg(short, long)]
    pub path: Option<PathBuf>,

    /// 搜索方式: bfs | dfs
    #[arg(short, long, default_value = "bfs")]
    pub strategy: String,

    /// 自动清理大于此大小的 target（如 100MB, 1GB）
    #[arg(short, long)]
    pub threshold: Option<String>,

    /// 询问方式: real-time | after-scan | auto | none
    #[arg(short, long, default_value = "real-time")]
    pub ask_mode: String,

    /// 并行扫描线程数
    #[arg(long, default_value_t = num_cpus::get())]
    pub parallel_scan: usize,

    /// 并行清理线程数
    #[arg(long, default_value = "4")]
    pub parallel_clean: usize,

    /// 排除匹配的目录（可多次使用）
    #[arg(short, long)]
    pub exclude: Vec<String>,

    /// 预览模式：不实际执行清理
    #[arg(short, long)]
    pub dry_run: bool,

    /// JSON 格式输出
    #[arg(short, long)]
    pub json: bool,

    /// 最大扫描深度
    #[arg(long)]
    pub max_depth: Option<u32>,
}
