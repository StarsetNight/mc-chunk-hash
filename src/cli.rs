//! CLI 参数定义（clap derive）。

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// mc-chunk-hash —— Minecraft 区块哈希快照与比对工具
#[derive(Parser)]
#[command(name = "mc-chunk-hash", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// 并行线程数（默认等于 CPU 线程数）
    #[arg(long, global = true)]
    pub threads: Option<usize>,
}

#[derive(Subcommand)]
pub enum Command {
    /// 扫描存档目录，生成区块哈希快照
    Snapshot(SnapshotArgs),
    /// 比对两份快照，输出差异报告
    Compare(CompareArgs),
    /// 跑一遍完整快照流程并打印耗时（性能基准）
    Bench(BenchArgs),
}

#[derive(clap::Args)]
pub struct SnapshotArgs {
    /// 存档目录（含 region 文件）
    pub input_dir: PathBuf,

    /// 输出的快照文件路径
    #[arg(long, short = 'o')]
    pub output: PathBuf,

    /// 是否遍历多层子目录（用于整个世界的遍历）
    #[arg(long, default_value_t = false)]
    pub walk: bool,

    /// 强制指定存档格式（默认自动探测）
    #[arg(long)]
    pub format: Option<ArchiveFormatArg>,

    /// 哈希内容模式（默认 raw：对原始字节算）
    #[arg(long, value_enum, default_value_t = HashModeArg::Raw)]
    pub hash_mode: HashModeArg,
}

#[derive(clap::Args)]
pub struct CompareArgs {
    /// 旧快照文件
    pub old: PathBuf,
    /// 新快照文件
    pub new: PathBuf,

    /// 报告输出文件（默认输出到 stdout）
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// 报告格式
    #[arg(long, value_enum, default_value_t = ReportFormatArg::Text)]
    pub format: ReportFormatArg,
}

#[derive(clap::Args)]
pub struct BenchArgs {
    /// 存档目录
    pub input_dir: PathBuf,

    /// 是否遍历多层子目录
    #[arg(long, default_value_t = false)]
    pub walk: bool,

    /// 强制指定存档格式（默认自动探测）
    #[arg(long)]
    pub format: Option<ArchiveFormatArg>,

    /// 哈希内容模式（默认 raw：对原始字节算）
    #[arg(long, value_enum, default_value_t = HashModeArg::Raw)]
    pub hash_mode: HashModeArg,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ArchiveFormatArg {
    Auto,
    Anvil,
    LinearV2,
    BLinearV3,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum HashModeArg {
    /// 原始字节（任何变化都检测）
    Raw,
    /// 仅方块（方块 ID + 状态）
    Blocks,
    /// 方块 + 方块实体
    BlocksEntities,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ReportFormatArg {
    Text,
    Json,
    Html,
}
