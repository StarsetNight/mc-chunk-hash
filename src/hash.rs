//! 区块 / region 的 xxh3-128 哈希计算。
//!
//! 哈希语义（与参考实现 `mc-linear-tool` 的 `Region::hash()` 对齐，
//! 但改用 xxh3-128 替代 xxh64）：
//! * 对 chunk 的**原始压缩字节**（`raw_chunk`）计算，而非解压后的 NBT，
//!   保证跨格式稳定且可复现。
//! * 空 chunk 以固定哨兵字节标记，保证布局稳定。
//! * region 哈希由该 region 内 1024 个 chunk 哈希的**有序串联**再哈希得出，
//!   而非重新读字节。

use xxhash_rust::xxh3::Xxh3;

/// 128 位哈希值，字节序为大端 16 字节。
pub type Hash128 = [u8; 16];

/// 计算单个 chunk 原始字节的 xxh3-128 哈希。
///
/// 空 chunk 返回全零哈希（与 `is_empty` 语义一致，避免空与非空歧义）。
pub fn hash_chunk(raw_chunk: &[u8]) -> Hash128 {
    if raw_chunk.is_empty() {
        return [0u8; 16];
    }
    let mut hasher = Xxh3::new();
    hasher.update(raw_chunk);
    let d = hasher.digest128();
    u128_to_be_bytes(d)
}

/// 由 1024 个 chunk 哈希有序串联，计算 region 整体哈希。
///
/// 这是 region 级粗粒度跳过的核心：region 哈希由 chunk 哈希串联得出，
/// 比对时若 region 哈希一致，则可跳过其内部 chunk 的细比对。
/// 传入的 `chunk_hashes` 必须按 chunk 索引（0..1024）有序，含空 chunk 的全零哈希。
pub fn hash_region_from_chunk_hashes(chunk_hashes: &[Hash128]) -> Hash128 {
    let mut hasher = Xxh3::new();
    for h in chunk_hashes {
        hasher.update(h);
    }
    u128_to_be_bytes(hasher.digest128())
}

/// 将 xxh3-128 的 u128 结果转为大端 `[u8; 16]`。
fn u128_to_be_bytes(v: u128) -> Hash128 {
    let mut out = [0u8; 16];
    out.copy_from_slice(&v.to_be_bytes());
    out
}

/// 把 16 字节哈希格式化为小写十六进制字符串。
pub fn hash_to_hex(h: &Hash128) -> String {
    let mut s = String::with_capacity(32);
    for b in h {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chunk_hash_is_zero() {
        assert_eq!(hash_chunk(b""), [0u8; 16]);
    }

    #[test]
    fn hash_is_deterministic() {
        let data = b"hello world, chunk data";
        assert_eq!(hash_chunk(data), hash_chunk(data));
    }

    #[test]
    fn hash_differs_on_content() {
        assert_ne!(hash_chunk(b"aaa"), hash_chunk(b"aab"));
    }

    #[test]
    fn region_hash_stable_order() {
        let hashes_a: Vec<Hash128> = [b"c0".as_slice(), b"c1".as_slice(), b"c2".as_slice()]
            .iter()
            .map(|c| hash_chunk(c))
            .collect();
        let hashes_b: Vec<Hash128> = [b"c0".as_slice(), b"c1".as_slice(), b"c2".as_slice()]
            .iter()
            .map(|c| hash_chunk(c))
            .collect();
        let a = hash_region_from_chunk_hashes(&hashes_a);
        let b = hash_region_from_chunk_hashes(&hashes_b);
        assert_eq!(a, b);
    }

    #[test]
    fn region_hash_order_sensitive() {
        let h1_in = [b"a".as_slice(), b"b".as_slice(), b"".as_slice()];
        let h2_in = [b"b".as_slice(), b"a".as_slice(), b"".as_slice()];
        let h1 = hash_region_from_chunk_hashes(&h1_in.iter().map(|c| hash_chunk(c)).collect::<Vec<_>>());
        let h2 = hash_region_from_chunk_hashes(&h2_in.iter().map(|c| hash_chunk(c)).collect::<Vec<_>>());
        assert_ne!(h1, h2);
    }

    #[test]
    fn region_hash_distinguishes_empty_vs_empty() {
        // 空 chunk 哈希为全零，非空 chunk 哈希几乎不可能全零；
        // 串联后的 region 哈希应当能区分 "全部空" 与 "含内容"。
        let all_empty: Vec<Hash128> = vec![[0u8; 16]; 3];
        let one_filled: Vec<Hash128> = vec![[0u8; 16], hash_chunk(b"x"), [0u8; 16]];
        assert_ne!(
            hash_region_from_chunk_hashes(&all_empty),
            hash_region_from_chunk_hashes(&one_filled)
        );
    }

    #[test]
    fn hex_format_is_32_chars_lowercase() {
        let h = hash_chunk(b"x");
        let hex = hash_to_hex(&h);
        assert_eq!(hex.len(), 32);
        assert_eq!(hex, hex.to_lowercase());
    }
}
