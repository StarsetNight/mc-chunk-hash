//! 通用工具：文件类型探测、region 坐标解析、目录遍历。

use std::error::Error;
use std::io::{Read, Seek, SeekFrom};
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::thread::available_parallelism;

use walkdir::WalkDir;

/// 存档 region 文件支持的文件类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Anvil,
    LinearV2,
    BLinearV3,
    /// linear v1 已过时，不提供解析，仅用于识别并给出明确报错。
    LinearV1,
}

/// linear v2 的 magic 头（与 b_linear v3 共用此前缀，靠 version 区分）。
pub const LINEAR_V2_MAGIC: [u8; 8] = [0xC3, 0xFF, 0x13, 0x18, 0x3C, 0xCA, 0x9D, 0x9A];
/// b_linear v3 的 magic 头。
pub const B_LINEAR_V3_MAGIC: [u8; 8] = [0xFF, 0xFF, 0xDF, 0xF7, 0xED, 0xDA, 0xFD, 0x97];

/// 从文件名解析 region 坐标 (x, z)。
///
/// 支持 `r.<x>.<z>.mca` / `r.<x>.<z>.linear` / `r.<x>.<z>.b_linear` 以及
/// 参考实现里的 `c.<x>.<z>.mcc` 命名。
pub fn parse_region_coords(path: impl AsRef<Path>) -> Result<(i32, i32), String> {
    let path = path.as_ref();
    if path.is_dir() {
        return Err("path is a directory".to_string());
    }
    let filename = path
        .file_name()
        .ok_or("no filename")?
        .to_str()
        .ok_or("invalid filename (non-utf8)")?;

    let parts: Vec<&str> = filename.split('.').collect();
    if parts.len() < 3 || (parts[0] != "r" && parts[0] != "c") {
        return Err(format!("invalid filename format: {filename}"));
    }
    let region_x: i32 = parts[1]
        .parse()
        .map_err(|e: ParseIntError| e.to_string())?;
    let region_z: i32 = parts[2]
        .parse()
        .map_err(|e: ParseIntError| e.to_string())?;
    Ok((region_x, region_z))
}

/// 简单判断 region 文件类型。
///
/// 因 mca 无文件头无法准确判断，仅 linear 系列可信；识别失败则兜底为 Anvil。
/// 参考实现 `get_file_type` 的语义。
pub fn get_file_type<R: Read + Seek>(file: &mut R) -> Result<FileType, Box<dyn Error + Send + Sync>> {
    let pos = file.stream_position()?;
    let mut magic = [0u8; 8];
    if file.read_exact(&mut magic).is_err() {
        // 文件太小，读不到 8 字节，视为 anvil（由后续解析报错或按空处理）。
        file.seek(SeekFrom::Start(pos))?;
        return Ok(FileType::Anvil);
    }
    file.seek(SeekFrom::Start(pos))?;

    match magic {
        B_LINEAR_V3_MAGIC => {
            let mut version = [0u8; 1];
            file.read_exact(&mut version)?;
            file.seek(SeekFrom::Start(pos))?;
            if version[0] != 3 {
                return Err("不受支持的 b_linear 版本".into());
            }
            Ok(FileType::BLinearV3)
        }
        LINEAR_V2_MAGIC => {
            let mut version = [0u8; 1];
            file.read_exact(&mut version)?;
            file.seek(SeekFrom::Start(pos))?;
            match version[0] {
                1 | 2 => Ok(FileType::LinearV1),
                3 => Ok(FileType::LinearV2),
                _ => Err("不受支持的 linear 版本".into()),
            }
        }
        _ => Ok(FileType::Anvil),
    }
}

/// 遍历目录收集指定后缀的文件。
pub fn get_dir_file(
    path: impl AsRef<Path>,
    suffix: &str,
    max_depth: usize,
) -> Result<Vec<PathBuf>, Box<dyn Error + Send + Sync>> {
    Ok(WalkDir::new(path)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext.to_string_lossy() == suffix)
                .unwrap_or(false)
        })
        .map(|e| e.into_path())
        .collect())
}

/// 收集一个存档目录下所有 region 文件（mca / linear / b_linear）。
pub fn collect_region_files(
    dir: impl AsRef<Path>,
    walk: bool,
) -> Result<Vec<PathBuf>, Box<dyn Error + Send + Sync>> {
    let max_depth = if walk { 114 } else { 1 };
    let d = dir.as_ref();
    let mut files = get_dir_file(d, "mca", max_depth)?;
    files.extend(get_dir_file(d, "linear", max_depth)?);
    files.extend(get_dir_file(d, "b_linear", max_depth)?);
    Ok(files)
}

/// 获取可用的 CPU 线程数。
pub fn get_cpu_num() -> usize {
    available_parallelism().map(|n| n.get()).unwrap_or(1)
}
