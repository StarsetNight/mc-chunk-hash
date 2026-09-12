//! 颗粒度可控的区块哈希：支持仅方块 / 方块+方块实体 / 原始字节。
//!
//! 通过解析 chunk 的 NBT 结构，只提取指定字段做规范化哈希，
//! 从而做到"只看方块、忽略实体移动/光照/时间戳"等。

use crate::hash::Hash128;
use fastnbt::Value;

/// 哈希内容模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashMode {
    /// 原始字节（默认）：对 chunk 整个原始字节算哈希，任何变化都检测。
    Raw,
    /// 仅方块：只算 `sections[].block_states`（方块 ID + 状态），忽略实体/生物群系/高度图/方块实体/时间戳。
    Blocks,
    /// 方块 + 方块实体：在 Blocks 基础上加上 `block_entities`（箱子/漏斗/告示牌等内容）。
    BlocksAndEntities,
}

impl HashMode {
    pub fn name(&self) -> &'static str {
        match self {
            HashMode::Raw => "raw",
            HashMode::Blocks => "blocks",
            HashMode::BlocksAndEntities => "blocks+entities",
        }
    }
}

/// 按指定模式计算 chunk 哈希。
///
/// `raw` 是 chunk 解压后的 NBT 字节。空 chunk（无数据）返回全零哈希。
pub fn hash_chunk_with_mode(raw: &[u8], mode: HashMode) -> Hash128 {
    if raw.is_empty() {
        return [0u8; 16];
    }
    match mode {
        HashMode::Raw => crate::hash::hash_chunk(raw),
        HashMode::Blocks | HashMode::BlocksAndEntities => {
            // 解析 NBT；解析失败则回退到原始字节哈希（保证不崩溃、仍能产生稳定值）
            match fastnbt::from_bytes::<Value>(raw) {
                Ok(root) => {
                    let extracted = extract(root, mode);
                    hash_canonical(&extracted)
                }
                Err(_) => crate::hash::hash_chunk(raw),
            }
        }
    }
}

/// 从 chunk NBT 根 compound 提取需要参与哈希的字段。
fn extract(root: Value, mode: HashMode) -> Value {
    let Value::Compound(mut map) = root else {
        // 非 compound（异常），返回空，用空哈希
        return Value::Compound(Default::default());
    };

    let sections = map.remove("sections").unwrap_or(Value::List(Vec::new()));
    let mut out = std::collections::HashMap::new();
    out.insert("sections".to_string(), extract_sections_blocks(sections));

    if mode == HashMode::BlocksAndEntities {
        let be = map.remove("block_entities").unwrap_or(Value::List(Vec::new()));
        out.insert("block_entities".to_string(), be);
    }

    Value::Compound(out)
}

/// 只保留每个 section 里的 `block_states`（方块数据），丢弃 biomes 等其他内容。
fn extract_sections_blocks(sections: Value) -> Value {
    let Value::List(list) = sections else {
        return Value::List(Vec::new());
    };

    let mut out = Vec::with_capacity(list.len());
    for sec in list {
        if let Value::Compound(mut sec_map) = sec {
            // 只保留 block_states；若无 block_states（全空气 section），保留一个空标记
            let bs = sec_map.remove("block_states");
            let mut m = std::collections::HashMap::new();
            m.insert("block_states".to_string(), bs.unwrap_or(Value::Compound(Default::default())));
            out.push(Value::Compound(m));
        } else {
            out.push(Value::List(Vec::new()));
        }
    }
    Value::List(out)
}

/// 将 `fastnbt::Value` 规范化为确定性的字节序列。
///
/// 保证：相同语义内容 → 相同字节，不受 HashMap 迭代顺序影响。
/// 规范化规则：
/// * Compound：按 key 排序后递归序列化；
/// * 数值：固定字节序（大端）编码；
/// * 数组：先长度后元素；
/// * 每个值带类型标记，避免不同类型同字节碰撞。
pub fn canonical_bytes(v: &Value) -> Vec<u8> {
    let mut buf = Vec::new();
    write_canonical(v, &mut buf);
    buf
}

fn write_canonical(v: &Value, buf: &mut Vec<u8>) {
    match v {
        Value::Byte(x) => {
            buf.push(0x01);
            buf.push(*x as u8);
        }
        Value::Short(x) => {
            buf.push(0x02);
            buf.extend_from_slice(&x.to_be_bytes());
        }
        Value::Int(x) => {
            buf.push(0x03);
            buf.extend_from_slice(&x.to_be_bytes());
        }
        Value::Long(x) => {
            buf.push(0x04);
            buf.extend_from_slice(&x.to_be_bytes());
        }
        Value::Float(x) => {
            buf.push(0x05);
            buf.extend_from_slice(&x.to_be_bytes());
        }
        Value::Double(x) => {
            buf.push(0x06);
            buf.extend_from_slice(&x.to_be_bytes());
        }
        Value::String(s) => {
            buf.push(0x07);
            let b = s.as_bytes();
            buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
            buf.extend_from_slice(b);
        }
        Value::ByteArray(a) => {
            buf.push(0x08);
            buf.extend_from_slice(&(a.len() as u32).to_be_bytes());
            for x in a.iter() {
                buf.push(*x as u8);
            }
        }
        Value::IntArray(a) => {
            buf.push(0x09);
            buf.extend_from_slice(&(a.len() as u32).to_be_bytes());
            for x in a.iter() {
                buf.extend_from_slice(&x.to_be_bytes());
            }
        }
        Value::LongArray(a) => {
            buf.push(0x0A);
            buf.extend_from_slice(&(a.len() as u32).to_be_bytes());
            for x in a.iter() {
                buf.extend_from_slice(&x.to_be_bytes());
            }
        }
        Value::List(list) => {
            buf.push(0x0B);
            buf.extend_from_slice(&(list.len() as u32).to_be_bytes());
            for item in list {
                write_canonical(item, buf);
            }
        }
        Value::Compound(map) => {
            buf.push(0x0C);
            // 按 key 排序，保证确定性
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            buf.extend_from_slice(&(keys.len() as u32).to_be_bytes());
            for k in keys {
                let kb = k.as_bytes();
                buf.extend_from_slice(&(kb.len() as u32).to_be_bytes());
                buf.extend_from_slice(kb);
                write_canonical(&map[k], buf);
            }
        }
    }
}

/// 对规范化字节计算 xxh3-128。
fn hash_canonical(v: &Value) -> Hash128 {
    let bytes = canonical_bytes(v);
    crate::hash::hash_chunk(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_mode_matches_hash_chunk() {
        let data = b"some raw bytes";
        assert_eq!(hash_chunk_with_mode(data, HashMode::Raw), crate::hash::hash_chunk(data));
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(hash_chunk_with_mode(b"", HashMode::Blocks), [0u8; 16]);
    }

    #[test]
    fn canonical_compound_is_order_independent() {
        // 同一个 compound，两个不同 key 插入顺序，规范化字节应一致
        let mut m1 = std::collections::HashMap::new();
        m1.insert("a".to_string(), Value::Int(1));
        m1.insert("b".to_string(), Value::Int(2));
        let mut m2 = std::collections::HashMap::new();
        m2.insert("b".to_string(), Value::Int(2));
        m2.insert("a".to_string(), Value::Int(1));
        let v1 = Value::Compound(m1);
        let v2 = Value::Compound(m2);
        assert_eq!(canonical_bytes(&v1), canonical_bytes(&v2));
    }

    #[test]
    fn canonical_distinguishes_types() {
        // Int 1 与 Byte 1 规范化字节应不同（类型标记不同）
        assert_ne!(canonical_bytes(&Value::Int(1)), canonical_bytes(&Value::Byte(1)));
    }

    /// 端到端验证：方块相同、实体不同时，blocks 模式哈希应一致，raw 模式应不同。
    #[test]
    fn blocks_mode_ignores_entities() {
        // 构造两个 chunk：sections（方块）相同，entities 不同
        let section = Value::Compound({
            let mut m = std::collections::HashMap::new();
            m.insert(
                "block_states".to_string(),
                Value::Compound({
                    let mut bs = std::collections::HashMap::new();
                    bs.insert("palette".to_string(), Value::List(vec![
                        Value::Compound({
                            let mut p = std::collections::HashMap::new();
                            p.insert("Name".to_string(), Value::String("minecraft:stone".to_string()));
                            p
                        }),
                    ]));
                    bs
                }),
            );
            m
        });

        let make_chunk = |entity_id: &str| {
            let mut root = std::collections::HashMap::new();
            root.insert("sections".to_string(), Value::List(vec![section.clone()]));
            root.insert("entities".to_string(), Value::List(vec![
                Value::Compound({
                    let mut e = std::collections::HashMap::new();
                    e.insert("id".to_string(), Value::String(entity_id.to_string()));
                    e
                }),
            ]));
            Value::Compound(root)
        };

        let chunk_a = make_chunk("minecraft:zombie");
        let chunk_b = make_chunk("minecraft:skeleton");

        // 序列化为 NBT 字节
        let bytes_a = fastnbt::to_bytes(&chunk_a).unwrap();
        let bytes_b = fastnbt::to_bytes(&chunk_b).unwrap();

        // blocks 模式：实体不同 → 哈希相同
        assert_eq!(
            hash_chunk_with_mode(&bytes_a, HashMode::Blocks),
            hash_chunk_with_mode(&bytes_b, HashMode::Blocks),
            "blocks 模式应忽略实体差异"
        );

        // raw 模式：整体字节不同 → 哈希不同
        assert_ne!(
            hash_chunk_with_mode(&bytes_a, HashMode::Raw),
            hash_chunk_with_mode(&bytes_b, HashMode::Raw),
            "raw 模式应检测到实体差异"
        );
    }

    /// 端到端验证：方块变化时，blocks 模式应能检测到。
    #[test]
    fn blocks_mode_detects_block_change() {
        let make_chunk = |block_name: &str| {
            let section = Value::Compound({
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "block_states".to_string(),
                    Value::Compound({
                        let mut bs = std::collections::HashMap::new();
                        bs.insert("palette".to_string(), Value::List(vec![
                            Value::Compound({
                                let mut p = std::collections::HashMap::new();
                                p.insert("Name".to_string(), Value::String(block_name.to_string()));
                                p
                            }),
                        ]));
                        bs
                    }),
                );
                m
            });
            let mut root = std::collections::HashMap::new();
            root.insert("sections".to_string(), Value::List(vec![section]));
            root.insert("entities".to_string(), Value::List(vec![]));
            Value::Compound(root)
        };

        let bytes_a = fastnbt::to_bytes(&make_chunk("minecraft:stone")).unwrap();
        let bytes_b = fastnbt::to_bytes(&make_chunk("minecraft:dirt")).unwrap();

        assert_ne!(
            hash_chunk_with_mode(&bytes_a, HashMode::Blocks),
            hash_chunk_with_mode(&bytes_b, HashMode::Blocks),
            "blocks 模式应检测到方块变化"
        );
    }

    /// 复现问题：玩家经过 → InhabitedTime / LastUpdate 变化 → raw 哈希变，但 blocks 不变。
    #[test]
    fn inhabited_time_causes_raw_change_but_not_blocks() {
        let make_chunk = |inhabited_time: i64, last_update: i64| {
            // 方块完全相同的 section
            let section = Value::Compound({
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "block_states".to_string(),
                    Value::Compound({
                        let mut bs = std::collections::HashMap::new();
                        bs.insert(
                            "palette".to_string(),
                            Value::List(vec![Value::Compound({
                                let mut p = std::collections::HashMap::new();
                                p.insert(
                                    "Name".to_string(),
                                    Value::String("minecraft:stone".to_string()),
                                );
                                p
                            })]),
                        );
                        bs
                    }),
                );
                m
            });

            let mut root = std::collections::HashMap::new();
            root.insert("sections".to_string(), Value::List(vec![section]));
            // 玩家相关、随游戏推进而变的元字段
            root.insert("InhabitedTime".to_string(), Value::Long(inhabited_time));
            root.insert("LastUpdate".to_string(), Value::Long(last_update));
            Value::Compound(root)
        };

        let before = fastnbt::to_bytes(&make_chunk(100, 1000)).unwrap();
        let after = fastnbt::to_bytes(&make_chunk(5000, 9000)).unwrap();

        // raw 模式：InhabitedTime/LastUpdate 变了 → 哈希变（这就是你观察到的现象）
        assert_ne!(
            hash_chunk_with_mode(&before, HashMode::Raw),
            hash_chunk_with_mode(&after, HashMode::Raw),
            "raw 模式：玩家经过使 InhabitedTime 变化，哈希改变"
        );

        // blocks 模式：只算方块，忽略这些元字段 → 哈希不变
        assert_eq!(
            hash_chunk_with_mode(&before, HashMode::Blocks),
            hash_chunk_with_mode(&after, HashMode::Blocks),
            "blocks 模式：玩家经过不改变方块，哈希应保持不变"
        );
    }
}
