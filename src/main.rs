//! mc-chunk-hash 入口：命令分发。

mod archive;
mod blockhash;
mod cli;
mod compare;
mod hash;
mod report;
mod snapshot;
mod util;

use std::collections::VecDeque;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use clap::Parser;
use humantime::format_duration;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use cli::{ArchiveFormatArg, Cli, Command, HashModeArg, ReportFormatArg};
use blockhash::HashMode;
use hash::Hash128;
use report::ReportFormat;
use snapshot::{ChunkHash, MappedSnapshot, RegionHash, Snapshot};
use util::{collect_region_files, FileType, get_cpu_num};

fn main() {
    if let Err(e) = run() {
        eprintln!("错误: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let cli = Cli::parse();
    let threads = cli.threads.unwrap_or_else(get_cpu_num).max(1);

    match cli.command {
        Command::Snapshot(args) => {
            let mode = hash_mode_from_arg(args.hash_mode);
            let snap = build_snapshot(&args.input_dir, args.walk, args.format, mode, threads)?;
            snap.write_to_file(&args.output)?;
            println!(
                "已生成快照: {} （{} 个 region，哈希模式: {}）",
                args.output.display(),
                snap.regions.len(),
                mode.name()
            );
        }
        Command::Compare(args) => {
            let old = MappedSnapshot::open(&args.old)?;
            let new = MappedSnapshot::open(&args.new)?;
            let result = compare::compare(old.view(), new.view());

            let fmt = match args.format {
                ReportFormatArg::Text => ReportFormat::Text,
                ReportFormatArg::Json => ReportFormat::Json,
                ReportFormatArg::Html => ReportFormat::Html,
            };

            match args.output {
                Some(path) => {
                    let mut f = fs::File::create(&path)?;
                    report::write_report(&result, fmt, &mut f)?;
                    println!("报告已写入: {}", path.display());
                }
                None => {
                    report::write_report(&result, fmt, &mut std::io::stdout())?;
                }
            }
        }
        Command::Bench(args) => {
            let mode = hash_mode_from_arg(args.hash_mode);
            bench(&args.input_dir, args.walk, args.format, mode, threads)?;
        }
    }

    Ok(())
}

/// 将 CLI 的 HashModeArg 转为内部 HashMode。
fn hash_mode_from_arg(arg: HashModeArg) -> HashMode {
    match arg {
        HashModeArg::Raw => HashMode::Raw,
        HashModeArg::Blocks => HashMode::Blocks,
        HashModeArg::BlocksEntities => HashMode::BlocksAndEntities,
    }
}

/// 扫描存档，构建快照（并行）。
fn build_snapshot(
    input_dir: &PathBuf,
    walk: bool,
    format: Option<ArchiveFormatArg>,
    hash_mode: HashMode,
    threads: usize,
) -> Result<Snapshot, Box<dyn Error + Send + Sync>> {
    if !input_dir.is_dir() || !input_dir.exists() {
        return Err("输入目录不存在或不是目录".into());
    }

    let files = collect_region_files(input_dir, walk)?;
    let total = files.len();

    let mp = Arc::new(MultiProgress::new());
    let pb = mp.add(ProgressBar::new(total as u64));
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{wide_bar:.cyan/blue}] {pos}/{len} {per_sec} {msg}",
        )?
        .progress_chars("=>-"),
    );
    pb.set_message("扫描中");

    let tasks: Arc<Mutex<VecDeque<PathBuf>>> = Arc::new(Mutex::new(files.into_iter().collect()));

    // 每个线程产出若干 RegionHash，最后汇总
    let results: Arc<Mutex<Vec<RegionHash>>> = Arc::new(Mutex::new(Vec::new()));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let tasks = Arc::clone(&tasks);
        let results = Arc::clone(&results);
        let errors = Arc::clone(&errors);
        let pb = pb.clone();
        let mp = Arc::clone(&mp);
        handles.push(std::thread::spawn(move || {
            loop {
                let path = {
                    let mut q = tasks.lock().unwrap();
                    q.pop_front()
                };
                let Some(path) = path else { break };

                match process_region_file(&path, format, hash_mode) {
                    Ok(rh) => {
                        results.lock().unwrap().push(rh);
                        pb.inc(1);
                    }
                    Err(e) => {
                        errors.lock().unwrap().push(format!("{}: {e}", path.display()));
                        pb.inc(1);
                    }
                }
                let _ = &mp;
            }
        }));
    }

    for h in handles {
        h.join().map_err(|_| "工作线程 panic".to_string())?;
    }

    pb.finish_with_message("完成");

    let errs = errors.lock().unwrap().clone();
    if !errs.is_empty() {
        // 打印但继续：单个文件失败不阻断整体
        for e in &errs {
            eprintln!("警告: {e}");
        }
    }

    let mut regions = results.lock().unwrap().clone();
    regions.sort_by(|a, b| (a.region_x, a.region_z).cmp(&(b.region_x, b.region_z)));

    Ok(Snapshot::new(regions))
}

/// 解析单个 region 文件，产出 `RegionHash`。
fn process_region_file(
    path: &PathBuf,
    format: Option<ArchiveFormatArg>,
    hash_mode: HashMode,
) -> Result<RegionHash, Box<dyn Error + Send + Sync>> {
    // 若指定了强制格式，跳过自动探测（但仍需校验文件类型是否匹配可解析）
    if let Some(fmt) = format {
        // 仅对 linear 系列有意义；anvil 无 magic 直接解析
        match fmt {
            ArchiveFormatArg::Anvil => {
                let r = archive::parse_region_file_anvil(path)?;
                return Ok(region_to_hash(&r, hash_mode));
            }
            ArchiveFormatArg::LinearV2 => {
                let r = archive::parse_region_file_linear_v2(path)?;
                return Ok(region_to_hash(&r, hash_mode));
            }
            ArchiveFormatArg::BLinearV3 => {
                let r = archive::parse_region_file_b_linear_v3(path)?;
                return Ok(region_to_hash(&r, hash_mode));
            }
            ArchiveFormatArg::Auto => {}
        }
    }

    let r = archive::parse_region_file(path)?;
    Ok(region_to_hash(&r, hash_mode))
}

/// 从解析出的 region 计算 chunk 哈希 + region 哈希。
fn region_to_hash(r: &archive::ParsedRegion, hash_mode: HashMode) -> RegionHash {
    let mut chunk_hashes: Vec<Hash128> = Vec::with_capacity(1024);
    let mut chunks: Vec<ChunkHash> = Vec::new();

    for c in &r.chunks {
        let h = blockhash::hash_chunk_with_mode(&c.raw, hash_mode);
        chunk_hashes.push(h);
        if !c.is_empty() {
            chunks.push(ChunkHash {
                x: c.x,
                z: c.z,
                hash: h,
            });
        }
    }

    // region 哈希由 chunk 哈希有序串联
    let region_hash = hash::hash_region_from_chunk_hashes(&chunk_hashes);

    RegionHash {
        region_x: r.region_x,
        region_z: r.region_z,
        hash: region_hash,
        chunks,
    }
}

/// bench：跑一遍完整快照流程并计时。
fn bench(
    input_dir: &PathBuf,
    walk: bool,
    format: Option<ArchiveFormatArg>,
    hash_mode: HashMode,
    threads: usize,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let start = Instant::now();
    let snap = build_snapshot(input_dir, walk, format, hash_mode, threads)?;
    let elapsed = start.elapsed();

    println!("========== 性能基准 ==========");
    println!("哈希模式: {}", hash_mode.name());
    println!("region 文件数: {}", snap.regions.len());
    let total_chunks: usize = snap.regions.iter().map(|r| r.chunks.len()).sum();
    println!("非空 chunk 数: {}", total_chunks);
    println!("总耗时: {}", format_duration(elapsed));
    let secs = elapsed.as_secs_f64().max(1e-9);
    println!("速率: {:.2} region/s", snap.regions.len() as f64 / secs);

    // 序列化耗时（落盘）
    let bytes = snap.to_bytes()?;
    println!("快照序列化后大小: {:.2} KiB", bytes.len() as f64 / 1024.0);

    Ok(())
}

// 供 cli 使用的格式类型 re-export（仅测试用）
#[allow(unused)]
fn _assert_types() {
    let _: FileType = FileType::Anvil;
}
