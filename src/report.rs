//! 差异报告输出（文本 / JSON / HTML 可视化）。

use std::io::{self, Write};

use serde::Serialize;

use crate::compare::{ChunkDiff, CompareResult, RegionDiff};
use crate::hash::hash_to_hex;

/// 输出格式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportFormat {
    Text,
    Json,
    Html,
}

/// 生成人类可读的文本报告。
pub fn format_text(result: &CompareResult) -> String {
    let mut s = String::new();
    s.push_str("========== 区块哈希比对报告 ==========\n");
    s.push_str(&format!("新增 region: {}\n", result.added_region_count()));
    s.push_str(&format!("删除 region: {}\n", result.removed_region_count()));
    s.push_str(&format!("变更 region: {}\n", result.changed_region_count()));
    s.push_str(&format!("变更 chunk 总数: {}\n", result.changed_chunk_count()));
    s.push('\n');

    if result.added_region_count() + result.removed_region_count() + result.changed_region_count() == 0 {
        s.push_str("无差异。\n");
        return s;
    }

    for r in &result.added_regions {
        if let RegionDiff::Added { region_x, region_z, chunks } = r {
            s.push_str(&format!("[新增 region] ({region_x}, {region_z}) —— 含 {} 个 chunk\n", chunks.len()));
            for (x, z, _h) in chunks {
                s.push_str(&format!("    + chunk ({x}, {z})\n"));
            }
        }
    }
    for r in &result.removed_regions {
        if let RegionDiff::Removed { region_x, region_z, chunks } = r {
            s.push_str(&format!("[删除 region] ({region_x}, {region_z}) —— 含 {} 个 chunk\n", chunks.len()));
            for (x, z, _h) in chunks {
                s.push_str(&format!("    - chunk ({x}, {z})\n"));
            }
        }
    }
    for r in &result.changed_regions {
        if let RegionDiff::Changed { region_x, region_z, old_hash, new_hash, chunk_diffs } = r {
            s.push_str(&format!(
                "[变更 region] ({region_x}, {region_z})\n    old: {}\n    new: {}\n",
                hash_to_hex(old_hash),
                hash_to_hex(new_hash)
            ));
            for d in chunk_diffs {
                match d {
                    ChunkDiff::Added { x, z, new_hash } => {
                        s.push_str(&format!("    + chunk ({x}, {z})  {}", hash_to_hex(new_hash)));
                    }
                    ChunkDiff::Removed { x, z, old_hash } => {
                        s.push_str(&format!("    - chunk ({x}, {z})  {}", hash_to_hex(old_hash)));
                    }
                    ChunkDiff::Changed { x, z, old_hash, new_hash } => {
                        s.push_str(&format!(
                            "    ~ chunk ({x}, {z})\n        {} -> {}",
                            hash_to_hex(old_hash),
                            hash_to_hex(new_hash)
                        ));
                    }
                }
                s.push('\n');
            }
        }
    }

    s
}

/// 生成 JSON 报告。
pub fn format_json(result: &CompareResult) -> Result<String, serde_json::Error> {
    let out = JsonReport::from(result);
    serde_json::to_string_pretty(&out)
}

/// 生成自包含的 HTML 可视化报告。
pub fn format_html(result: &CompareResult) -> String {
    let data = HtmlReport::from(result);
    let data_json = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());
    html_template(&data_json)
}

/// 将比对结果写出到 writer 或 stdout。
pub fn write_report<W: Write>(
    result: &CompareResult,
    format: ReportFormat,
    writer: &mut W,
) -> io::Result<()> {
    match format {
        ReportFormat::Text => write!(writer, "{}", format_text(result)),
        ReportFormat::Json => {
            let s = format_json(result).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            write!(writer, "{s}")
        }
        ReportFormat::Html => write!(writer, "{}", format_html(result)),
    }
}

// ---------------------------------------------------------------------------
// JSON 序列化模型
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct JsonReport {
    added_regions: usize,
    removed_regions: usize,
    changed_regions: usize,
    changed_chunks: usize,
    details: JsonDetails,
}

#[derive(Serialize)]
struct JsonDetails {
    added: Vec<JsonRegion>,
    removed: Vec<JsonRegion>,
    changed: Vec<JsonChangedRegion>,
}

#[derive(Serialize)]
struct JsonRegion {
    region_x: i32,
    region_z: i32,
    chunk_count: usize,
    chunks: Vec<JsonChunk>,
}

#[derive(Serialize)]
struct JsonChunk {
    x: i64,
    z: i64,
}

#[derive(Serialize)]
struct JsonChangedRegion {
    region_x: i32,
    region_z: i32,
    old_hash: String,
    new_hash: String,
    chunks: Vec<JsonChunkDiff>,
}

#[derive(Serialize)]
struct JsonChunkDiff {
    #[serde(rename = "type")]
    kind: &'static str,
    x: i64,
    z: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_hash: Option<String>,
}

impl From<&CompareResult> for JsonReport {
    fn from(r: &CompareResult) -> Self {
        let added: Vec<JsonRegion> = r
            .added_regions
            .iter()
            .map(|rd| match rd {
                RegionDiff::Added { region_x, region_z, chunks } => JsonRegion {
                    region_x: *region_x,
                    region_z: *region_z,
                    chunk_count: chunks.len(),
                    chunks: chunks.iter().map(|(x, z, _)| JsonChunk { x: *x, z: *z }).collect(),
                },
                _ => unreachable!(),
            })
            .collect();

        let removed: Vec<JsonRegion> = r
            .removed_regions
            .iter()
            .map(|rd| match rd {
                RegionDiff::Removed { region_x, region_z, chunks } => JsonRegion {
                    region_x: *region_x,
                    region_z: *region_z,
                    chunk_count: chunks.len(),
                    chunks: chunks.iter().map(|(x, z, _)| JsonChunk { x: *x, z: *z }).collect(),
                },
                _ => unreachable!(),
            })
            .collect();

        let changed: Vec<JsonChangedRegion> = r
            .changed_regions
            .iter()
            .map(|rd| match rd {
                RegionDiff::Changed { region_x, region_z, old_hash, new_hash, chunk_diffs } => {
                    JsonChangedRegion {
                        region_x: *region_x,
                        region_z: *region_z,
                        old_hash: hash_to_hex(old_hash),
                        new_hash: hash_to_hex(new_hash),
                        chunks: chunk_diffs
                            .iter()
                            .map(|d| match d {
                                ChunkDiff::Added { x, z, new_hash } => JsonChunkDiff {
                                    kind: "added",
                                    x: *x,
                                    z: *z,
                                    old_hash: None,
                                    new_hash: Some(hash_to_hex(new_hash)),
                                },
                                ChunkDiff::Removed { x, z, old_hash } => JsonChunkDiff {
                                    kind: "removed",
                                    x: *x,
                                    z: *z,
                                    old_hash: Some(hash_to_hex(old_hash)),
                                    new_hash: None,
                                },
                                ChunkDiff::Changed { x, z, old_hash, new_hash } => JsonChunkDiff {
                                    kind: "changed",
                                    x: *x,
                                    z: *z,
                                    old_hash: Some(hash_to_hex(old_hash)),
                                    new_hash: Some(hash_to_hex(new_hash)),
                                },
                            })
                            .collect(),
                    }
                }
                _ => unreachable!(),
            })
            .collect();

        JsonReport {
            added_regions: r.added_region_count(),
            removed_regions: r.removed_region_count(),
            changed_regions: r.changed_region_count(),
            changed_chunks: r.changed_chunk_count(),
            details: JsonDetails { added, removed, changed },
        }
    }
}

// ---------------------------------------------------------------------------
// HTML 可视化报告
// ---------------------------------------------------------------------------

/// HTML 报告的数据模型（序列化为 JSON 嵌入页面）。
#[derive(Serialize)]
struct HtmlReport {
    added_regions: usize,
    removed_regions: usize,
    changed_regions: usize,
    changed_chunks: usize,
    regions: Vec<HtmlRegion>,
}

#[derive(Serialize)]
struct HtmlRegion {
    /// region 类型："added" | "removed" | "changed"
    kind: &'static str,
    region_x: i32,
    region_z: i32,
    old_hash: Option<String>,
    new_hash: Option<String>,
    chunks: Vec<HtmlChunk>,
}

#[derive(Serialize)]
struct HtmlChunk {
    kind: &'static str,
    x: i64,
    z: i64,
    /// 区域内局部坐标（0..32）
    lx: i32,
    lz: i32,
    old_hash: Option<String>,
    new_hash: Option<String>,
}

impl From<&CompareResult> for HtmlReport {
    fn from(r: &CompareResult) -> Self {
        let mut regions = Vec::new();

        for rd in &r.added_regions {
            if let RegionDiff::Added { region_x, region_z, chunks } = rd {
                regions.push(HtmlRegion {
                    kind: "added",
                    region_x: *region_x,
                    region_z: *region_z,
                    old_hash: None,
                    new_hash: None,
                    chunks: chunks
                        .iter()
                        .map(|(x, z, h)| HtmlChunk {
                            kind: "added",
                            x: *x,
                            z: *z,
                            lx: floor_mod32(*x),
                            lz: floor_mod32(*z),
                            old_hash: None,
                            new_hash: Some(hash_to_hex(h)),
                        })
                        .collect(),
                });
            }
        }

        for rd in &r.removed_regions {
            if let RegionDiff::Removed { region_x, region_z, chunks } = rd {
                regions.push(HtmlRegion {
                    kind: "removed",
                    region_x: *region_x,
                    region_z: *region_z,
                    old_hash: None,
                    new_hash: None,
                    chunks: chunks
                        .iter()
                        .map(|(x, z, h)| HtmlChunk {
                            kind: "removed",
                            x: *x,
                            z: *z,
                            lx: floor_mod32(*x),
                            lz: floor_mod32(*z),
                            old_hash: Some(hash_to_hex(h)),
                            new_hash: None,
                        })
                        .collect(),
                });
            }
        }

        for rd in &r.changed_regions {
            if let RegionDiff::Changed { region_x, region_z, old_hash, new_hash, chunk_diffs } = rd {
                regions.push(HtmlRegion {
                    kind: "changed",
                    region_x: *region_x,
                    region_z: *region_z,
                    old_hash: Some(hash_to_hex(old_hash)),
                    new_hash: Some(hash_to_hex(new_hash)),
                    chunks: chunk_diffs
                        .iter()
                        .map(|d| match d {
                            ChunkDiff::Added { x, z, new_hash } => HtmlChunk {
                                kind: "added",
                                x: *x,
                                z: *z,
                                lx: floor_mod32(*x),
                                lz: floor_mod32(*z),
                                old_hash: None,
                                new_hash: Some(hash_to_hex(new_hash)),
                            },
                            ChunkDiff::Removed { x, z, old_hash } => HtmlChunk {
                                kind: "removed",
                                x: *x,
                                z: *z,
                                lx: floor_mod32(*x),
                                lz: floor_mod32(*z),
                                old_hash: Some(hash_to_hex(old_hash)),
                                new_hash: None,
                            },
                            ChunkDiff::Changed { x, z, old_hash, new_hash } => HtmlChunk {
                                kind: "changed",
                                x: *x,
                                z: *z,
                                lx: floor_mod32(*x),
                                lz: floor_mod32(*z),
                                old_hash: Some(hash_to_hex(old_hash)),
                                new_hash: Some(hash_to_hex(new_hash)),
                            },
                        })
                        .collect(),
                });
            }
        }

        HtmlReport {
            added_regions: r.added_region_count(),
            removed_regions: r.removed_region_count(),
            changed_regions: r.changed_region_count(),
            changed_chunks: r.changed_chunk_count(),
            regions,
        }
    }
}

/// Java 风格的 floorMod(32)：保证结果在 0..32（负坐标也正确）。
fn floor_mod32(v: i64) -> i32 {
    let m = v % 32;
    if m < 0 { (m + 32) as i32 } else { m as i32 }
}

/// 自包含 HTML 模板，内嵌数据 JSON 与渲染 JS。
fn html_template(data_json: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>mc-chunk-hash 区块比对报告</title>
<style>
  :root {{
    --added: #2ecc71; --removed: #e74c3c; --changed: #f1c40f;
    --bg: #1e1e2e; --card: #2a2a40; --text: #e0e0e0; --muted: #888;
  }}
  * {{ box-sizing: border-box; }}
  body {{ margin: 0; font-family: -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
         background: var(--bg); color: var(--text); padding: 24px; }}
  h1 {{ font-size: 22px; margin: 0 0 4px; }}
  .sub {{ color: var(--muted); font-size: 13px; margin-bottom: 16px; }}
  .cards {{ display: flex; gap: 16px; flex-wrap: wrap; margin-bottom: 16px; }}
  .card {{ background: var(--card); border-radius: 10px; padding: 16px 20px; min-width: 140px; }}
  .card .num {{ font-size: 32px; font-weight: 700; }}
  .card .label {{ color: var(--muted); font-size: 13px; }}
  .card.added .num {{ color: var(--added); }}
  .card.removed .num {{ color: var(--removed); }}
  .card.changed .num {{ color: var(--changed); }}

  /* 视图切换 */
  .viewbar {{ display: flex; gap: 8px; margin-bottom: 16px; }}
  .viewbtn {{ background: #3a3a55; border: none; color: var(--text); padding: 8px 16px;
              border-radius: 8px; cursor: pointer; font-size: 13px; }}
  .viewbtn.active {{ background: #6c5ce7; color: #fff; }}

  .region {{ background: var(--card); border-radius: 10px; padding: 16px; margin-bottom: 16px; }}
  .region h2 {{ font-size: 16px; margin: 0 0 10px; display:flex; align-items:center; gap:8px; }}
  .badge {{ font-size: 11px; padding: 2px 8px; border-radius: 20px; color:#111; font-weight:600; }}
  .badge.added {{ background: var(--added); }}
  .badge.removed {{ background: var(--removed); }}
  .badge.changed {{ background: var(--changed); }}
  .grid {{ display: grid; grid-template-columns: repeat(32, 14px);
          grid-template-rows: repeat(32, 14px); gap: 1px; width: fit-content; }}
  .cell {{ width: 14px; height: 14px; border-radius: 2px; background: #3a3a55;
           cursor: default; position: relative; }}
  .cell.added {{ background: var(--added); }}
  .cell.removed {{ background: var(--removed); }}
  .cell.changed {{ background: var(--changed); }}
  .cell:hover {{ outline: 2px solid #fff; z-index: 10; }}
  .legend {{ display: flex; gap: 16px; margin: 12px 0; font-size: 12px; color: var(--muted); }}
  .legend span {{ display: inline-flex; align-items: center; gap: 5px; }}
  .swatch {{ width: 12px; height: 12px; border-radius: 3px; display: inline-block; }}
  .detail {{ margin-top: 12px; font-size: 12px; color: var(--muted); display:none; }}
  .detail table {{ border-collapse: collapse; width: 100%; max-width: 800px; }}
  .detail th, .detail td {{ text-align: left; padding: 4px 8px; border-bottom: 1px solid #3a3a55; }}
  .detail .mono {{ font-family: ui-monospace, Consolas, monospace; font-size: 11px; word-break: break-all; }}
  .toggle {{ background: #3a3a55; border: none; color: var(--text); padding: 4px 12px;
            border-radius: 6px; cursor: pointer; font-size: 12px; margin-top: 8px; }}
  .toggle:hover {{ background: #4a4a66; }}
  .empty {{ color: var(--muted); padding: 30px; text-align:center; }}

  /* 世界地图 */
  #mapwrap {{ position: relative; background: var(--card); border-radius: 10px; overflow: hidden; }}
  #map {{ display: block; cursor: grab; }}
  #map.dragging {{ cursor: grabbing; }}
  #tooltip {{ position: fixed; background: #0d0d1a; border: 1px solid #444; border-radius: 6px;
              padding: 6px 10px; font-size: 12px; pointer-events: none; display: none; z-index: 1000;
              max-width: 320px; }}
  #tooltip .mono {{ font-family: ui-monospace, Consolas, monospace; font-size: 11px; }}
  .hint {{ color: var(--muted); font-size: 12px; margin: 8px 0; }}
</style>
</head>
<body>
<h1>mc-chunk-hash 区块比对报告</h1>
<div class="sub">chunk 网格：每个小格代表一个 chunk，颜色表变化类型</div>

<div class="viewbar">
  <button class="viewbtn active" id="btn-regions" onclick="switchView('regions')">region 列表</button>
  <button class="viewbtn" id="btn-map" onclick="switchView('map')">世界大地图</button>
</div>

<div id="summary" class="cards"></div>

<div class="legend">
  <span><i class="swatch" style="background:#2ecc71"></i> 新增 chunk</span>
  <span><i class="swatch" style="background:#e74c3c"></i> 删除 chunk</span>
  <span><i class="swatch" style="background:#f1c40f"></i> 变更 chunk</span>
  <span><i class="swatch" style="background:#3a3a55"></i> 未变 chunk</span>
  <span><i class="swatch" style="background:#6c5ce7"></i> 整个 region 增/删</span>
</div>

<div id="view-regions">
  <div id="regions"></div>
</div>
<div id="view-map" style="display:none;">
  <div class="hint">滚轮缩放 · 拖拽平移 · 悬停查看 chunk 详情</div>
  <div id="mapwrap" style="width:100%; height:70vh; position:relative;">
    <canvas id="map"></canvas>
  </div>
</div>

<div id="tooltip"></div>

<script>
const DATA = {data_json};

function esc(s) {{ return String(s).replace(/[&<>"]/g, c => ({{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}}[c])); }}

function switchView(v) {{
  document.getElementById('view-regions').style.display = v === 'regions' ? 'block' : 'none';
  document.getElementById('view-map').style.display = v === 'map' ? 'block' : 'none';
  document.getElementById('btn-regions').classList.toggle('active', v === 'regions');
  document.getElementById('btn-map').classList.toggle('active', v === 'map');
  if (v === 'map') {{ initMap(); }}
}}

function renderSummary(d) {{
  const cards = [
    {{cls:'added', num:d.added_regions, label:'新增 region'}},
    {{cls:'removed', num:d.removed_regions, label:'删除 region'}},
    {{cls:'changed', num:d.changed_regions, label:'变更 region'}},
    {{cls:'changed', num:d.changed_chunks, label:'变更 chunk 总数'}},
  ];
  document.getElementById('summary').innerHTML = cards.map(c =>
    `<div class="card ${{c.cls}}"><div class="num">${{c.num}}</div><div class="label">${{c.label}}</div></div>`
  ).join('');
}}

function renderRegion(r) {{
  let cells = new Array(1024).fill(null).map(() => '');
  const map = {{}};
  r.chunks.forEach(c => {{ map[c.lz * 32 + c.lx] = c; }});

  let gridHtml = '<div class="grid">';
  for (let i = 0; i < 1024; i++) {{
    const c = map[i];
    let cls = 'cell';
    let title = '';
    if (c) {{
      cls += ' ' + c.kind;
      const world = `chunk (${{c.x}}, ${{c.z}})`;
      const oh = c.old_hash ? '\\nold: ' + c.old_hash : '';
      const nh = c.new_hash ? '\\nnew: ' + c.new_hash : '';
      title = `title="${{esc(world + oh + nh)}}"`;
    }}
    gridHtml += `<div class="${{cls}}" ${{title}}></div>`;
  }}
  gridHtml += '</div>';

  let detailHtml = '<table><tr><th>世界坐标</th><th>类型</th><th>旧哈希</th><th>新哈希</th></tr>';
  r.chunks.forEach(c => {{
    const kindLabel = c.kind === 'added' ? '新增' : c.kind === 'removed' ? '删除' : '变更';
    detailHtml += `<tr><td>(${{c.x}}, ${{c.z}})</td><td>${{kindLabel}}</td>` +
      `<td class="mono">${{c.old_hash || '—'}}</td><td class="mono">${{c.new_hash || '—'}}</td></tr>`;
  }});
  detailHtml += '</table>';

  const badge = `<span class="badge ${{r.kind}}">${{r.kind === 'added' ? '新增' : r.kind === 'removed' ? '删除' : '变更'}}</span>`;
  const hashInfo = r.old_hash && r.new_hash
    ? `<span style="font-size:11px;color:#888">${{r.old_hash.slice(0,12)}}… → ${{r.new_hash.slice(0,12)}}…</span>` : '';

  return `<div class="region">
    <h2>region (${{r.region_x}}, ${{r.region_z}}) ${{badge}} ${{hashInfo}}</h2>
    ${{gridHtml}}
    <button class="toggle" onclick="toggleDetail(this)">展开 chunk 明细 (${{r.chunks.length}})</button>
    <div class="detail">${{detailHtml}}</div>
  </div>`;
}}

function toggleDetail(btn) {{
  const d = btn.nextElementSibling;
  if (d.style.display === 'block') {{
    d.style.display = 'none'; btn.textContent = btn.textContent.replace('收起', '展开');
  }} else {{
    d.style.display = 'block'; btn.textContent = btn.textContent.replace('展开', '收起');
  }}
}}

// ---------------- 世界大地图 ----------------
const COLORS = {{ added: '#2ecc71', removed: '#e74c3c', changed: '#f1c40f' }};
let mapState = {{ scale: 1, ox: 0, oy: 0, chunks: [], minX: 0, maxX: 0, minZ: 0, maxZ: 0, inited: false }};

function collectWorldChunks() {{
  const byCoord = new Map();
  // 先用 region 级信息：新增/删除的 region，其所有 chunk 标记为该类型
  DATA.regions.forEach(r => {{
    if (r.kind === 'added' || r.kind === 'removed') {{
      r.chunks.forEach(c => byCoord.set(c.x + ',' + c.z, {{
        x: c.x, z: c.z, kind: r.kind, regionX: r.region_x, regionZ: r.region_z,
        old_hash: c.old_hash, new_hash: c.new_hash
      }}));
    }}
  }});
  // chunk 级信息覆盖（changed region 内的 chunk）
  DATA.regions.forEach(r => {{
    if (r.kind === 'changed') {{
      r.chunks.forEach(c => byCoord.set(c.x + ',' + c.z, {{
        x: c.x, z: c.z, kind: c.kind, regionX: r.region_x, regionZ: r.region_z,
        old_hash: c.old_hash, new_hash: c.new_hash
      }}));
    }}
  }});
  return Array.from(byCoord.values());
}}

function initMap() {{
  if (mapState.inited) return;
  mapState.inited = true;
  mapState.chunks = collectWorldChunks();

  if (mapState.chunks.length === 0) {{
    document.getElementById('map').style.display = 'none';
    return;
  }}

  let xs = mapState.chunks.map(c => c.x), zs = mapState.chunks.map(c => c.z);
  mapState.minX = Math.min(...xs); mapState.maxX = Math.max(...xs);
  mapState.minZ = Math.min(...zs); mapState.maxZ = Math.max(...zs);

  const canvas = document.getElementById('map');
  const wrap = document.getElementById('mapwrap');
  const w = wrap.clientWidth, h = wrap.clientHeight;

  // 初始缩放：让整个世界范围自适应画布，每个 chunk 至少 1px
  const worldW = mapState.maxX - mapState.minX + 1;
  const worldH = mapState.maxZ - mapState.minZ + 1;
  mapState.scale = Math.max(1, Math.min(w / worldW, h / worldH));
  mapState.ox = (w - worldW * mapState.scale) / 2 - mapState.minX * mapState.scale;
  mapState.oy = (h - worldH * mapState.scale) / 2 - mapState.minZ * mapState.scale;

  canvas.width = w; canvas.height = h;
  drawMap();

  // 交互：滚轮缩放 + 拖拽平移
  canvas.addEventListener('wheel', ev => {{
    ev.preventDefault();
    const factor = ev.deltaY < 0 ? 1.2 : 1/1.2;
    const oldScale = mapState.scale;
    mapState.scale = Math.max(1, Math.min(60, mapState.scale * factor));
    const mx = ev.offsetX, my = ev.offsetY;
    mapState.ox = mx - (mx - mapState.ox) * (mapState.scale / oldScale);
    mapState.oy = my - (my - mapState.oy) * (mapState.scale / oldScale);
    drawMap();
  }});

  let dragging = false, lastX = 0, lastY = 0;
  canvas.addEventListener('mousedown', ev => {{
    dragging = true; lastX = ev.clientX; lastY = ev.clientY;
    canvas.classList.add('dragging');
  }});
  window.addEventListener('mousemove', ev => {{
    if (!dragging) {{ hoverChunk(ev); return; }}
    mapState.ox += ev.clientX - lastX;
    mapState.oy += ev.clientY - lastY;
    lastX = ev.clientX; lastY = ev.clientY;
    drawMap();
  }});
  window.addEventListener('mouseup', () => {{ dragging = false; canvas.classList.remove('dragging'); }});
  canvas.addEventListener('mouseleave', () => {{ document.getElementById('tooltip').style.display = 'none'; }});
}}

function drawMap() {{
  const canvas = document.getElementById('map');
  const ctx = canvas.getContext('2d');
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  // 背景
  ctx.fillStyle = '#1e1e2e';
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  const s = mapState.scale;
  const cell = Math.max(1, s);
  const drawGap = cell > 3;

  for (const c of mapState.chunks) {{
    const px = c.x * s + mapState.ox;
    const py = c.z * s + mapState.oy;
    if (px < -cell || py < -cell || px > canvas.width || py > canvas.height) continue;
    ctx.fillStyle = COLORS[c.kind] || '#888';
    ctx.fillRect(px, py, drawGap ? cell - 1 : cell, drawGap ? cell - 1 : cell);
  }}

  // region 边界虚线（在较大缩放时显示）
  if (s >= 4) {{
    ctx.strokeStyle = 'rgba(120,120,180,0.35)';
    ctx.lineWidth = 1;
    for (let gx = mapState.minX; gx <= mapState.maxX; gx++) {{
      if (gx % 32 === 0) {{
        const x = gx * s + mapState.ox;
        ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, canvas.height); ctx.stroke();
      }}
    }}
    for (let gz = mapState.minZ; gz <= mapState.maxZ; gz++) {{
      if (gz % 32 === 0) {{
        const y = gz * s + mapState.oy;
        ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(canvas.width, y); ctx.stroke();
      }}
    }}
  }}
}}

function hoverChunk(ev) {{
  const canvas = document.getElementById('map');
  const rect = canvas.getBoundingClientRect();
  const mx = ev.clientX - rect.left, my = ev.clientY - rect.top;
  const s = mapState.scale;
  const cx = Math.floor((mx - mapState.ox) / s);
  const cz = Math.floor((my - mapState.oy) / s);
  const key = cx + ',' + cz;

  // 在 chunks 里找（小数据集直接线性查找，够用）
  const found = mapState.chunks.find(c => c.x === cx && c.z === cz);
  const tip = document.getElementById('tooltip');
  if (found && s >= 2) {{
    const kindLabel = found.kind === 'added' ? '新增' : found.kind === 'removed' ? '删除' : '变更';
    tip.innerHTML =
      `<div style="font-weight:600;color:${{COLORS[found.kind]}}">${{kindLabel}} chunk (${{found.x}}, ${{found.z}})</div>` +
      `<div style="color:#888">region (${{found.regionX}}, ${{found.regionZ}})</div>` +
      (found.old_hash ? `<div class="mono">old: ${{found.old_hash}}</div>` : '') +
      (found.new_hash ? `<div class="mono">new: ${{found.new_hash}}</div>` : '');
    tip.style.display = 'block';
    tip.style.left = (ev.clientX + 12) + 'px';
    tip.style.top = (ev.clientY + 12) + 'px';
  }} else {{
    tip.style.display = 'none';
  }}
}}

function render() {{
  renderSummary(DATA);
  if (DATA.regions.length === 0) {{
    document.getElementById('regions').innerHTML = '<div class="empty">无差异。</div>';
    return;
  }}
  document.getElementById('regions').innerHTML = DATA.regions.map(renderRegion).join('');
}}

render();
</script>
</body>
</html>"#,
        data_json = data_json
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::CompareResult;

    fn sample_result() -> CompareResult {
        CompareResult {
            added_regions: vec![],
            removed_regions: vec![],
            changed_regions: vec![RegionDiff::Changed {
                region_x: 0,
                region_z: 0,
                old_hash: [1u8; 16],
                new_hash: [2u8; 16],
                chunk_diffs: vec![ChunkDiff::Changed {
                    x: 3,
                    z: 5,
                    old_hash: [3u8; 16],
                    new_hash: [4u8; 16],
                }],
            }],
        }
    }

    #[test]
    fn html_contains_data_json() {
        let html = format_html(&sample_result());
        assert!(html.contains("const DATA ="));
        assert!(html.contains("\"changed_regions\":1"));
        assert!(html.contains("<div class=\"grid\">"));
        // 局部坐标正确：chunk (3,5) 对应 lx=3, lz=5
        assert!(html.contains("\"lx\":3"));
        assert!(html.contains("\"lz\":5"));
    }

    #[test]
    fn html_empty_reports_no_diff() {
        let empty = CompareResult::default();
        let html = format_html(&empty);
        assert!(html.contains("\"regions\":[]"));
    }

    #[test]
    fn floor_mod_handles_negative() {
        assert_eq!(floor_mod32(0), 0);
        assert_eq!(floor_mod32(31), 31);
        assert_eq!(floor_mod32(32), 0);
        assert_eq!(floor_mod32(-1), 31);
        assert_eq!(floor_mod32(-33), 31);
    }
}
