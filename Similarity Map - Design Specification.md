# Similarity Map — Design Specification

**Project:** Romance Factory  
**Component:** Manuscript Repetition Visualizer  
**Status:** Draft  

---

## 1. Overview

The Similarity Map is an interactive visual tool for detecting and exploring repeated content within a long-form manuscript. It renders a portrait-oriented page grid where each cell represents one page of the document. The macro-grid is arranged **10 columns wide × up to 30 rows tall** so its shape mirrors the page count and aspect of a real book. Each page cell is a **20×20 pixel canvas** encoding a **20×20 sub-grid** that shows the approximate location within the page where each cluster appears — 1 pixel per sub-cell, 400 sub-cells per page — so the reader sees not only which pages contain repetition but where on those pages it lives.

The goal is to give an author or editor a single-glance "repetition fingerprint" of an entire book — with the ability to drill into any page and see exactly what is being repeated, how often, how closely, and where on the page.

---

## 2. Goals

- Detect **exact** and **fuzzy** (paraphrase-level) phrase repetition across a manuscript
- Surface repetition at both **page** and **intra-page** spatial granularity
- Allow real-time exploration via **threshold** and **phrase-length** sliders
- Produce **stable cluster identities** for use in editorial scoring pipelines
- Integrate cleanly into the Romance Factory agent ecosystem

### Non-Goals

- Sentence-level grammar or style analysis
- Cross-document comparison (single manuscript only, for now)
- Real-time streaming as the author writes

---

## 3. Core Concepts

### 3.0 Import and Pagination

Before any analysis begins, the raw document is converted into a sequence of **pages**. How pagination works depends on the import format:

**PDF / structured document** — natural page breaks are preserved. Each PDF page becomes one grid cell. The page boundary is exact.

**Plain-text blob** — no natural page breaks exist. The text is split into artificial pages using a configurable **Tokens per Page** setting. The import stage tokenizes the full text by whitespace splitting (`text.split_whitespace()`), then slices into chunks of the target size:

```
tokens_per_page = 400   (user-configurable, e.g. 200–2000)

page 1: tokens[0    .. 399 ]
page 2: tokens[400  .. 799 ]
page 3: tokens[800  .. 1199]
...
```

Each chunk is converted back to its character span so `char_start` / `char_end` fields remain character-accurate. The last page may be shorter than `tokens_per_page`.

**Constraint:** `tokens_per_page` must be significantly larger than the phrase window size. A reasonable minimum is `tokens_per_page ≥ 4 × window_size` so that each page contains enough windows for the sub-grid to be meaningfully populated. The UI enforces this with a soft warning when the ratio falls below 4×.

**Chapter / scene breaks (optional):** When the text contains explicit break markers (blank lines between scenes, `---` dividers, chapter headings matched by a configurable regex), the importer can use those as page boundaries instead of token count, with a maximum page size cap applied to oversized chapters.

### Import Settings Panel

Before analysis begins the user sees an **Import Settings** panel with all parameters that affect total processing time. The panel displays a **live window-count estimate** and **estimated embedding time** as settings are adjusted, so the user can tune stride and phrase length before committing to a potentially long run.

```
┌─────────────────────────────────────────────────────────┐
│  Import Settings                           [Analyze] [✕]│
├─────────────────────────────────────────────────────────┤
│  Tokens per page    [────●──────────]  400 tokens        │
│  Phrase length      [──●────────────]   20 tokens        │
│  Stride             [─────●──────────]   5 words         │
│  Min repetitions    [──●────────────]    3 occurrences   │
│  Min samples        [──●────────────]    3               │
│  Chapter break      [^Chapter\s+\d+              ]  ✎   │
│                                                          │
│  Estimated windows:  ~24,000                             │
│  Estimated time:     ~4 min  (CPU · MiniLM-L6-v2)       │
│                                                          │
│  ⚠  Stride is 25% of phrase length — good coverage      │
└─────────────────────────────────────────────────────────┘
```

The time estimate is derived from a **benchmark sample**: on first launch the backend embeds a fixed 128-window probe and records throughput (windows/sec). Subsequent estimates multiply that rate by the projected window count. The benchmark result is cached in app data and updated whenever the model changes.

---

### 3.1 Phrase Window

The document is split into overlapping text windows of configurable token length. A window is the atomic unit of comparison. Each window carries its **character offset** within its page so it can be positioned in the sub-grid.

```
page 12: "the moonlight fell across her face like a wound"
  window A: { text: "the moonlight fell across her",  char_start: 0,  char_end: 29  }
  window B: { text: "moonlight fell across her face",  char_start: 4,  char_end: 33  }
  window C: { text: "fell across her face like a",     char_start: 14, char_end: 40  }
```

Windows slide across pages with a fixed stride. Smaller windows catch micro-phrase repetition; larger windows catch thematic or structural repetition.

**Windows do not cross page boundaries.** If the remaining text on a page is shorter than one window, it is included as a shorter terminal window (or skipped if below a minimum token floor). This ensures sub-cell positions always resolve within a single page.

### 3.2 Embedding

Each window is converted to a dense vector via a language embedding model. Semantically similar phrases produce vectors that are close together in the embedding space (measured by cosine similarity).

### 3.3 Similarity Score

For any two windows A and B:

```
similarity = (A · B) / (‖A‖ · ‖B‖)     [cosine similarity, range 0.0–1.0]
```

A similarity of 1.0 is an exact duplicate. 0.95+ is near-identical phrasing. 0.80–0.95 is paraphrase or strong thematic echo.

### 3.4 Repetition Density

Each page is assigned a **repetition density score** — the maximum similarity found among all of its windows against any window elsewhere in the document.

### 3.5 Cluster

Groups of windows with mutual high similarity form a **cluster** — a recurring phrase, motif, or structural pattern. Clusters are given identity labels that persist across the visualization.

### 3.6 Sub-Cell

Each page cell in the grid is subdivided into a **20×20 sub-grid** of 400 sub-cells. Each sub-cell represents approximately 0.25% of a page's text, distributed in reading order (top-left = beginning of page, bottom-right = end).

A window is mapped to a sub-cell by its normalized position within the page:

```
sub_cell_index = floor(window_char_midpoint / page_char_count × 400)

row = floor(sub_cell_index / 20)      [0–19, top to bottom]
col =       sub_cell_index mod 20     [0–19, left to right]
```

Because each page cell is rendered as a **20×20 px canvas**, each sub-cell maps to exactly **one pixel**. Multiple clusters occupying the same sub-cell cannot be spatially dithered at this scale; instead they are rendered using **weighted color blending** — see Section 3.7.

Sub-cells with no windows, or all windows below the similarity threshold, render as transparent (background shows through).

### 3.7 Sub-Cell Color Blending

Because each sub-cell is exactly one pixel at the standard 20×20 px canvas size, spatial dithering (checker, quadrant, scatter patterns) cannot be applied — there is only one pixel to assign. Instead, multiple clusters within a sub-cell are resolved using **weighted color blending in linear RGB space**.

**Single cluster — direct color**

The pixel is set to the cluster's HSV color, converted to sRGB:
```
pixel = hsv_to_rgb(cluster_hue, sat(sim), val(page_density))
```

**Multiple clusters — similarity-weighted blend**

Clusters are sorted by `sim_to_centroid` descending and capped at **8**. Their linear-RGB colors are blended with weights proportional to their similarity scores:

```
weight[i] = sim_to_centroid[i]^γ         // same gamma as saturation curve
total_weight = Σ weight[i]
color = Σ (weight[i] / total_weight × linear_rgb(cluster[i]))
pixel = linear_to_srgb(color)
```

This means the dominant cluster (highest similarity) contributes most to the pixel color, but secondary clusters produce a visible hue shift. At macro-grid viewing distance the pixel reads as the dominant color; at zoom the hue blend reveals the overlap.

**Zoom / detail view — dithering re-enabled**

At higher zoom levels (when the grid renderer scales page cells up to 100×200 px or larger), the rasterizer can optionally switch to **spatial dithering** for sub-cells that contain multiple clusters, using the patterns below. This is computed on-demand in the frontend from the stored sub-cell cluster list — no re-IPC required.

| Cluster count | Pattern |
|---|---|
| 1 | Solid fill |
| 2 | 50/50 checkerboard — `(px_row + px_col) mod 2` |
| 3 | Thirds scatter — `(px_row * 3 + px_col * 7) mod 3` |
| 4 | 2×2 quadrant tile — `(px_row mod 2) * 2 + (px_col mod 2)` |
| 5–8 | Modulo scatter — `(px_row * 11 + px_col * 7) mod N` |

Clusters are sorted by `sim_to_centroid` descending before slot assignment. Maximum clusters per sub-cell: **8** (top 8 by similarity, others discarded).

---

## 4. Processing Pipeline

```
┌──────────────┐     ┌─────────────────────┐     ┌─────────────────────┐     ┌──────────────────┐
│  Document    │────▶│  Import / Paginator  │────▶│  Text Window Engine  │────▶│ Embedding Engine │
│  (PDF/text   │     │  PDF → natural pages │     │  (configurable size, │     │ (ONNX / remote)  │
│   /txt blob) │     │  text → token chunks │     │   sliding stride)    │     │                  │
└──────────────┘     └─────────────────────┘     └─────────────────────┘     └────────┬─────────┘
                                                           │
                                                           ▼
                                              ┌────────────────────────┐
                                              │   LanceDB Vector Store  │
                                              │   page · char_offset    │
                                              │   · window · vec        │
                                              └────────────┬───────────┘
                                                           │
                                              ┌────────────▼───────────┐
                                              │   HDBSCAN Clustering   │
                                              │   (density-based,      │
                                              │    no fixed k)         │
                                              └────────────┬───────────┘
                                                           │
                                              ┌────────────▼───────────┐
                                              │   KMeans Stabilizer    │
                                              │   (on HDBSCAN-filtered  │
                                              │    subset only)         │
                                              └────────────┬───────────┘
                                                           │
                                              ┌────────────▼───────────┐
                                              │   Sub-Cell Mapper      │
                                              │   window → (row, col)  │
                                              │   on its page's 20×20  │
                                              └────────────┬───────────┘
                                                           │
                                              ┌────────────▼───────────┐
                                              │   HSV Color Mapper     │
                                              │   cluster → hue        │
                                              │   similarity → S, V    │
                                              └────────────┬───────────┘
                                                           │
                                              ┌────────────▼───────────┐
                                              │   Page Canvas Raster   │
                                              │   20×20 px RGBA per    │
                                              │   page (pre-rendered   │
                                              │   in Rust)             │
                                              └────────────┬───────────┘
                                                           │
                                              ┌────────────▼───────────┐
                                              │   Grid Renderer (UI)   │
                                              │   composites canvases, │
                                              │   CSS scales on resize │
                                              └────────────────────────┘
```

### Stage Details

| Stage | Input | Output | Notes |
|---|---|---|---|
| Import / Paginator | Raw file (PDF or text blob) | `Vec<Page { page_num, text, char_offset_in_doc }>` | PDF: preserves natural breaks. Text: slices at `tokens_per_page` boundary; optional chapter-break detection |
| Text Window Engine | Page text + page boundaries | `{ page, char_start, char_end, text }` per window | Stride and window size are slider-driven; windows do not cross page boundaries |
| Embedding Engine | Window text | `Vec<f32>` per window | ONNX local or SentenceTransformers via FFI |
| LanceDB Store | Embeddings + metadata | Queryable vector table | Persists between sessions |
| HDBSCAN | All embeddings | Cluster labels + noise flags | No fixed k; ignores noise naturally |
| KMeans Stabilizer | HDBSCAN-filtered subset | Stable cluster IDs (integers) | Only clusters with ≥ 3 members |
| Sub-Cell Mapper | Cluster assignments + char offsets | `(page, row, col) → [SubCellCluster]` | Populates 20×20 per-page grids; stores all clusters per sub-cell |
| HSV Color Mapper | Cluster registry | `cluster_id → (H, S, V)` lookup table | Computed once; reused by rasterizer |
| Page Canvas Raster | Sub-cell grids + color lookup + threshold | `[u8; 20×20×4]` per page | Applies similarity-weighted color blend per pixel; shipped to frontend once |

---

## 5. Clustering Strategy

Clustering uses a **two-stage pipeline** to get the best of both approaches:

### Stage 1 — HDBSCAN (organic detection)
- Finds natural clusters of any shape and size
- Automatically marks one-off phrases as noise (label `-1`)
- No need to guess the number of clusters
- Produces clean, meaningful groups of genuinely repeated content

**HDBSCAN parameters (user-configurable in Import Settings):**

| Parameter | What it controls | Default (300-page manuscript) |
|---|---|---|
| **Min repetitions** | How many times a phrase must recur across the document to count as a cluster. Converted to `min_cluster_size` internally: `min_reps × max(1, floor(phrase_length / stride))` | **3** occurrences |
| **Min samples** | Noise sensitivity — higher values are stricter, filtering more marginal windows to noise. HDBSCAN's `min_samples` parameter directly. | **3** |

Exposing **Min repetitions** rather than the raw `min_cluster_size` makes the setting intuitive regardless of stride: "a phrase must appear at least 3 times across the manuscript to be flagged." The backend computes the HDBSCAN parameter from the phrase/stride ratio automatically.

**Scaling guidance:**

| Document size | Recommended Min repetitions |
|---|---|
| < 100 pages | 2 — catch anything that appears more than once |
| 100–400 pages | 3 — default; filters incidental similarity |
| 400–800 pages | 3–5 — larger corpus means more coincidental near-matches |
| 800+ pages | 5–8 — aggressive filtering to surface only true structural repetition |

### Stage 2 — KMeans (stable labeling only)
- Runs on the HDBSCAN-filtered subset with **k = number of clusters HDBSCAN found** — no reduction, no merging
- Purely re-labels HDBSCAN's organic clusters with stable integer IDs so that hue assignments are deterministic between runs
- Does **not** collapse or combine clusters; if HDBSCAN finds 100 distinct repeated phrases, KMeans produces 100 stable labels and all 100 are shown
- A manuscript with many distinct repeated clusters (e.g., 80–100 in 300 pages) is itself a diagnostic signal — it means the text is a mosaic of recycled material. This is intentional and should be fully visible: the grid will be dense with different hues, which reads immediately as "this document has many different recurring elements." Suppressing that would hide the problem.

### Stage 3 — Centroid computation
After KMeans assigns stable IDs, each cluster's **centroid** is computed as the mean of its member embeddings:

```
centroid[k] = mean({ embedding[w] : cluster_id[w] == k })
```

The centroid is a synthetic 384-dimensional vector — it may not correspond to any actual window in the document. It is stored in the `ClusterRegistry` and used as the reference point for all `sim_to_centroid` scores.

The **most central window** — `argmax_w cosine_similarity(embedding[w], centroid[k])` — is identified for each cluster and stored as `most_central_window_id`. This is the excerpt shown to the user in tooltips and the detail panel as the canonical example of that cluster.

**Why both:** HDBSCAN's intelligence + KMeans' stability + centroid accuracy. HDBSCAN finds the real patterns; KMeans gives them stable names; the centroid gives each cluster a single, maximally representative point that is more stable than any individual window and more meaningful than the first-seen example.

### Detection Scale and Phrase Length

The phrase length setting determines what *scale* of repetition the system resolves into a single cluster. The same repeated content looks different at different phrase lengths:

| Phrase length | What a repeated 1.5-page block looks like |
|---|---|
| 5–20 tokens | Many small overlapping clusters — individual sentences and phrases detected separately; noisy but granular |
| 20–100 tokens | Several clusters — paragraph-sized chunks of the block grouped together; the repetition is visible but fragmented |
| **100–500 tokens** | **One or two dominant clusters spanning the full block** — the most legible representation of large-block repetition |
| 500–1500 tokens | One cluster, very high similarity, but short passages and variations may be subsumed or missed |

**For large-block repetition (paragraph runs, scene beats, near-duplicate pages), 100–500 tokens is the recommended phrase length.** The system will produce a single cluster whose color appears as a consistent band across the same sub-cell region on every page where the block occurs. Because the repeated block is ~600 tokens at 400 tokens/page, it spans page boundaries — the band appears in the lower sub-cells of the starting page and the upper sub-cells of the following page, consistently across all instances.

**Multi-pass approach:** Running analysis twice — once at 20 tokens (phrase level) and once at 200 tokens (paragraph level) — reveals both the large structural repetition and the finer phrase echoes within it. Each run produces an independent map; the user can switch between them. This is supported naturally by the session persistence model (each run is a separate job with its own settings hash).

---

## 6. Color Encoding (HSV)

The grid uses the HSV color model because it separates **identity** (what cluster) from **intensity** (how similar) cleanly.

### Hue → Cluster Identity

Each cluster is assigned a unique hue using the golden-ratio conjugate distribution:

```
hue = (cluster_id × 0.6180339887) mod 1.0
```

This spreads clusters evenly around the color wheel, preventing neighboring clusters from having similar colors. Noise points (no cluster) render transparent. Sub-cells with no windows render as the background color.

### Saturation → Constant (1.0)

Saturation is fixed at **1.0** for all clustered windows. The hue alone carries cluster identity; keeping saturation constant ensures every cluster reads as a pure, unambiguous color at any zoom level. Noise windows (no cluster) render transparent regardless.

### Value → Proximity to Cluster Centroid

Value encodes how archetypal this particular window is — how close it sits to the cluster centroid in embedding space:

```
V = cosine_similarity(window_embedding, cluster_centroid)^γ
```

`γ` (gamma) is a tunable contrast parameter (default: `1.5`).

- **V = 1.0** — the most central window (exact centroid match or the nearest member to it): maximum brightness, fully vivid color
- **V near 1.0** — near-identical phrasing: bright
- **V lower** — a weaker echo, more distant paraphrase: darker shade of the same hue
- **Exact 1:1 duplicate** — cosine similarity of 1.0 → V = 1.0, same as the centroid

This means brightness directly encodes typicality: the canonical instances of a repeated phrase are the brightest pixels on the map, and vaguer echoes are darker. No `density_ceiling` parameter is needed — the scale is self-normalizing, anchored to the centroid.

This creates a **heatmap within a heatmap within a heatmap**: the macro-grid shows which pages have repetition, the sub-grid shows where on those pages, and the value channel shows how archetypal each instance is.

---

## 7. UI — Grid Visualization

### 7.1 Layout

The grid is a portrait-oriented arrangement of page cells, mirroring the shape of a real book:

- Fixed **10 columns wide**, height grows with page count (up to ~30 rows for a ~300-page novel)
- ~200-page manuscript → 10 × 20 macro-grid → **200 px wide, 400 px tall** at base scale
- ~300-page manuscript → 10 × 30 macro-grid → **200 px wide, 600 px tall** at base scale
- Each macro-cell = one page, rendered as a **20×20 px canvas**
- Each canvas encodes a **20×20 sub-grid** — **one pixel per sub-cell**
- Sub-cells are colored by the dominant cluster (or a similarity-weighted blend for overlapping clusters); empty sub-cells are transparent
- The browser scales the entire grid via CSS — `image-rendering: pixelated` keeps sub-cell pixels crisp at any zoom; detail view switches to `image-rendering: auto` (bilinear) at high magnification
- A configurable cell gap (default 1 px, same color as background) separates page cells so the grid reads as a book of pages

```
Macro-grid (excerpt, 10 cols × 4 rows shown):

  page 1    page 2    page 3    page 4    page 5  ...  page 10
 ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐   ┌────────┐
 │ 20×20  │ │ 20×20  │ │ 20×20  │ │ 20×20  │ │ 20×20  │...│ 20×20  │  row 0
 │ pixels │ │ pixels │ │ pixels │ │ pixels │ │ pixels │   │ pixels │
 └────────┘ └────────┘ └────────┘ └────────┘ └────────┘   └────────┘
 ┌────────┐   ...
 │ 20×20  │                                                            row 1
 │ pixels │
 └────────┘
  ...
                                                                       row 29

Single page cell (20×20 px = 20×20 sub-grid, 1 px/sub-cell):

 ┌────────────────────┐
 │· · ■ · · · · · · ·│  row 0   (top of page text)
 │· · ■ · · · ▪ · · ·│  row 1
 │· · · · · · ▪ · · ·│  row 2
 │· · · · · · · · · ·│  row 3   (no clusters)
 │· ◆ · · · · · · · ·│  row 4   (two-cluster blend pixel)
 │· · · · · · · · · ·│  ...
 │...                 │
 └────────────────────┘
  col 0            col 19

 ■ = cluster 2 (orange, high sim) — solid pixel
 ▪ = cluster 5 (cyan, medium sim) — solid pixel
 ◆ = clusters 2 + 7 overlapping — similarity-weighted color blend
     (reads as orange-leaning mix at standard zoom; dither applied at zoom)
```

### 7.2 Controls

Controls are grouped into two tiers: **Import settings** (require a full re-analysis when changed) and **Display settings** (update the visualization instantly or with a lightweight re-raster).

#### Import Settings

These live in the Import Settings panel (Section 3.0) and are locked while analysis is running. Changing any of them after analysis requires re-running from the beginning.

| Control | Type | Range | Effect |
|---|---|---|---|
| **Tokens per Page** | Slider + numeric | 200–2000 tokens | Re-paginates (text blob only), then full pipeline. Disabled for PDF (natural pages). |
| **Phrase Length** | Slider + numeric | 5–1500 tokens | Full pipeline rerun |
| **Stride** | Slider + numeric | 1–200 words | Full pipeline rerun; directly controls window count and processing time |
| **Min Repetitions** | Slider + numeric | 2–20 occurrences | Sets HDBSCAN `min_cluster_size` via `min_reps × max(1, floor(phrase_length / stride))`; a phrase must recur at least this many times to form a cluster |
| **Min Samples** | Slider + numeric | 1–10 | HDBSCAN noise sensitivity; higher = stricter noise filtering, fewer marginal cluster members |
| **Chapter Break** | Regex text field | — | Optional regex for chapter/section boundary detection in plain-text imports. Default: `^Chapter\s+\d+`. Leave blank to use token-count pagination only. Also matches markdown headings — e.g. `^(Chapter\s+\d+\|#\s+.+)` for mixed formats. |

**Stride guidance:**

| Document size | Recommended stride | Approx. windows | Embedding time (CPU) |
|---|---|---|---|
| < 50 pages | 1–2 words | < 10,000 | < 1 min |
| 50–200 pages | 3–5 words | 10,000–40,000 | 1–8 min |
| 200–500 pages | 5–15 words | 20,000–60,000 | 4–15 min |
| 500–1000 pages | 15–40 words | 20,000–50,000 | 4–12 min |
| 1000+ pages (epic) | 40–100 words | 20,000–60,000 | 4–15 min |

Default: **`max(1, floor(phrase_length × 0.25))`** — 25% of phrase length, minimum 1 word. This is shown in the panel and the user can override it freely.

The Import Settings panel shows live estimates (window count, embedding time) that update as sliders move, derived from the benchmark sample taken at first launch. This lets the user tune stride before committing — no need to cancel a multi-hour run.

**Tokens per Page guidance:**

| Value | Approximate equivalent |
|---|---|
| 200 | ~1 paragraph / short scene beat |
| 400 | ~1 printed page (typical novel prose) |
| 800 | ~1 scene (short chapter or section) |
| 1500 | ~1 chapter (short chapters) |
| 2000 | ~1 chapter (long chapters) |

Default: **400 tokens** — closest to a conventional printed page, which keeps the macro-grid page count meaningful and the sub-grid positions spatially intuitive.

The UI shows a warning when `tokens_per_page < 4 × phrase_length` (sub-cells may be sparsely populated) and when the stride estimate exceeds 30 minutes (nudges user to increase stride).

#### Display Settings

These update the visualization without re-running the analysis pipeline.

| Control | Type | Scope | Effect |
|---|---|---|---|
| **Tolerance** | Slider (0.75–1.00) | Display | Re-applies alpha mask — no re-raster, no IPC |
| **Cluster Filter** | Toggle per cluster | Display | Re-rasters affected pages only |
| **Gamma (γ)** | Slider (0.5–3.0) | Display | Re-rasters all canvases (saturation curve only) |

**Phrase Length breakpoints:**

| Token Range | What it detects |
|---|---|
| 5–20 | Repeated phrases and sentence fragments |
| 20–100 | Repeated sentences and short passages |
| **100–500** | **Repeated paragraphs, scene beats, large prose blocks** — best range for detecting near-duplicate pages or multi-paragraph runs |
| 500–1500 | Structural repetition (chapter openings, chapter endings, recurring framing devices) |

A repeated block of ~6 paragraphs (~600 tokens) is most cleanly resolved at **100–500 tokens**. At that scale each window covers 1–3 paragraphs, producing a tight dense cluster in embedding space. All instances of the block appear as the same cluster color banded across the same sub-cell rows on every affected page — and because 600 tokens spans a page boundary at 400 tokens/page, the band wraps visibly from the bottom of one page cell into the top of the next, consistently at every occurrence.

**Tolerance thresholds:**

| Value | Meaning |
|---|---|
| 1.00 | Exact duplicates only |
| 0.95 | Near-identical phrasing |
| 0.90 | Paraphrase |
| 0.85 | Strong thematic echo |
| 0.80 | Same idea, different words |
| < 0.80 | Not shown |

### 7.3 Analysis Progress and Cancellation

Once analysis starts, the Import Settings panel transitions to a **progress view**:

```
┌─────────────────────────────────────────────────────────┐
│  Analyzing…                              [Cancel import] │
├─────────────────────────────────────────────────────────┤
│  ● Paginating          ✓ done                            │
│  ● Windowing           ✓ done (24,312 windows)           │
│  ● Embedding           ████████████░░░░░  62%            │
│    15,073 / 24,312 windows · ~3 min 20 sec remaining    │
│  ○ Clustering          —                                 │
│  ○ Rasterizing         —                                 │
│                                                          │
│  Pages ready: 47 / 200  (grid fills in as pages arrive) │
└─────────────────────────────────────────────────────────┘
```

**Progress events:** The Rust backend emits `similarity-map:progress` events with `{ stage, pct, windows_done, windows_total, eta_seconds }`. The ETA is a rolling estimate: the backend tracks a sliding window of the last 50 embedding batch durations and extrapolates from the current throughput rather than the initial benchmark.

**Cancellation:** The **Cancel import** button calls `cancel_analysis(job_id)`. The backend stops at the next batch boundary (within ~1 second), **commits all completed embedding batches to LanceDB**, marks the job `status: "partial"`, and restores the Import Settings panel with all previous values intact.

**Partial results:** Pages already rasterized before cancellation are discarded — the grid is not shown in a partial state. But the embeddings are saved. The Import Settings panel returns showing the same settings that were active, and a **resume banner** appears if at least one batch was committed:

```
┌─────────────────────────────────────────────────────────┐
│  ↩  Partial analysis saved (62% · 15,073 windows done)  │
│     Resume with current settings, or change and restart │
│                              [Resume]  [Start fresh]    │
└─────────────────────────────────────────────────────────┘
```

If the user changes any import setting (stride, phrase length, tokens per page) the partial job is incompatible and the banner changes to **Start fresh** only, with the partial job discarded from LanceDB to free space.

**Grid fills in progressively:** During a successful run, page canvases arrive as `similarity-map:page-ready` events and are drawn into the grid immediately. The user sees the map populate left-to-right, top-to-bottom as each page's clustering and rasterization completes — clustering runs per-batch, not waiting for all embeddings.

### 7.4 Interactions

| Interaction | Behavior |
|---|---|
| Hover on macro-cell | Tooltip: page number, dominant clusters, max similarity |
| Hover on sub-cell | Tooltip: approximate text position, cluster name, similarity score, excerpt |
| Click on macro-cell | Side panel: full list of matching windows with counterpart page links |
| Click on sub-cell | Side panel jumps to that specific window's matches |
| Click a counterpart link | That page's macro-cell pulses; its sub-cell highlights |
| Drag Tolerance slider | Sub-cell alpha updates in-place — no re-raster |
| Drag Phrase Length / Stride / Tokens per Page | Opens Import Settings panel with current values; user must click Analyze to rerun |

### 7.5 Canvas Rendering Strategy

Each page cell is a pre-rendered **20×20 px RGBA pixel array** built in Rust after clustering completes. The frontend receives these arrays once and creates static `ImageBitmap` objects from them.

**Why 20×20 px:**
- 20×20 sub-grid × 1 pixel per sub-cell = 20 px per axis
- Each pixel corresponds to exactly one sub-cell position on the page
- The rasterizer iterates all 400 pixels, looks up the sub-cell's cluster list, blends the color (Section 3.7), and writes the RGBA value
- Total memory per page: **1.6 KB** (20 × 20 × 4 bytes)
- 300 pages: **~480 KB** — negligible even for long manuscripts
- Compositing 300 bitmaps onto a single canvas is a trivial GPU draw pass; the entire grid fits in a ~200×600 px area at 1:1 scale

**Rasterization loop (Rust pseudocode):**
```rust
for row in 0..20_usize {
  for col in 0..20_usize {
    let clusters = &grid.cells[row][col].clusters;
    let n = clusters.len().min(8);

    let color = if n == 0 {
      TRANSPARENT
    } else {
      // Similarity-weighted blend in linear RGB
      let mut r = 0.0_f32;
      let mut g = 0.0_f32;
      let mut b = 0.0_f32;
      let mut total_w = 0.0_f32;

      for cluster in &clusters[..n] {
        if cluster.similarity < threshold { continue; }
        let w = cluster.similarity.powf(gamma);
        let (cr, cg, cb) = hsv_to_linear_rgb(
          cluster_hue(cluster.id),
          sat(cluster.similarity, gamma),
          val(page_density),
        );
        r += w * cr;  g += w * cg;  b += w * cb;
        total_w += w;
      }

      if total_w == 0.0 {
        TRANSPARENT
      } else {
        linear_to_srgb_rgba(r / total_w, g / total_w, b / total_w, 1.0)
      }
    };

    let px = (row * 20 + col) * 4;
    pixels[px..px+4].copy_from_slice(&color);
  }
}
```

**Resize / zoom behavior:**  
The macro-grid scales purely via CSS. At standard viewing scale (~1× to 4×), `image-rendering: pixelated` preserves hard pixel boundaries so each sub-cell reads as a distinct color dot. At high zoom (where the user wants to see intra-sub-cell detail), the renderer can optionally switch to spatial dithering (Section 3.7, zoom table) computed in the frontend from the stored sub-cell cluster lists — no re-IPC required.

**Threshold-only updates (Tolerance slider):**  
The frontend maintains a **mask canvas** per page: a 20×20 alpha layer applied on top of the base bitmap. Each pixel's alpha is `1` if the highest-similarity cluster in that sub-cell exceeds the current threshold, else `0`. Updating the mask is a 400-pixel scan with no IPC round-trip — threshold dragging is effectively instant.

**Cluster-filter updates:**  
When a cluster is toggled off, only the pages containing that cluster need re-rastering. The `ClusterRegistry` maintains a `cluster_id → Vec<page>` index so the Rust backend issues targeted `raster_pages(job_id, affected_pages)` commands. Re-rastering a single 20×20 canvas takes microseconds.

---

## 8. Data Model

### LanceDB Schema

```
Table: windows
┌──────────────────┬──────────────────┬──────────────────────────────────────────────────┐
│ Field            │ Type             │ Description                                      │
├──────────────────┼──────────────────┼──────────────────────────────────────────────────┤
│ window_id        │ string (UUID)    │ Unique window identifier                          │
│ job_id           │ string (UUID)    │ Parent analysis job                               │
│ window_index     │ u32              │ Sequential index within job (0-based); used for   │
│                  │                  │ resume — skip rows where index < windows_committed│
│ page             │ u32              │ 1-based page number                               │
│ char_start       │ u32              │ Char offset from start of page text               │
│ char_end         │ u32              │ End of window in page text                        │
│ doc_char_start   │ u32              │ Char offset from start of full document           │
│ text             │ string           │ Raw window text                                   │
│ embedding        │ vector(N)        │ Float32 embedding                                 │
│ cluster_id       │ i32              │ Stable KMeans cluster label                       │
│ hdbscan_label    │ i32              │ HDBSCAN label (-1 = noise)                        │
│ sim_to_centroid  │ f32              │ Cosine similarity to the cluster centroid vector  │
│ sub_cell_row     │ u8               │ Pre-computed sub-cell row (0–19)                  │
│ sub_cell_col     │ u8               │ Pre-computed sub-cell col (0–19)                  │
└──────────────────┴──────────────────┴──────────────────────────────────────────────────┘

Table: pages
┌──────────────────┬──────────────────┬──────────────────────────────────────────────────┐
│ Field            │ Type             │ Description                                      │
├──────────────────┼──────────────────┼──────────────────────────────────────────────────┤
│ page             │ u32              │ 1-based page number                               │
│ doc_char_start   │ u32              │ Char offset of page start in full document        │
│ doc_char_end     │ u32              │ Char offset of page end in full document          │
│ token_count      │ u32              │ Approximate token count for this page             │
│ pagination_mode  │ string           │ "pdf", "token", or "chapter"                      │
└──────────────────┴──────────────────┴──────────────────────────────────────────────────┘
```

### Sub-Cell Grid (in-memory, per page)

```rust
/// 20×20 grid of sub-cells for one page.
/// Each sub-cell holds all clusters present within it, sorted by similarity
/// descending, ready for color-blend rendering (or dither at zoom).
struct PageSubGrid {
    page: u32,
    cells: [[SubCell; 20]; 20],
}

struct SubCell {
    /// All clusters present in this sub-cell, sorted by sim_to_centroid desc.
    /// Empty vec = no windows / below threshold. Capped at 8 entries.
    clusters: Vec<SubCellCluster>,
}

struct SubCellCluster {
    cluster_id: i32,
    similarity: f32,        // sim_to_centroid of the best window in this cluster
    window_id: String,      // best-matching window for tooltip/detail lookup
}
```

### Page Canvas (sent to frontend)

```rust
/// Flat RGBA pixel array for a single page cell.
/// Layout: row-major, (0,0) = top-left = start of page text.
/// Each pixel = one sub-cell (1:1 mapping at base scale).
struct PageCanvas {
    page: u32,
    pixels: [u8; 20 * 20 * 4],   // RGBA, 1.6 KB per page
}
```

### Display State (sidecar JSON per job)

A small JSON file stored alongside the LanceDB table (`$APPDATA/similarity-map/sessions/<job_id>.json`) records the user's last display settings for that job. Restored automatically when the user chooses **Restore Session**.

```json
{
  "job_id": "...",
  "tolerance": 0.88,
  "gamma": 1.5,
  "hidden_clusters": [3, 7],
  "zoom": 1.0,
  "scroll_x": 0,
  "scroll_y": 0,
  "saved_at": "2026-05-24T14:32:00Z"
}
```

Written on every display-setting change (debounced 2 s) and on window close. Deleted when the job is discarded.

### Job Registry (LanceDB + sidecar JSON)

Each analysis run — complete or partial — is recorded in a `jobs` table so the system can detect resumable state on next open.

```
Table: jobs
┌──────────────────┬──────────────────┬──────────────────────────────────────────────────┐
│ Field            │ Type             │ Description                                      │
├──────────────────┼──────────────────┼──────────────────────────────────────────────────┤
│ job_id           │ string (UUID)    │ Unique run identifier                             │
│ document_path    │ string           │ Absolute path to the source file                  │
│ document_hash    │ string (SHA-256) │ Hash of file contents — detects edits             │
│ settings_hash    │ string (SHA-256) │ Hash of {window_size, stride, tokens_per_page,    │
│                  │                  │   min_repetitions, min_samples}                   │
│ window_size      │ u32              │ Phrase length used                                │
│ stride           │ u32              │ Stride used                                       │
│ tokens_per_page  │ u32 / null       │ null = PDF natural pages                          │
│ pagination_mode  │ string           │ "pdf", "token", or "chapter"                      │
│ min_repetitions  │ u32              │ Minimum cluster recurrences (default 3)            │
│ min_samples      │ u32              │ HDBSCAN min_samples (default 3)                   │
│ chapter_break_re │ string / null    │ Chapter break regex, null if unused               │
│ windows_total    │ u32              │ Total windows planned for this job                │
│ windows_committed│ u32              │ Windows successfully embedded and written         │
│ status           │ string           │ "running", "partial", "complete", "discarded"     │
│ created_at       │ timestamp        │ When analysis started                             │
│ updated_at       │ timestamp        │ Last batch commit or status change                │
└──────────────────┴──────────────────┴──────────────────────────────────────────────────┘
```

`settings_hash` is the key that determines whether a partial job is resumable. If the user changes stride, window size, or tokens per page, the hash changes and the partial job is treated as incompatible — the resume banner is hidden and the partial job is deleted on next "Start fresh" confirmation.

`document_hash` guards against resuming into a file that has been edited since the job started. If the hash has changed, the partial job is automatically discarded and the user is notified.

### Cluster Registry (in-memory)

```rust
struct ClusterRegistry {
    clusters: HashMap<i32, ClusterInfo>,
}

struct ClusterInfo {
    cluster_id: i32,
    hue: f32,                        // golden-ratio assigned
    centroid: Vec<f32>,              // mean of all member embeddings (384-dim)
    most_central_window_id: String,  // member with highest cosine sim to centroid
                                     // — used as the display excerpt in tooltips
    member_count: u32,
    pages: Vec<u32>,                 // pages where this cluster appears
}
```

---

## 9. Architecture

### Tech Stack

| Layer | Technology | Rationale |
|---|---|---|
| Shell | Tauri 2 (Rust + WebView) | Lightweight desktop, native file access |
| Backend | Rust | Embedding, clustering, sub-cell mapping, canvas rasterization |
| Embedding | ONNX Runtime (`ort` crate, local) — `all-MiniLM-L6-v2` | 22 MB model, auto-downloaded on first use to app data dir; fully offline after that |
| Vector store | LanceDB | Local, fast, no server, metadata-rich |
| Clustering | HDBSCAN via Python FFI or Rust port | HDBSCAN + KMeans two-stage pipeline |
| Frontend | Vanilla JS + Canvas 2D | ImageBitmap compositing for the grid; mask layer for threshold |
| IPC | Tauri commands + events | Rust → frontend pixel array delivery (streamed per page) |

### Tauri Command Surface

```
// Called immediately when a document is opened, before showing any UI.
// Returns the most recent complete session (if any) and any partial job (if any).
check_document_session(path: String)
  → DocumentSessionState {
      complete_job: Option<CompleteJobInfo {
        job_id,
        created_at,
        page_count,
        window_size,
        stride,
        tokens_per_page: Option<u32>,
        pagination_mode,
      }>,
      partial_job: Option<PartialJobInfo {
        job_id,
        windows_committed,
        windows_total,
        pct,
        cancelled_at,
        window_size,
        stride,
        tokens_per_page: Option<u32>,
      }>,
    }

restore_session(job_id: String)
  → RestoreHandle { job_id, page_count }
  // Re-rasters all canvases from stored LanceDB index. No re-embedding.
  // Streams page-ready events just like analyze_document.
  event: similarity-map:progress { job_id, stage: "rasterizing", pct }
  event: similarity-map:page-ready { job_id, page, canvas_rgba_b64 }

discard_job(job_id: String)
  → ()
  // Deletes all windows rows for this job from LanceDB and removes the job record.
  // Called when user chooses "Generate New Map" over a complete or partial session.

// Called once at startup (or before first analysis). Returns immediately if
// model is already present; otherwise streams download progress events.
ensure_embedding_model()
  → ModelStatus { present: bool, path: String, size_mb: f32 }
  event: similarity-map:model-download-progress { pct, bytes_received, total_bytes }
  event: similarity-map:model-ready { path }

// Called before opening the Import Settings panel.
// Returns the live window-count estimate and ETA for the given settings
// without starting analysis — used to drive the live feedback in the panel.
estimate_analysis(
    path: String,
    window_size: u32,
    stride: u32,
    tokens_per_page: Option<u32>,
  )
  → AnalysisEstimate { page_count, window_count, eta_seconds, benchmark_windows_per_sec }

analyze_document(
    path: String,
    window_size: u32,
    stride: u32,
    tokens_per_page: Option<u32>,         // None = use natural PDF page breaks
    chapter_break_regex: Option<String>,  // None = token-count only; default "^Chapter\s+\d+"
    min_repetitions: u32,                 // default 3
    min_samples: u32,                     // default 3
  )
  → AnalysisHandle { job_id, page_count, window_count, pagination_mode: "pdf" | "token" | "chapter" }
  event: similarity-map:progress {
    job_id,
    stage: "paginating" | "windowing" | "embedding" | "clustering" | "rasterizing",
    pct,                    // 0.0–1.0 within the current stage
    windows_done,           // embedding stage only
    windows_total,          // embedding stage only
    eta_seconds,            // rolling estimate; null for non-embedding stages
  }
  event: similarity-map:page-ready { job_id, page, canvas_rgba_b64 }

get_page_canvases(job_id: String, threshold: f32)
  → Vec<PageCanvas>

raster_pages(job_id: String, pages: Vec<u32>, threshold: f32)
  → Vec<PageCanvas>     // targeted re-raster for cluster filter updates

get_page_detail(job_id: String, page: u32, row: u8, col: u8, threshold: f32)
  → SubCellDetail { window_text, cluster_id, similarity, matches: Vec<WindowMatch> }

cancel_analysis(job_id: String)
  → CancelResult { windows_committed, status: "partial" | "discarded" }
  // Commits all completed embedding batches to LanceDB before returning.
  // status = "partial" when at least one batch was committed (resumable).
  // status = "discarded" when cancelled before any embeddings completed.

// Check whether a resumable partial job exists for a document + settings combo.
check_partial_job(
    path: String,
    window_size: u32,
    stride: u32,
    tokens_per_page: Option<u32>,
  )
  → Option<PartialJobInfo { job_id, windows_committed, windows_total, pct, cancelled_at }>

resume_analysis(job_id: String)
  → AnalysisHandle { job_id, page_count, window_count, windows_already_done }
  // Same progress events as analyze_document; eta_seconds accounts for
  // already-completed windows. Embedding resumes from windows_already_done.
  event: similarity-map:progress { ... }
  event: similarity-map:page-ready { ... }
```

Pages stream to the frontend as `similarity-map:page-ready` events during analysis so the grid fills in progressively rather than all at once.

---

## 10. Performance Considerations

| Operation | Trigger | Re-computation scope | Expected cost |
|---|---|---|---|
| Pagination | Tokens per Page slider (text blob) | Full document | Fast (text scan + slice); cached until setting changes |
| Windowing + char offsets | Phrase Length or Stride change | Full document | Fast (text only); re-runs within fixed page boundaries |
| Embedding | Phrase Length or Stride change | All windows | **Dominant cost.** Window count = `(total_tokens − window_size) / stride`. Progress bar with rolling ETA. Cached in LanceDB; unchanged settings skip this stage. |
| HDBSCAN | Phrase Length slider | All embeddings | Medium |
| KMeans stabilization | Phrase Length slider | HDBSCAN output | Fast |
| Sub-cell mapping | Phrase Length slider | All windows | Fast (arithmetic) |
| Canvas rasterization | Phrase Length slider / Gamma slider / cluster toggle | All pages (or targeted subset) | Very fast — 20×20 px (400 pixels) per page; color blend per pixel in tight loop |
| Threshold mask update | Tolerance slider | Frontend only, no IPC | Near-instant — 1-bit mask scan per page canvas |
| Cluster filter toggle | Cluster toggle | Affected pages only (via `cluster → pages` index) | Fast targeted re-raster |
| Resize / zoom | Window resize | None — CSS scaling only | Instant |

**Caching strategy:** LanceDB persists embeddings and sub-cell assignments between sessions. If the document content and window size are unchanged on reopen, the entire pipeline up to and including clustering is skipped. Only the canvas rasterization step re-runs (it's fast and stateless given the stored index).

### 10.1 Checkpoint and Resume

The embedding stage is the only stage worth checkpointing — all other stages are either fast enough to re-run (pagination, windowing, sub-cell mapping, rasterization) or depend on the completed embedding set (clustering). Embeddings are written to LanceDB in batches of 32 windows. On cancel, the final partial batch is discarded and the last fully committed batch marks the checkpoint.

**Resume flow:**

```
1. User opens document  →  backend calls check_partial_job(path, settings)
2. Partial job found    →  Import Settings panel shows resume banner with % complete
3. User clicks Resume   →  resume_analysis(job_id)
4. Backend queries LanceDB: SELECT window_id FROM windows WHERE job_id = ? ORDER BY window_index
5. Already-embedded windows are skipped; embedding loop starts at windows_committed
6. Clustering runs on the full final embedding set once all windows are done
7. Job status updated to "complete"; partial job record retired
```

**Settings change → incompatible:**

```
1. User opens resume banner, then moves the Stride slider
2. settings_hash changes → banner switches to "Start fresh" only
3. On "Start fresh" confirm → DELETE FROM windows WHERE job_id = <partial_job_id>
4. Old job record marked "discarded"
5. Fresh analyze_document() begins
```

**Document edited since last run:**

```
1. On open, backend computes SHA-256 of current file
2. document_hash mismatch → partial job auto-discarded with notification:
   "The document was edited since the partial analysis. Starting fresh."
```

**Storage cost of a partial job:**  
Each embedded window stores a 384-float vector (1.5 KB) plus metadata (~200 bytes) ≈ ~1.7 KB per window. For a 1000-page epic at stride 40: ~25,000 windows → ~42 MB for a fully-saved partial run. Acceptable; displayed to the user in the resume banner ("42 MB saved").

### 10.2 Session Restore

When a document is opened, `check_document_session(path)` runs immediately. The result drives the first thing the user sees:

**State machine on document open:**

```
check_document_session(path)
        │
        ├─ complete_job found (document_hash matches)
        │       │
        │       └──▶  Show session dialog:
        │
        │   ┌──────────────────────────────────────────────────────┐
        │   │  Previous session found                              │
        │   │                                                      │
        │   │  A similarity map for this file was generated on     │
        │   │  May 24, 2026 · 312 pages · stride 5 · phrase 20    │
        │   │                                                      │
        │   │        [Restore Session]   [Generate New Map]        │
        │   └──────────────────────────────────────────────────────┘
        │
        │       [Restore Session] ──▶ restore_session(job_id)
        │                              re-rasters from LanceDB (~seconds)
        │                              loads display state JSON
        │                              grid appears at last scroll / zoom
        │
        │       [Generate New Map] ──▶ discard_job(complete_job.job_id)
        │                              open Import Settings panel
        │                              (partial_job resume banner shown if exists)
        │
        ├─ no complete_job, partial_job found
        │       │
        │       └──▶  Open Import Settings panel with resume banner (Section 7.3)
        │
        └─ no session at all
                │
                └──▶  Open Import Settings panel fresh
```

**Restore is fast.** All embeddings, cluster assignments, centroid vectors, and sub-cell mappings are already in LanceDB. The restore path only runs the rasterization stage — 400 pixels × N pages, each a tight inner loop. For 300 pages this completes in under a second on any modern CPU. A brief "Restoring…" progress bar is shown but rarely seen to completion.

**Multiple complete sessions (different settings, same file):** Only the most recently completed job is offered in the dialog. Older complete jobs for the same file are retained in LanceDB until explicitly discarded (via a future "Manage sessions" UI) or until the user chooses "Generate New Map" (which discards all prior jobs for that file).

**Session dialog is non-blocking.** The grid panel is visible but empty behind the dialog. If the user dismisses the dialog via Escape or clicks outside it, behaviour is the same as "Generate New Map."

---

## 11. Design Decisions

All questions have been resolved. This section records each decision for reference.

| # | Question | Decision |
|---|---|---|
| 0 | Tokenizer for Tokens per Page | Whitespace split (`split_whitespace()`). Fast, dependency-free, sufficient for prose pagination. |
| 1 | Embedding model | Local ONNX only — `all-MiniLM-L6-v2` (22 MB, Apache 2.0). Auto-downloaded from Hugging Face on first use; fully offline thereafter. Manuscript text never leaves the machine. |
| 2 | Stride policy | First-class import setting. Default `max(1, floor(window_size × 0.25))`. User overrides freely; UI shows live window count and ETA; nudges toward larger stride if estimate exceeds 30 min. |
| 3 | Anchor / centroid | Cluster centroid (mean of all member embeddings). `most_central_window_id` is the display exemplar. Document edits always force full reprocess — no stale centroid risk. |
| 4 | Sub-cell collision | Similarity-weighted color blend (linear RGB) at 1 px/sub-cell. Spatial dither patterns (checker, thirds, quadrant, scatter) reserved for zoom view, computed frontend-side. |
| 5 | Session persistence | On document open: check for complete job (matching `document_hash`). Dialog: **[Restore Session] / [Generate New Map]**. Restore re-rasters from LanceDB in under a second and reloads display state JSON. See Section 10.2. |
| 6 | Sub-grid resolution | 20×20 sub-grid, 20×20 px canvas — 1 pixel per sub-cell at base scale. Macro-grid: 10 cols × up to 30 rows (portrait). |
| 7 | Checkpoint / resume | Cancel commits completed embedding batches to LanceDB, saves job as `status: "partial"`. On next open with matching settings hash: resume banner in Import Settings panel. See Section 10.1. |
| 8 | HDBSCAN parameters | **Min Repetitions** (default 3) and **Min Samples** (default 3) are import settings in the panel. Min repetitions is converted to `min_cluster_size` internally via phrase/stride ratio. Scaling guidance by document size in Section 5. |
| 9 | KMeans role | Stable integer labeling only — `k = HDBSCAN cluster count`, no merging or reduction. Many clusters (e.g. 80–100 in 300 pages) is a valid and diagnostically important result; the dense multi-hue grid is itself the signal. |
| 10 | Value channel | `V = sim_to_centroid^γ` — brightness encodes typicality. V=1.0 at the centroid / exact matches; dimmer for weaker echoes. Self-normalizing; no `density_ceiling` parameter needed. Saturation fixed at 1.0. |
| 11 | Chapter break regex | Optional text field in Import Settings. Default `^Chapter\s+\d+`. User edits freely for markdown headings (`^#\s+.+`), scene breaks, or mixed formats. Leave blank for pure token-count pagination. |
| 12 | Session storage management | No automatic cleanup. Sessions accumulate in LanceDB until explicit user action. 42 MB per completed 1000-page job is negligible in the context of modern ML tooling; this is not a problem worth solving automatically. |
