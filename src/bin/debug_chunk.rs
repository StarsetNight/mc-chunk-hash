//! 调试工具：读取一个 mca 文件的指定 chunk，dump 其 NBT 结构。
//!
//! 用法：
//! ```bash
//! cargo run --bin debug_chunk -- <mca路径> [chunk索引默认0]
//! ```
//!
//! chunk 索引计算公式：`index = local_z * 32 + local_x`
//! 其中 `local_x = chunk世界坐标x mod 32`（负坐标需 +32 归一化到 0..32）。
//!
//! 用途：排查"为什么某个 chunk 的哈希变了"——dump 出来看是方块(sections)变了、
//! 还是光照(BlockLight/SkyLight)、还是元数据(InhabitedTime/LastUpdate)变了。
//!
//! 详细说明见 `docs/HANDOFF.md`。
use std::io::{Cursor, Read};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let chunk_idx: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    let data = std::fs::read(path).unwrap();
    let mut cur = Cursor::new(&data[..]);
    let mut locations = vec![(0u32, 0u8); 1024];
    for loc in locations.iter_mut() {
        let mut b = [0u8; 4];
        cur.read_exact(&mut b).unwrap();
        let offset = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        *loc = (offset, b[3]);
    }
    let (offset, count) = locations[chunk_idx];
    if offset == 0 || count == 0 {
        println!("chunk 为空");
        return;
    }
    let pos = (offset as usize) * 4096;
    let mut cur = Cursor::new(&data[pos..]);
    let mut lb = [0u8; 4];
    cur.read_exact(&mut lb).unwrap();
    let chunk_len = u32::from_be_bytes(lb);
    let mut cb = [0u8; 1];
    cur.read_exact(&mut cb).unwrap();
    let comp = cb[0];
    let mut compressed = vec![0u8; (chunk_len - 1) as usize];
    cur.read_exact(&mut compressed).unwrap();

    let mut out = Vec::new();
    match comp {
        1 => { flate2::read::GzDecoder::new(&compressed[..]).read_to_end(&mut out).unwrap(); }
        2 => { flate2::read::ZlibDecoder::new(&compressed[..]).read_to_end(&mut out).unwrap(); }
        3 => out = compressed,
        _ => panic!("comp {comp}"),
    };

    let root: fastnbt::Value = fastnbt::from_bytes(&out).unwrap();
    if let fastnbt::Value::Compound(map) = &root {
        // 顶层所有字段 + 类型概览
        let mut top_keys: Vec<&String> = map.keys().collect();
        top_keys.sort();
        println!("=== 顶层字段概览 ===");
        for k in top_keys {
            let t = match &map[k] {
                fastnbt::Value::Compound(_) => "Compound".to_string(),
                fastnbt::Value::List(l) => format!("List({})", l.len()),
                fastnbt::Value::LongArray(a) => format!("LongArray({})", a.len()),
                fastnbt::Value::ByteArray(a) => format!("ByteArray({})", a.len()),
                fastnbt::Value::IntArray(a) => format!("IntArray({})", a.len()),
                other => format!("{:?}", other),
            };
            println!("  {k}: {t}");
        }

        if let Some(fastnbt::Value::List(sections)) = map.get("sections") {
            println!("\n=== 所有 section 的字段 ===");
            for (i, sec) in sections.iter().enumerate() {
                if let fastnbt::Value::Compound(sec_map) = sec {
                    let mut sk: Vec<&String> = sec_map.keys().collect();
                    sk.sort();
                    let mut detail = Vec::new();
                    for k in &sk {
                        let t = match &sec_map[*k] {
                            fastnbt::Value::Compound(c) => format!("Compound({})", c.len()),
                            fastnbt::Value::List(l) => format!("List({})", l.len()),
                            fastnbt::Value::LongArray(a) => format!("LongArray({})", a.len()),
                            fastnbt::Value::ByteArray(a) => format!("ByteArray({})", a.len()),
                            fastnbt::Value::IntArray(a) => format!("IntArray({})", a.len()),
                            other => format!("{:?}", other),
                        };
                        detail.push(format!("{k}={t}"));
                    }
                    println!("  section[{i}]: {}", detail.join(", "));
                } else {
                    println!("  section[{i}]: 非 compound");
                }
            }
        }
    }
}
