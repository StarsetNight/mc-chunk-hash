//! 快照比对引擎：region 级粗粒度跳过 + chunk 级细比对。

use std::collections::{HashMap, HashSet};

use crate::hash::Hash128;
use crate::snapshot::{ArchivedRegionHash, ArchivedSnapshot};

/// chunk 级差异。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChunkDiff {
    Added { x: i64, z: i64, new_hash: Hash128 },
    Removed { x: i64, z: i64, old_hash: Hash128 },
    Changed { x: i64, z: i64, old_hash: Hash128, new_hash: Hash128 },
}

/// region 级差异。
#[derive(Clone, Debug)]
pub enum RegionDiff {
    /// 新增 region（新快照有、旧快照无），含其中所有 chunk。
    Added {
        region_x: i32,
        region_z: i32,
        chunks: Vec<(i64, i64, Hash128)>,
    },
    /// 删除 region（旧快照有、新快照无），含其中所有 chunk。
    Removed {
        region_x: i32,
        region_z: i32,
        chunks: Vec<(i64, i64, Hash128)>,
    },
    /// region 哈希变化的 region，含 chunk 级差异明细。
    Changed {
        region_x: i32,
        region_z: i32,
        old_hash: Hash128,
        new_hash: Hash128,
        chunk_diffs: Vec<ChunkDiff>,
    },
}

/// 比对结果汇总。
#[derive(Default, Clone, Debug)]
pub struct CompareResult {
    pub added_regions: Vec<RegionDiff>,
    pub removed_regions: Vec<RegionDiff>,
    pub changed_regions: Vec<RegionDiff>,
}

impl CompareResult {
    pub fn added_region_count(&self) -> usize {
        self.added_regions.len()
    }
    pub fn removed_region_count(&self) -> usize {
        self.removed_regions.len()
    }
    pub fn changed_region_count(&self) -> usize {
        self.changed_regions.len()
    }
    /// 所有变更 chunk 的总数（含新增/删除 region 内的 chunk，以及变更 region 内的 chunk diff 数）。
    pub fn changed_chunk_count(&self) -> usize {
        let mut n = 0;
        for r in &self.added_regions {
            if let RegionDiff::Added { chunks, .. } = r {
                n += chunks.len();
            }
        }
        for r in &self.removed_regions {
            if let RegionDiff::Removed { chunks, .. } = r {
                n += chunks.len();
            }
        }
        for r in &self.changed_regions {
            if let RegionDiff::Changed { chunk_diffs, .. } = r {
                n += chunk_diffs.len();
            }
        }
        n
    }
}

fn region_key(rx: i32, rz: i32) -> (i32, i32) {
    (rx, rz)
}

/// 比较两份快照，返回差异。
pub fn compare(old: &ArchivedSnapshot, new: &ArchivedSnapshot) -> CompareResult {
    let old_regions: HashMap<(i32, i32), &ArchivedRegionHash> = old
        .regions
        .iter()
        .map(|r| (region_key(i32::from(r.region_x), i32::from(r.region_z)), r))
        .collect();
    let new_regions: HashMap<(i32, i32), &ArchivedRegionHash> = new
        .regions
        .iter()
        .map(|r| (region_key(i32::from(r.region_x), i32::from(r.region_z)), r))
        .collect();

    let old_keys: HashSet<(i32, i32)> = old_regions.keys().copied().collect();
    let new_keys: HashSet<(i32, i32)> = new_regions.keys().copied().collect();

    let mut result = CompareResult::default();

    // 新增 region（新有旧无）
    let mut added_keys: Vec<_> = new_keys.difference(&old_keys).copied().collect();
    added_keys.sort();
    for key in added_keys {
        let r = new_regions[&key];
        result.added_regions.push(RegionDiff::Added {
            region_x: i32::from(r.region_x),
            region_z: i32::from(r.region_z),
            chunks: collect_chunks(r),
        });
    }

    // 删除 region（旧有新无）
    let mut removed_keys: Vec<_> = old_keys.difference(&new_keys).copied().collect();
    removed_keys.sort();
    for key in removed_keys {
        let r = old_regions[&key];
        result.removed_regions.push(RegionDiff::Removed {
            region_x: i32::from(r.region_x),
            region_z: i32::from(r.region_z),
            chunks: collect_chunks(r),
        });
    }

    // 共同 region：先比 region 哈希，相同则跳过
    let mut common_keys: Vec<_> = old_keys.intersection(&new_keys).copied().collect();
    common_keys.sort();
    for key in common_keys {
        let old_r = old_regions[&key];
        let new_r = new_regions[&key];
        if old_r.hash == new_r.hash {
            // 粗粒度跳过：region 哈希一致，内部 chunk 视为未变
            continue;
        }
        let chunk_diffs = diff_chunks(old_r, new_r);
        result.changed_regions.push(RegionDiff::Changed {
            region_x: i32::from(old_r.region_x),
            region_z: i32::from(old_r.region_z),
            old_hash: hash_arr(&old_r.hash),
            new_hash: hash_arr(&new_r.hash),
            chunk_diffs,
        });
    }

    result
}

/// 将 archived 的哈希字段（借用）转为 `Hash128`。
///
/// rkyv 把 `[u8; 16]` 归档为 `[u8; 16]`（u8 无字节序），故此字段可直接拷贝。
#[inline]
fn hash_arr(h: &[u8; 16]) -> Hash128 {
    *h
}

fn collect_chunks(r: &ArchivedRegionHash) -> Vec<(i64, i64, Hash128)> {
    r.chunks
        .iter()
        .map(|c| (i64::from(c.x), i64::from(c.z), hash_arr(&c.hash)))
        .collect()
}

/// 对同一 region 的两份 chunk 列表做细比对。
///
/// 说明：快照内的 `chunks` 只存了非空 chunk 的哈希。空 chunk（槽位无数据）在
/// 快照中不出现。因此新增/删除/变更都以"非空 chunk 的存在性 + 哈希"来判定。
fn diff_chunks(old_r: &ArchivedRegionHash, new_r: &ArchivedRegionHash) -> Vec<ChunkDiff> {
    let old_map: HashMap<(i64, i64), Hash128> = old_r
        .chunks
        .iter()
        .map(|c| ((i64::from(c.x), i64::from(c.z)), hash_arr(&c.hash)))
        .collect();
    let new_map: HashMap<(i64, i64), Hash128> = new_r
        .chunks
        .iter()
        .map(|c| ((i64::from(c.x), i64::from(c.z)), hash_arr(&c.hash)))
        .collect();

    let old_keys: HashSet<_> = old_map.keys().copied().collect();
    let new_keys: HashSet<_> = new_map.keys().copied().collect();

    let mut diffs = Vec::new();

    // 新增 chunk
    let mut added: Vec<_> = new_keys.difference(&old_keys).copied().collect();
    added.sort();
    for key in added {
        if let Some(h) = new_map.get(&key) {
            diffs.push(ChunkDiff::Added { x: key.0, z: key.1, new_hash: *h });
        }
    }

    // 删除 chunk
    let mut removed: Vec<_> = old_keys.difference(&new_keys).copied().collect();
    removed.sort();
    for key in removed {
        if let Some(h) = old_map.get(&key) {
            diffs.push(ChunkDiff::Removed { x: key.0, z: key.1, old_hash: *h });
        }
    }

    // 变更 chunk
    let mut changed: Vec<_> = old_keys.intersection(&new_keys).copied().collect();
    changed.sort();
    for key in changed {
        let oh = old_map[&key];
        let nh = new_map[&key];
        if oh != nh {
            diffs.push(ChunkDiff::Changed { x: key.0, z: key.1, old_hash: oh, new_hash: nh });
        }
    }

    diffs
}
