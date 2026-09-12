# mc-chunk-hash

Minecraft 区块哈希快照与比对的独立工具。

从一个 Minecraft 存档扫描所有 region 文件，为每个 chunk 与每个 region 计算
**xxh3-128** 哈希，持久化为**快照文件**（rkyv 序列化 + mmap 读取），之后可对
两份快照做**差异比对**，借助 region 级整体哈希做粗粒度跳过，快速定位变更。

## 设计要点

- **哈希算法**：xxh3-128（128 位非加密哈希，快、抗碰撞好）。
- **快照持久化**：rkyv 零拷贝序列化 + mmap 映射读取，不整体反序列化。
- **两级哈希**：
  - chunk 级：对每个 chunk 算 xxh3-128。
  - region 级：由 1024 个 chunk 哈希有序串联再哈希。
- **粗粒度跳过**：比对比时先比 region 哈希，一致则跳过其内部 chunk 细比对。
- **支持格式**：Anvil（原版 `.mca`）、Linear v2（`.linear`）、Linear v3（`.b_linear`）。
  Linear v1 已过时，不支持。

### 哈希内容模式（`--hash-mode`）

可选择对 chunk 的哪些内容算哈希：

| 模式 | 说明 |
|---|---|
| `raw`（默认） | 对 chunk 整个原始字节算哈希，**任何变化**（含实体移动、时间戳）都检测 |
| `blocks` | 只解析并哈希 `sections[].block_states`（方块 ID + 状态），**忽略实体、生物群系、高度图、方块实体、时间戳** |
| `blocks-entities` | 在 `blocks` 基础上加上 `block_entities`（箱子/漏斗/告示牌等内容），仍忽略实体移动 |

> 用法示例：`mc-chunk-hash snapshot ./world -o s.bin --hash-mode blocks`
>
> 「只看方块不吃实体」即 `--hash-mode blocks`：生物移动、掉落物、刷怪都不会触发变更，
> 只有放置/破坏方块才会。

> 本工具从 [mc-linear-tool](https://github.com/XuanRikka/mc-linear-tool) 拆分而来，
> 其源码保留在 `reference/` 目录作为技术参考。

## 安装

```bash
cargo install --path mc-chunk-hash
```

## 使用

### 生成快照

```bash
mc-chunk-hash snapshot <存档目录> --output <快照文件>
# 可选：--walk 遍历多层子目录；--threads N 指定线程数；
#      --format anvil|linear-v2|b-linear-v3；--hash-mode raw|blocks|blocks-entities
```

### 比对

```bash
mc-chunk-hash compare <旧快照> <新快照>
# 可选：--format text|json|html 报告格式；--output <文件> 输出到文件
```

**报告格式**：

| 格式 | 说明 |
|---|---|
| `text`（默认） | 人类可读的纯文本报告 |
| `json` | 结构化 JSON，供脚本/程序消费 |
| `html` | 自包含的可视化 HTML 报告（打开浏览器即看） |

**HTML 可视化报告**包含：
- 汇总卡片：新增/删除/变更 region 数、变更 chunk 总数
- **世界大地图**：所有 region 按世界坐标拼成一张鸟瞰地图（滚轮缩放、拖拽平移、悬停看详情）
- region 网格图：单个 region 的 32×32 chunk 格点
- chunk 明细表：可展开查看每个变更 chunk 的新旧哈希对照

示例：

```bash
mc-chunk-hash compare baseline.bin current.bin --format html -o report.html
# 然后用浏览器打开 report.html
```

### 性能基准

```bash
mc-chunk-hash bench <存档目录>
```

## 快照格式

`SNAPSHOT_VERSION = 1` 的 rkyv 二进制：
- `Snapshot { version: u32, regions: Vec<RegionHash> }`
- `RegionHash { region_x: i32, region_z: i32, hash: [u8;16], chunks: Vec<ChunkHash> }`
- `ChunkHash { x: i64, z: i64, hash: [u8;16] }`

## 许可证

MIT
