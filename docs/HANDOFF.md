# mc-chunk-hash 项目交接文档

> 本文档面向**接手本项目的新开发人员或 AI Agent**。阅读本文档前你**没有任何前置上下文**，
> 请按顺序通读，即可掌握项目全貌、已完成的工作、以及当前遗留问题与下一步方向。

---

## 1. 项目是什么

`mc-chunk-hash` 是一个 **Minecraft Java 版存档区块哈希比对工具**。

核心用途：对 Minecraft 存档的每个 region 文件（`.mca` / `.linear` / `.b_linear`）扫描，
为其中每个 chunk（区块）计算一个 xxh3-128 哈希，持久化为"快照文件"；之后存档有改动时
再跑一次，比对两份快照，找出哪些 chunk 变了、新增了、删除了。

### 一句话流程

```
snapshot（生成快照） → 存档改动 → snapshot（再生成） → compare（比对差异）
```

### 典型使用场景

- 检测玩家在服务器上"改造了哪些区域"（建筑、挖矿、破坏）
- 追踪存档变化的范围与位置
- 用 HTML 报告在地图上可视化变化区域

---

## 2. 仓库布局

```
D:\Projects\mc-linear-tool\          # workspace 根
├── Cargo.toml                       # workspace 根（members = ["mc-chunk-hash"]）
├── Cargo.lock
├── .gitignore
├── mc-chunk-hash/                   # ★ 本项目（唯一被编译的成员）
│   ├── Cargo.toml
│   ├── README.md                    # 用户使用说明（简版）
│   └── src/
│       ├── main.rs                  # 入口 + 命令分发 + 多线程快照构建
│       ├── cli.rs                   # clap 命令行参数定义
│       ├── archive.rs               # 存档解析：从 region 文件提取 chunk 原始字节
│       ├── hash.rs                  # xxh3-128 哈希 + region 哈希串联
│       ├── blockhash.rs             # ★ 颗粒度可控哈希（raw/blocks/blocks+entities）
│       ├── snapshot.rs              # 快照模型 + rkyv 序列化 + mmap 读取
│       ├── compare.rs               # 比对引擎（region 级粗粒度跳过 + chunk 级细比对）
│       ├── report.rs                # 差异报告：text / json / html（含地图可视化）
│       ├── util.rs                  # 文件类型探测、坐标解析、目录遍历
│       └── bin/
│           └── debug_chunk.rs       # ★ 调试工具：dump 某个 chunk 的 NBT 结构
└── reference/                       # 原 mc-linear-tool 项目源码（仅作技术参考，不编译）
    ├── mclinear/                    # 原库（anvil/linear_v1/v2/v3 模型）
    ├── src/                         # 原 CLI
    └── ...
```

**重要**：`reference/` 是**上一版项目**（原 `mc-linear-tool`，一个格式转换工具）的完整源码，
只作技术参考，**不参与编译**（不在 workspace members 里）。本项目是从它拆出来重写的。

---

## 3. 技术栈与依赖

| 依赖 | 用途 |
|---|---|
| `clap` | CLI 参数解析 |
| `xxhash-rust`（`xxh3`） | xxh3-128 哈希 |
| `fastnbt` | 解析 chunk 的 NBT 数据（用于 blocks 模式） |
| `rkyv` | 快照零拷贝序列化 |
| `memmap2` | 快照 mmap 读取 |
| `zstd` / `flate2` / `lz4_flex` | 各格式的 chunk 解压 |
| `serde` / `serde_json` | JSON 报告 |
| `indicatif` / `humantime` / `walkdir` | 进度条 / 计时 / 目录遍历 |

**构建方式**：`cargo build --release`（依赖已全部下载到本地 `~/.cargo`，可加 `--offline`）。

---

## 4. 三个命令

```bash
# 1. 生成快照
mc-chunk-hash snapshot <存档目录> -o <快照文件> [--hash-mode raw|blocks|blocks-entities] [--walk]

# 2. 比对
mc-chunk-hash compare <旧快照> <新快照> [--format text|json|html] [-o 报告文件]

# 3. 测速
mc-chunk-hash bench <存档目录> [--hash-mode ...]
```

### 关键参数：`--hash-mode`（核心概念，务必理解）

| 模式 | 哈希内容 | 适用场景 |
|---|---|---|
| `raw`（默认） | chunk **整个原始 NBT 字节** | 检测任何变化（含实体/时间戳/光照） |
| `blocks` | **只提取 `sections[].block_states`**（方块 ID + 状态） | 只看方块，忽略光照/时间戳/生物群系等 |
| `blocks-entities` | blocks + `block_entities`（箱子/漏斗等内容） | 方块 + 方块实体，仍忽略实体移动 |

---

## 5. 各模块职责与关键实现细节

### 5.1 archive.rs —— 存档解析

从 region 文件提取每个 chunk 的**解压后原始 NBT 字节**：

- **Anvil（`.mca`）**：读 8KB header（1024 location + 1024 timestamp），按 sector 偏移读取
  每个 chunk，用 `flate2`（gzip/zlib）解压。参考 `reference/mclinear/src/models/anvil.rs`。
- **Linear v2**：读 SuperBlock → ChunkBitMap → nbt_features → buckets（zstd 解压）。
- **Linear v3（b_linear）**：读 Header → 16 个 bucket offset → 逐 bucket zstd 解压。
- 解析出的 `ParsedChunk.raw` 是**解压后的 NBT 字节**（非压缩），后续 blocks 模式直接吃它。

### 5.2 hash.rs —— 基础哈希

- `hash_chunk(raw)`: 对字节算 xxh3-128（空返回全零）。
- `hash_region_from_chunk_hashes(chunk_hashes)`: 1024 个 chunk 哈希**有序串联**再哈希，
  得到 region 整体哈希（用于粗粒度跳过）。

### 5.3 blockhash.rs —— ★ 核心：颗粒度可控哈希

这是本项目最关键、也是最近 debug 重点的模块。逻辑：

1. `hash_chunk_with_mode(raw, mode)`：
   - `Raw` → 直接 `hash_chunk(raw)`
   - `Blocks` / `BlocksAndEntities` → `fastnbt::from_bytes::<Value>(raw)` 解析成 NBT 树，
     调用 `extract()` 只保留需要的字段，再 `hash_canonical()` 算哈希。
2. `extract()`：只保留顶层 `sections`（和 blocks+entities 模式下的 `block_entities`），
   **丢弃** `InhabitedTime`、`LastUpdate`、`Heightmaps`、`block_ticks`、`fluid_ticks`、
   `structures`、`PostProcessing`、`isLightOn` 等所有元数据。
3. `extract_sections_blocks()`：每个 section 里**只保留 `block_states`**，
   丢弃 `biomes`、`BlockLight`、`SkyLight`（这三个是 section 内的独立字段）。
4. `write_canonical()`：把 `fastnbt::Value` 规范化为确定性字节（Compound 按 key 排序、
   每个值带类型标记、固定大端字节序），保证 HashMap 迭代顺序不影响哈希。

### 5.4 snapshot.rs —— 快照持久化

- 数据模型：`Snapshot { version, regions: Vec<RegionHash> }`
- `RegionHash { region_x, region_z, hash: [u8;16], chunks: Vec<ChunkHash> }`
- `ChunkHash { x, z, hash: [u8;16] }`
- 用 `rkyv` 序列化落盘，读时 `memmap2` mmap + `access_unchecked` 零拷贝访问。

### 5.5 compare.rs —— 比对引擎

- 先按 region 坐标做差集 → 得到 新增/删除 region。
- 共同 region：先比 **region 哈希**，相同则**整段跳过**（粗粒度跳过，核心提速点）；
  不同才进入 chunk 级细比对（按 chunk 世界坐标 join）。
- 输出 `CompareResult { added_regions, removed_regions, changed_regions }`。

### 5.6 report.rs —— 报告输出（含 HTML 地图）

三种格式：
- `text`：纯文本报告
- `json`：结构化 JSON
- `html`：**自包含可视化报告**，内嵌数据 JSON + CSS + JS，零外部依赖，浏览器打开即看。
  包含：汇总卡片、**世界大地图**（所有 region 按世界坐标拼成 canvas 鸟瞰图，滚轮缩放/
  拖拽平移/悬停看详情）、单个 region 的 32×32 网格、chunk 明细表。

### 5.7 bin/debug_chunk.rs —— ★ 调试工具

**这是交接后最常用的排查工具**。用法：

```bash
cargo run --bin debug_chunk -- <mca路径> <chunk索引>
```

作用：读取一个 mca 文件里指定索引的 chunk，解压并 dump 其 NBT 结构（顶层字段概览 +
所有 section 的字段类型与大小）。这是 debug"为什么哈希变了"的利器。

---

## 6. ★ 关键 debug 结论（务必理解，这是最近工作的核心产出）

### 结论 1：raw 模式会因"玩家经过"而大量误报

**现象**：用户"只跑图、没放方块"，raw 模式下报告了 3041 个 chunk 变更。

**原因**：chunk NBT 顶层有 `InhabitedTime`（玩家累计停留 tick）、`LastUpdate`（最后保存
时间）等**随游戏推进自动变化、但与方块无关**的字段。玩家只要踏入 chunk，`InhabitedTime`
就累加，导致整个 chunk 原始字节变化 → raw 哈希变化。

**验证**：测试 `inhabited_time_causes_raw_change_but_not_blocks` 已覆盖，通过。

### 结论 2：blocks 模式能排除 InhabitedTime，但仍会记录"跑图真实改动"

**现象**：改用 `blocks` 模式后，3041 → 20 个变更（大量噪声消失）。但用户"纯跑图 + 放俩
方块"仍有 20 个变更。

**分析**：
- 2 个 `added`/`removed`：跑图时**新加载/生成了 chunk**（真实变化）。
- 18 个 `changed`：跑图**真实改动了方块**（踩草地变草径、踩雪、踩庄稼、方块 tick 等），
  这些是方块层面真实变化，blocks 模式如实记录了。

### 结论 3：工具本身是稳定的（无 bug）

**验证方法**：用户**不进游戏、不动存档**，连续跑两次 `snapshot --hash-mode blocks`
再 compare，结果 **0 变更**。证明哈希是确定性的、工具无 bug。

### 结论 4：真实存档的 chunk NBT 结构（实测 dump）

接手后可直接参考，数据结构是（DataVersion 4903，很新的版本）：

```
chunk (root compound, 16 keys)
├── DataVersion = Int
├── xPos / yPos / zPos = Int
├── Status = String("minecraft:full")
├── InhabitedTime = Long        ★ 玩家停留，raw 模式下是误报元凶
├── LastUpdate = Long           ★ 同上
├── sections = List(24)         ★ 方块数据在这里
│   └── [i] (compound):
│       ├── Y = Byte             (section 的 Y 坐标)
│       ├── block_states (compound):
│       │   ├── palette = List   (方块类型列表，每项含 Name/Properties)
│       │   └── data = LongArray (方块状态索引打包数组)
│       ├── biomes (compound)    ★ blocks 模式已排除
│       ├── BlockLight = ByteArray(2048)  ★ 只在有光照的 section 出现，blocks 模式已排除
│       └── SkyLight  = ByteArray(2048)   ★ 同上
├── block_entities = List        (方块实体，如箱子/刷怪笼)
├── Heightmaps / structures / block_ticks / fluid_ticks / PostProcessing / isLightOn ...
```

**重点**：字段名是**小写** `sections`（不是旧版的 `Level.Sections`），`block_states` 也是
小写。`BlockLight`/`SkyLight` 是 section 内的**独立 ByteArray 字段**（仅在光照非零时存在），
不在 `block_states.data` 里。

---

## 7. 如何复现与验证

### 7.1 跑单元测试（19 个测试，全过）

```bash
cd D:\Projects\mc-linear-tool
cargo test --offline
```

### 7.2 生成真实存档快照并比对

```bash
cd D:\Projects\mc-linear-tool
cargo build --release --offline

# 存档目录（真实测试存档，路径含中文，注意引号）
$SAVE = "D:\Minecraft\.minecraft\versions\26.2-Liyuan\saves\漓苏NLY-孤云城邦（无MTR最终版本）"

# 生成基线快照（建议用 blocks 模式）
target\release\mc-chunk-hash.exe snapshot "$SAVE\dimensions\minecraft\overworld\region" -o baseline.bin --hash-mode blocks

# 改动存档后
target\release\mc-chunk-hash.exe snapshot "$SAVE\dimensions\minecraft\overworld\region" -o current.bin --hash-mode blocks

# 比对并生成 HTML 报告
target\release\mc-chunk-hash.exe compare baseline.bin current.bin --format html -o report.html
```

### 7.3 用 debug_chunk 排查单个 chunk

```bash
cargo run --bin debug_chunk -- "<mca路径>" <chunk索引>
```

chunk 索引计算公式：`index = local_z * 32 + local_x`，其中
`local_x = chunk世界x mod 32`（负坐标需 +32），`local_z` 同理。

---

## 8. 遗留问题 / 待办

1. **`added` / `removed` chunk 的语义需进一步向用户说明**：
   跑图导致的新 chunk 加载/生成会被报告为 added/removed，这是"真实变化"但用户可能
   觉得是噪声。是否需要一个选项过滤掉"纯加载导致的 chunk 出现/消失"，待定。

2. **"严格只看方块"仍会有被动方块变化误报**：
   踩草地→草径、踩雪、踩庄稼等"玩家移动间接导致的方块变化"在 blocks 模式仍会被记录。
   这是**正确行为**（方块确实变了），但若用户想只检测"主动放置/破坏"，需要额外逻辑
   （例如排除特定方块类型的 palette 变化），目前未做。

3. **Linear v2/v3 格式的 blocks 模式未在真实存档验证过**：
   当前 blocks 模式的 NBT 解析假设顶层是 `sections`（小写）。只对 Anvil 格式实测过。
   Linear 系列解压后的 chunk 如果 NBT 结构不同，blocks 模式可能提取为空（回退到 raw 哈希），
   需要实测确认。

4. **`IsLightOn` 字段**：这个 mod/版本特有的字段（顶层 `isLightOn`），blocks 模式已排除，
   但未验证它是否在某些版本里会影响 `block_states` 内部。

5. **debug_chunk.rs 目前是临时工具**：功能可用，但缺少完善的错误处理和参数校验，
   chunk 索引需手动计算，可考虑集成为主命令的 `debug` 子命令。

---

## 9. 交接给 AI Agent 的一句话摘要

> 这是一个 Rust 写的 Minecraft 存档 chunk 哈希比对工具。核心模块是 `blockhash.rs`
> （实现 raw/blocks/blocks+entities 三种哈希粒度，通过 fastnbt 解析 chunk NBT 并规范化）。
> 近期 debug 的核心结论是：raw 模式会因 `InhabitedTime` 等元字段导致"玩家经过就误报"，
> blocks 模式能排除这些，但跑图导致的真实方块变化（草径/雪/新增chunk）仍会被记录——
> 这是正确行为而非 bug，已通过"静止两次对比零变更"验证工具稳定性。
> 下一步重点是：① 决定是否过滤"新 chunk 加载"的 added/removed；② 验证 Linear 格式的
> blocks 模式；③ 考虑把 debug_chunk 正式化。

---

## 10. 常用命令速查

```bash
# 构建
cargo build --release --offline

# 测试
cargo test --offline

# 生成快照（blocks 模式，推荐）
mc-chunk-hash snapshot <存档region目录> -o s.bin --hash-mode blocks

# 比对（HTML 报告）
mc-chunk-hash compare old.bin new.bin --format html -o report.html

# 调试单 chunk
cargo run --bin debug_chunk -- <mca> <chunk索引>
```
