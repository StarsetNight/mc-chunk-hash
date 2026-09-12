//! 快照模型与持久化。
//!
//! 采用 **rkyv 序列化 + mmap** 读写：
//! * 写出时把 `Snapshot` 用 rkyv 序列化为字节后落盘。
//! * 读入时用 mmap 映射文件，`check_archived_root` 零拷贝访问，
//!   不整体反序列化。
//!
//! 增量比对阶段利用 region 级哈希做粗粒度跳过，因此快照模型在 region 与
//! chunk 两级都保存哈希，命中未变化的 region 时无需细读 chunk。

use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use rkyv::{Archive, Deserialize, Serialize};

use crate::hash::Hash128;

/// 单个 chunk 的哈希记录。
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
pub struct ChunkHash {
    /// chunk 世界坐标 X。
    pub x: i64,
    /// chunk 世界坐标 Z。
    pub z: i64,
    /// chunk 原始字节的 xxh3-128 哈希。
    pub hash: Hash128,
}

/// 单个 region 的哈希记录。
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
pub struct RegionHash {
    /// region 坐标。
    pub region_x: i32,
    pub region_z: i32,
    /// region 整体哈希（由 chunk 哈希有序串联得出）。
    pub hash: Hash128,
    /// 该 region 内**非空** chunk 的哈希列表（按 chunk 索引升序）。
    pub chunks: Vec<ChunkHash>,
}

/// 整份快照。
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
pub struct Snapshot {
    /// 快照格式版本。
    pub version: u32,
    /// 所有 region（按 (region_x, region_z) 升序排列，保证扫描顺序稳定）。
    pub regions: Vec<RegionHash>,
}

/// 当前快照格式版本。
pub const SNAPSHOT_VERSION: u32 = 1;

impl Snapshot {
    pub fn new(regions: Vec<RegionHash>) -> Self {
        Snapshot {
            version: SNAPSHOT_VERSION,
            regions,
        }
    }

    /// 序列化到字节（rkyv AlignedVec）。
    pub fn to_bytes(&self) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        rkyv::to_bytes::<rkyv::rancor::Error>(self)
            .map(|a| a.to_vec())
            .map_err(|e| format!("序列化快照失败: {e}").into())
    }

    /// 写入文件。
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error + Send + Sync>> {
        let bytes = self.to_bytes()?;
        let mut f = File::create(path)?;
        f.write_all(&bytes)?;
        f.flush()?;
        Ok(())
    }
}

/// 已 mmap 映射的快照文件，提供零拷贝访问。
///
/// 注意：`ArchivedSnapshot` 借用于 mmap 缓冲区，因此必须与 mmap 缓冲区同生命周期。
/// 这里用 `OwnedMmap` 持有映射，`view` 返回借用。
pub struct MappedSnapshot {
    mmap: memmap2::Mmap,
}

impl MappedSnapshot {
    /// mmap 打开快照文件并校验。
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let file = File::open(path)?;
        // SAFETY: mmap 只读映射，文件在映射生命周期内不会被本进程修改。
        let mmap = unsafe { memmap2::Mmap::map(&file) }?;
        // 校验可反序列化
        let _ = Self::archive_root(&mmap)?;
        Ok(MappedSnapshot { mmap })
    }

    /// 零拷贝校验根对象。
    fn archive_root(
        mmap: &memmap2::Mmap,
    ) -> Result<&ArchivedSnapshot, Box<dyn Error + Send + Sync>> {
        rkyv::access::<ArchivedSnapshot, rkyv::rancor::Error>(mmap)
            .map_err(|e| format!("读取快照失败（可能格式不匹配或损坏）: {e}").into())
    }

    /// 获取归档根（借用 mmap 缓冲区）。
    pub fn view(&self) -> &ArchivedSnapshot {
        // SAFETY: 已在 open 中通过 access 校验，mmap 内容不可变且生命周期正确。
        unsafe { rkyv::access_unchecked::<ArchivedSnapshot>(self.mmap.as_ref()) }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> Snapshot {
        let region = RegionHash {
            region_x: 0,
            region_z: 0,
            hash: [1u8; 16],
            chunks: vec![
                ChunkHash { x: 0, z: 0, hash: [2u8; 16] },
                ChunkHash { x: 1, z: 0, hash: [3u8; 16] },
            ],
        };
        Snapshot::new(vec![region])
    }

    #[test]
    fn roundtrip_via_bytes() {
        let snap = sample_snapshot();
        let bytes = snap.to_bytes().unwrap();
        let restored: Snapshot = rkyv::from_bytes::<Snapshot, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(restored.version, SNAPSHOT_VERSION);
        assert_eq!(restored.regions.len(), 1);
        assert_eq!(restored.regions[0].region_x, 0);
        assert_eq!(restored.regions[0].chunks.len(), 2);
        assert_eq!(restored.regions[0].chunks[0].hash, [2u8; 16]);
    }

    #[test]
    fn roundtrip_via_file_and_mmap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snap.bin");
        sample_snapshot().write_to_file(&path).unwrap();

        let mapped = MappedSnapshot::open(&path).unwrap();
        let view = mapped.view();
        assert_eq!(view.version, SNAPSHOT_VERSION);
        assert_eq!(view.regions.len(), 1);
        assert_eq!(view.regions[0].region_x, 0);
        assert_eq!(view.regions[0].chunks.len(), 2);
    }
}
