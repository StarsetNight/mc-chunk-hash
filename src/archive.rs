//! 存档解析：从 region 文件提取每个 chunk 的原始字节与坐标。
//!
//! 与参考实现 `mc-linear-tool` 的关键差异：这里**只做最小提取**，
//! 不为每个 chunk 构造完整 `Region`，也尽量按需解压，减少内存分配。
//!
//! 一个 region 固定包含 32×32 = 1024 个 chunk 槽位；空 chunk 返回空字节。

use std::error::Error;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use byteorder::{BigEndian, ReadBytesExt};
use flate2::read::{GzDecoder, ZlibDecoder};
use lz4_flex::frame::FrameDecoder;
use zstd::stream::read::Decoder as ZstdDecoder;

use crate::util::parse_region_coords;

/// 一个已解析的 chunk：世界坐标 + 原始字节（空则表示该槽位无数据）。
#[derive(Clone, Debug)]
pub struct ParsedChunk {
    pub x: i64,
    pub z: i64,
    pub raw: Vec<u8>,
}

impl ParsedChunk {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }
}

/// 一个解析完成的 region：坐标 + 1024 个 chunk（按 chunk 索引有序）。
#[derive(Clone, Debug)]
pub struct ParsedRegion {
    pub region_x: i32,
    pub region_z: i32,
    pub chunks: Vec<ParsedChunk>,
}

/// 根据文件路径自动探测类型并解析为 `ParsedRegion`。
pub fn parse_region_file(path: &Path) -> Result<ParsedRegion, Box<dyn Error + Send + Sync>> {
    let file_type = {
        let mut f = File::open(path)?;
        crate::util::get_file_type(&mut f)?
    };
    match file_type {
        crate::util::FileType::Anvil => parse_anvil(path),
        crate::util::FileType::LinearV2 => parse_linear_v2(path),
        crate::util::FileType::BLinearV3 => parse_b_linear_v3(path),
        crate::util::FileType::LinearV1 => {
            Err(format!("不支持已过时的 linear v1 格式: {}", path.display()).into())
        }
    }
}

/// 强制按 anvil 解析。
pub fn parse_region_file_anvil(path: &Path) -> Result<ParsedRegion, Box<dyn Error + Send + Sync>> {
    parse_anvil(path)
}

/// 强制按 linear v2 解析。
pub fn parse_region_file_linear_v2(
    path: &Path,
) -> Result<ParsedRegion, Box<dyn Error + Send + Sync>> {
    parse_linear_v2(path)
}

/// 强制按 b_linear v3 解析。
pub fn parse_region_file_b_linear_v3(
    path: &Path,
) -> Result<ParsedRegion, Box<dyn Error + Send + Sync>> {
    parse_b_linear_v3(path)
}

// ---------------------------------------------------------------------------
// Anvil (原版 .mca)
// ---------------------------------------------------------------------------

/// 读取 anvil 的 1024 个 location + 1024 个 timestamp，返回 (offset, count, ts)。
fn read_anvil_superblock(
    f: &mut File,
) -> Result<Vec<(u32, u8, u32)>, Box<dyn Error + Send + Sync>> {
    let mut infos = Vec::with_capacity(1024);
    for _ in 0..1024 {
        // location：3 字节 sector_offset（大端）+ 1 字节 sector_count
        let b0 = f.read_u8()?;
        let b1 = f.read_u8()?;
        let b2 = f.read_u8()?;
        let sector_count = f.read_u8()?;
        let sector_offset: u32 = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        infos.push((sector_offset, sector_count, 0u32));
    }
    // timestamp 表
    let mut i = 0;
    let mut tbuf = [0u8; 4];
    while i < 1024 {
        f.read_exact(&mut tbuf)?;
        infos[i].2 = u32::from_be_bytes(tbuf);
        i += 1;
    }
    Ok(infos)
}

/// 从 mca 文件解析全部 chunk。返回 1024 个 chunk（按索引），空槽位为 Vec::new()。
fn parse_anvil(path: &Path) -> Result<ParsedRegion, Box<dyn Error + Send + Sync>> {
    let (region_x, region_z) = parse_region_coords(path)?;
    let mut f = File::open(path)?;

    let infos = read_anvil_superblock(&mut f)?;

    let mut chunks: Vec<ParsedChunk> = Vec::with_capacity(1024);
    for (index, (sector_offset, sector_count, _ts)) in infos.iter().enumerate() {
        let x = region_x as i64 * 32 + (index % 32) as i64;
        let z = region_z as i64 * 32 + (index / 32) as i64;

        if *sector_offset == 0 || *sector_count == 0 {
            chunks.push(ParsedChunk { x, z, raw: Vec::new() });
            continue;
        }

        f.seek(SeekFrom::Start((*sector_offset as u64) * 4096))?;
        let chunk_length = f.read_u32::<BigEndian>()?;
        let compression_type = f.read_u8()?;

        // 外部 mcc（compression_type >= 128）：本阶段不做 mcc 外部读取，直接按空处理。
        // 参考实现会读同目录 c.x.z.mcc，这里为保持独立且聚焦哈希，遇到 mcc 报跳过。
        if compression_type >= 128 {
            // 仍视为"存在该 chunk"，但内容无法在本文件内取到；标记为原始压缩字节不可得。
            // 为保证语义一致，这里读取 mcc 文件（与参考实现一致）。
            let mcc_path = path.with_file_name(format!("c.{x}.{z}.mcc"));
            let raw = match File::open(&mcc_path) {
                Ok(mut mf) => {
                    let mut buf = Vec::new();
                    mf.read_to_end(&mut buf)?;
                    decompress(&buf, compression_type - 128)?
                }
                Err(_) => {
                    return Err(format!(
                        "anvil chunk ({x},{z}) 为外部 mcc 但读取失败: {}",
                        mcc_path.display()
                    )
                    .into())
                }
            };
            chunks.push(ParsedChunk { x, z, raw });
            continue;
        }

        let max_len = *sector_count as u32 * 4096 - 5;
        if chunk_length == 0 || chunk_length > max_len {
            // 数据异常，按空处理并继续（容错）。
            chunks.push(ParsedChunk { x, z, raw: Vec::new() });
            continue;
        }

        let mut compressed = vec![0u8; (chunk_length - 1) as usize];
        f.read_exact(&mut compressed)?;
        let raw = decompress(&compressed, compression_type)?;
        chunks.push(ParsedChunk { x, z, raw });
    }

    Ok(ParsedRegion { region_x, region_z, chunks })
}

/// 解压 anvil chunk 数据（compression_type: 1=gzip 2=zlib 3=无压缩 4=lz4）。
fn decompress(data: &[u8], compression_type: u8) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    match compression_type {
        1 => {
            let mut d = GzDecoder::new(Cursor::new(data));
            let mut out = Vec::new();
            d.read_to_end(&mut out)?;
            Ok(out)
        }
        2 => {
            let mut d = ZlibDecoder::new(Cursor::new(data));
            let mut out = Vec::new();
            d.read_to_end(&mut out)?;
            Ok(out)
        }
        3 => Ok(data.to_vec()),
        4 => {
            let mut d = FrameDecoder::new(Cursor::new(data));
            let mut out = Vec::new();
            d.read_to_end(&mut out)?;
            Ok(out)
        }
        _ => Err(format!("unknown compression type: {compression_type}").into()),
    }
}

// ---------------------------------------------------------------------------
// Linear v2
// ---------------------------------------------------------------------------

/// 读取 linear v2 的 SuperBlock（跳过 8 字节 magic，读 version/newest_ts/grid/region_x/region_z）。
#[allow(dead_code)]
struct LinearV2Header {
    version: u8,
    newest_timestamp: u64,
    grid_size: i8,
    region_x: i32,
    region_z: i32,
}

fn parse_linear_v2(path: &Path) -> Result<ParsedRegion, Box<dyn Error + Send + Sync>> {
    let mut f = File::open(path)?;

    // magic 8 字节
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic)?;
    debug_assert_eq!(magic, crate::util::LINEAR_V2_MAGIC);

    let version = f.read_u8()?;
    let newest_timestamp = f.read_u64::<BigEndian>()?;
    let grid_size = f.read_i8()?;
    let region_x = f.read_i32::<BigEndian>()?;
    let region_z = f.read_i32::<BigEndian>()?;
    let _header = LinearV2Header {
        version,
        newest_timestamp,
        grid_size,
        region_x,
        region_z,
    };

    if ![1, 2, 4, 8, 16, 32].contains(&grid_size) {
        return Err(format!("非法的 grid_size: {grid_size}").into());
    }

    // ChunkBitMap: 128 字节，128*8 = 1024 bit
    let mut bitmap_bytes = [0u8; 128];
    f.read_exact(&mut bitmap_bytes)?;
    let mut bitmap = [false; 1024];
    for i in 0..128 {
        let byte = bitmap_bytes[i];
        for j in 0..8 {
            bitmap[i * 8 + j] = (byte >> (7 - j)) & 1 == 1;
        }
    }

    // nbt_features: deserialize_hashmap —— 读 (len, bytes, value) 直到 len==0 结束标记
    loop {
        let key_len = f.read_u8()?;
        if key_len == 0 {
            break;
        }
        let mut key = vec![0u8; key_len as usize];
        f.read_exact(&mut key)?;
        let mut val = [0u8; 4];
        f.read_exact(&mut val)?;
        // 丢弃 nbt_features，我们不需要它
    }

    // buckets：header (bucket_size, compress_level, xxhash64) 各有 grid^2 个
    let bucket_count = (grid_size as usize) * (grid_size as usize);
    let mut bucket_headers: Vec<(u32, i8, u64)> = Vec::with_capacity(bucket_count);
    for _ in 0..bucket_count {
        let bucket_size = f.read_u32::<BigEndian>()?;
        let compress_level = f.read_i8()?;
        let xxhash = f.read_u64::<BigEndian>()?;
        bucket_headers.push((bucket_size, compress_level, xxhash));
    }

    // 读取并解压每个 bucket
    let mut buckets: Vec<Vec<u8>> = Vec::with_capacity(bucket_count);
    for (bucket_size, _cl, _h) in &bucket_headers {
        if *bucket_size == 0 {
            buckets.push(Vec::new());
            continue;
        }
        let mut compressed = vec![0u8; *bucket_size as usize];
        f.read_exact(&mut compressed)?;
        let decoded = zstd::decode_all(Cursor::new(&compressed))?;
        buckets.push(decoded);
    }

    // 将 bucket 内 chunk 按布局展平成 1024 个 chunk
    let chunks_per_bucket = 32 / grid_size as usize;
    let mut chunks: Vec<ParsedChunk> = Vec::with_capacity(1024);

    for chunk_index in 0..1024 {
        let x_in_region = chunk_index % 32;
        let z_in_region = chunk_index / 32;
        let bucket_x = x_in_region / chunks_per_bucket;
        let bucket_z = z_in_region / chunks_per_bucket;
        let ix = x_in_region % chunks_per_bucket;
        let iz = z_in_region % chunks_per_bucket;
        let bucket_index = bucket_x * grid_size as usize + bucket_z;

        let world_x = region_x as i64 * 32 + x_in_region as i64;
        let world_z = region_z as i64 * 32 + z_in_region as i64;

        let raw = if !bitmap[chunk_index] {
            Vec::new()
        } else if let Some(bucket) = buckets.get(bucket_index) {
            // 在 bucket 内解析第 local_index 个 chunk
            let local_index = ix * chunks_per_bucket + iz;
            extract_bucket_chunk(bucket, local_index)
        } else {
            Vec::new()
        };

        chunks.push(ParsedChunk { x: world_x, z: world_z, raw });
    }

    Ok(ParsedRegion { region_x, region_z, chunks })
}

/// 从 linear v2 解压后的 bucket 字节中提取第 `target_index` 个 chunk 的原始数据。
///
/// bucket 内每个 chunk 编码为 (chunk_size: u32, timestamp: u64, data: [size-8] 字节)；
/// 空 chunk 只占 4 字节（chunk_size == 0）。
fn extract_bucket_chunk(bucket: &[u8], target_index: usize) -> Vec<u8> {
    let mut cur = Cursor::new(bucket);
    let mut idx = 0usize;
    loop {
        let chunk_size = match cur.read_u32::<BigEndian>() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        if chunk_size < 8 {
            // 空 chunk 仅 4 字节头，无后续内容
            if idx == target_index {
                return Vec::new();
            }
            idx += 1;
            continue;
        }
        let data_len = (chunk_size - 8) as usize;
        let mut ts = [0u8; 8];
        if cur.read_exact(&mut ts).is_err() {
            return Vec::new();
        }
        let mut data = vec![0u8; data_len];
        if cur.read_exact(&mut data).is_err() {
            return Vec::new();
        }
        if idx == target_index {
            return data;
        }
        idx += 1;
    }
}

// ---------------------------------------------------------------------------
// b_linear v3
// ---------------------------------------------------------------------------

fn parse_b_linear_v3(path: &Path) -> Result<ParsedRegion, Box<dyn Error + Send + Sync>> {
    let (region_x, region_z) = parse_region_coords(path)?;
    let mut f = File::open(path)?;

    // magic 8 + version 1 + zstd_level 1 + xxhash32_seed 4
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic)?;
    debug_assert_eq!(magic, crate::util::B_LINEAR_V3_MAGIC);
    let version = f.read_u8()?;
    if version != 3 {
        return Err(format!("不受支持的 b_linear 版本: {version}").into());
    }
    let _zstd_level = f.read_i8()?;
    let _xxhash32_seed = f.read_u32::<BigEndian>()?;

    // BucketOffsetTable: 16 个 u64 offset
    let mut offsets = [0u64; 16];
    for o in offsets.iter_mut() {
        *o = f.read_u64::<BigEndian>()?;
    }

    // 对每个非零 bucket 定位并解析 64 个 chunk
    let mut chunks: Vec<Option<ParsedChunk>> = Vec::with_capacity(1024);
    for (_bucket_index, &offset) in offsets.iter().enumerate() {
        if offset == 0 {
            for _ in 0..64 {
                chunks.push(None);
            }
            continue;
        }
        f.seek(SeekFrom::Start(offset))?;
        let bucket_chunks = read_b_linear_bucket(&mut f)?;
        for c in bucket_chunks {
            chunks.push(c);
        }
    }

    // 展平并补世界坐标
    let mut result: Vec<ParsedChunk> = Vec::with_capacity(1024);
    for (chunk_index, c) in chunks.into_iter().enumerate() {
        let world_x = region_x as i64 * 32 + (chunk_index % 32) as i64;
        let world_z = region_z as i64 * 32 + (chunk_index / 32) as i64;
        match c {
            Some(mut pc) => {
                pc.x = world_x;
                pc.z = world_z;
                result.push(pc);
            }
            None => result.push(ParsedChunk { x: world_x, z: world_z, raw: Vec::new() }),
        }
    }

    Ok(ParsedRegion { region_x, region_z, chunks: result })
}

/// 解析 b_linear v3 的一个 bucket（64 个 chunk）。
fn read_b_linear_bucket(f: &mut File) -> Result<Vec<Option<ParsedChunk>>, Box<dyn Error + Send + Sync>> {
    let raw_size = f.read_u32::<BigEndian>()? as usize;
    let cmp_size = f.read_u32::<BigEndian>()? as usize;

    let mut compressed = vec![0u8; cmp_size];
    f.read_exact(&mut compressed)?;

    let mut decoder = ZstdDecoder::new(compressed.as_slice())?;
    let mut decompressed = Vec::with_capacity(raw_size);
    decoder.read_to_end(&mut decompressed)?;

    let mut cur = Cursor::new(decompressed);
    let mut chunks = Vec::with_capacity(64);

    for _ in 0..64 {
        let entry_size = cur.read_u32::<BigEndian>()?;
        if entry_size == 0 {
            chunks.push(None);
            continue;
        }
        // entry: chunk_size(u32) + write_timestamp(u64) + xxhash(u32) + data
        let mut entry = vec![0u8; entry_size as usize];
        cur.read_exact(&mut entry)?;
        let mut ec = Cursor::new(entry);
        let _chunk_size = ec.read_u32::<BigEndian>()?;
        let _write_timestamp = ec.read_u64::<BigEndian>()?;
        let _xxhash = ec.read_u32::<BigEndian>()?;
        let data_len = entry_size as usize - 16;
        let mut data = vec![0u8; data_len];
        ec.read_exact(&mut data)?;
        chunks.push(Some(ParsedChunk { x: 0, z: 0, raw: data }));
    }

    Ok(chunks)
}
