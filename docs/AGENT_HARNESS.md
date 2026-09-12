# 给接手 AI Agent 的上下文种子（Harness Prompt）

> 将下面这段内容作为新 AI Agent 的**初始上下文**喂入，即可让它无需重读全部历史对话，
> 快速进入工作状态。配合 `HANDOFF.md` 一起使用。

---

你是接手 `mc-chunk-hash` 项目的开发 AI。这是一个 Rust 写的 Minecraft Java 版存档
区块哈希比对工具。在你接手之前，项目已经完成了一版可用实现，并经过一轮 debug，
留下了明确的结论和待办。请先完整阅读 `docs/HANDOFF.md`（技术交接文档），那是你的
主要上下文来源。

## 你需要知道的关键事实（摘要）

1. **项目位置**：`D:\Projects\mc-linear-tool`，是一个 Cargo workspace，唯一成员是
   `mc-chunk-hash/`。`reference/` 是旧项目源码，不编译，仅参考。

2. **核心功能**：`snapshot`（生成 chunk 哈希快照）→ `compare`（比对差异），
   三种哈希粒度见 `--hash-mode`（raw / blocks / blocks-entities）。

3. **最关键的技术结论**（已 debug 验证，不要推翻）：
   - `raw` 模式会对 chunk 整个原始 NBT 字节算哈希，`InhabitedTime`/`LastUpdate` 等
     随游戏推进变化的字段会导致"玩家经过就误报"。
   - `blocks` 模式只提取 `sections[].block_states`，能排除光照、时间戳、生物群系等，
     是"只看方块"的正确实现。
   - 工具**无 bug**：静止两次 snapshot 对比零变更（已实测验证）。
   - 真实存档的 chunk NBT 字段名是**小写** `sections`/`block_states`，`BlockLight`/
     `SkyLight` 是 section 内的独立 ByteArray 字段。

4. **遗留待办**（你的工作起点，详见 HANDOFF.md 第 8 节）：
   - ① 是否过滤"跑图导致的新 chunk 加载"（added/removed 误报）
   - ② 未验证 Linear v2/v3 格式下的 blocks 模式
   - ③ 考虑把 `bin/debug_chunk.rs` 正式化（当前是临时调试工具）

## 你的任务

按 HANDOFF.md 第 8 节的待办顺序推进，优先解决用户最关心的：
- 用户希望"只看方块、忽略一切非主动改动"，但目前跑图仍会带来少量真实方块变化
  （草径/雪/庄稼/新 chunk）被记录。请评估并实现过滤方案。

## 环境注意

- 依赖已全部下载到本地 `~/.cargo`，构建用 `cargo build --release --offline`。
- 测试存档路径含中文和括号，注意命令里加引号。
- 调试单 chunk 用 `cargo run --bin debug_chunk -- <mca> <chunk索引>`。
