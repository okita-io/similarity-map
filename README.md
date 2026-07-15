# Similarity Map

A local manuscript-repetition analyzer with a Tauri desktop visualizer, a reusable
Rust core, a headless CLI, and Python bindings.

Part of the **Romance Factory** manuscript tooling ecosystem. The versioned
`AnalysisOutput` contract is designed for editorial-pipeline consumption; the desktop
app provides spatial exploration and persistent file-based sessions.

## What it does

Analyze a novel or long document and get a **repetition fingerprint**. In the desktop
app, each grid cell represents a page: color shows *what* is repeating, brightness
shows *how closely* it matches its cluster centroid, and position shows *where on the
page* it appears.

The map helps authors and editors answer questions like:

- Which pages recycle the same phrases or scene beats?
- Is repetition clustered in one chapter or spread throughout?
- Are echoes exact duplicates, near-identical wording, or paraphrases?

## How it looks

The macro-grid wraps page cells to the available window width. Each page is rendered
from a **20×20 RGBA raster** and displayed at a larger CSS-controlled size, so the user
can see both *that* a page repeats something and *where* on that page.

**Color (hue)** — cluster identity: each recurring phrase/motif gets a distinct color.  
**Brightness (value)** — how archetypal the match is (brighter = closer to the cluster's core phrasing).  
**Empty pixels** — no repetition above the current threshold.

The text preview shows canonical and duplicate spans and can navigate to pages by
cluster. Direct grid-cell drill-down and custom hover tooltips are not complete; see
[`CURRENT-STATE.md`](./CURRENT-STATE.md).

## How it works

```mermaid
flowchart LR
  A[Document] --> B[Paginate]
  B --> C[Sliding phrase windows]
  C --> D[Embed with MiniLM]
  D --> E[Cluster similar windows]
  E --> F[Map to page sub-grid]
  F --> G[Render color grid]
```

1. **Import** — PDFs keep natural page breaks; plain text is split into configurable token-sized pages (~400 tokens ≈ one printed page).
2. **Window** — Overlapping text windows slide across each page (size and stride are adjustable).
3. **Embed** — Each window becomes a vector via a local ONNX model
   (`all-MiniLM-L6-v2`, offline after the first download).
4. **Cluster** — HDBSCAN finds organic repetition groups; KMeans assigns stable labels so colors stay consistent between runs.
5. **Visualize** — Windows map to a 20×20 sub-grid per page; clusters render as HSV-colored pixels with similarity-weighted blending when multiple motifs overlap.

> **Embedding-quality warning:** the current ONNX path still feeds hash-derived token
> IDs to MiniLM instead of the model's WordPiece tokenizer. Exact and strongly lexical
> repetition can be useful, but paraphrase-level semantic claims are not yet validated.
> Treat production scores as heuristic until tokenizer integration and reference-vector
> tests land.

### Detection scales

| Phrase length | Best for |
|---|---|
| 5–20 tokens | Repeated phrases and sentence fragments |
| 20–100 tokens | Sentences and short passages |
| **100–500 tokens** | **Paragraphs, scene beats, near-duplicate blocks** |
| 500–1500 tokens | Structural patterns (chapter openings, framing devices) |

Run twice at different phrase lengths (e.g. 20 and 200 tokens) to see both fine-grained echoes and large structural repetition.

## Features

- **Reusable analysis core** — in-memory single-pass and Romance Factory multi-pass APIs
- **Versioned editorial output** — validated `AnalysisOutput` v1 JSON with structural span locations
- **Three adapters** — Tauri desktop, JSON/stdin CLI, and PyO3 Python bindings
- **Desktop exploration** — page raster, text highlights, tolerance, cluster filter, and gamma controls
- **Romance Factory workflow** — story/chapter loading, presets, settings YAML export, and JSON export
- **Saved file results** — named analyses plus LanceDB-backed session restore
- **Progressive rendering** — the grid fills in page-by-page as analysis runs
- **Cancel and resume** — partial embedding progress is saved; resume compatible runs or start fresh
- **Offline test path** — deterministic embedder validates contracts and orchestration without ONNX
- **Privacy-first** — local ONNX inference; manuscript text never leaves your machine

## Tech stack

| Layer | Technology |
|---|---|
| Portable engine | Rust crate `similarity-core` |
| Pipeline contract | Serde types + JSON Schema (`AnalysisOutput` v1) |
| Desktop adapter | [Tauri 2](https://v2.tauri.app/) + LanceDB |
| Headless adapters | `similarity-cli` and PyO3 `similarity-core-py` |
| Embeddings | dynamically loaded ONNX Runtime + `all-MiniLM-L6-v2` |
| Clustering | Rust HDBSCAN + deterministic KMeans stabilization |
| Frontend | Vanilla JS + Canvas 2D; no Node build step |

## Building from source

### Prerequisites

- **Rust** (stable, 2021 edition) — [install via rustup](https://rustup.rs/)
- **Tauri CLI** — `cargo install tauri-cli --version "^2"`
- **ONNX Runtime 1.24.x (required for production embedding)** — embedding uses
  `ort` 2.0.0-rc.12 with dynamic loading. The native shared library is separate from
  the downloaded `.onnx` model. Offline tests that use the deterministic embedder do
  not require ONNX Runtime.
  - macOS: `brew install onnxruntime`
  - Linux (Debian/Ubuntu): install a system `libonnxruntime` package, or build from [GitHub releases](https://github.com/microsoft/onnxruntime/releases)
  - If the library is not in a standard location, set the full path before running the app:
    ```bash
    export ORT_DYLIB_PATH="$(brew --prefix onnxruntime)/lib/libonnxruntime.dylib"   # macOS Homebrew
    ```
  - The app probes `ORT_DYLIB_PATH`, then common Homebrew paths (`/opt/homebrew/lib`, `/usr/local/lib` on macOS).
- **Protocol Buffers compiler (`protoc`)** — required at compile time by LanceDB (`lance-encoding` generates code from `.proto` files)
  - macOS: `brew install protobuf`
  - Linux (Debian/Ubuntu): `sudo apt install protobuf-compiler`
  - If `protoc` is installed but not found: `export PROTOC="$(brew --prefix protobuf)/bin/protoc"` (macOS Homebrew)
- **System dependencies** (macOS): Xcode Command Line Tools (`xcode-select --install`)
- **System dependencies** (Linux): `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`

### Development

There are two ways to run the app while developing. The frontend is plain static files in `src/` — no bundler, `npm install`, or separate dev server required. Tauri serves `src/` directly via `frontendDist`.

**A. `cargo tauri dev`** — rebuilds Rust on change and reloads the webview (restart the app to pick up JS/CSS edits):

```bash
cd similarity-map
cargo tauri dev
```

**B. `cargo run`** — same static frontend, fastest if you are only changing Rust:

```bash
cd similarity-map/src-tauri
cargo run
```

To hot-reload frontend files without restarting Tauri, run a static server in another terminal and point the webview at it (optional):

```bash
python3 -m http.server 1420 --directory src
```

Then temporarily add `"devUrl": "http://localhost:1420"` to `src-tauri/tauri.conf.json` (and remove it again for normal dev).

### In-app debug log

There's a collapsible log drawer pinned to the bottom of the app window. It captures:

- All `console.log/info/warn/error` calls from frontend JS
- Unhandled errors and promise rejections
- `similarity-map:log` events emitted from the Rust backend (model load, pipeline stages, IPC commands, errors)

Use the **Level** dropdown to filter, **Copy** to paste a session into a bug report, and **Clear** to reset. From the JS console you can also call `window.logPanel.expand()` and `window.logPanel.log('info', 'me', 'hello')`.

### Running tests

```bash
# Compile every workspace member
cargo check --workspace

# Component suites
cargo test -p similarity-core
cargo test -p similarity-map
cargo test -p similarity-cli --test cli_smoke
cargo test -p similarity-core-py

# Full workspace gate
cargo test --workspace
```

As of 2026-07-14, the component suites pass (305 unit tests plus 2 CLI smoke tests),
but `cargo test --workspace` exposes one CLI unit-test compile error: the test module in
`similarity-cli/src/analyze.rs` is missing an import for `build_scope_manifest`.
`cargo test -p similarity-cli` hits the same test-target defect; the focused
`--test cli_smoke` command above passes. The separate three-test Python `pytest` suite
requires `maturin develop` and was not run in this verification.

### Production build

```bash
# Build a release bundle (output under target/release/bundle/)
cargo tauri build
```

This produces platform-specific installers (.dmg on macOS, .deb/.AppImage on Linux,
.msi on Windows). The current config references `src-tauri/icons/*`, but those icon
assets are not committed; add them before treating the bundle command as a release
gate.

### Project structure

```
similarity-map/
├── similarity-core/        # Portable stages, contracts, reports, storage primitives
│   ├── src/analyze_prose.rs
│   ├── src/multi_pass.rs
│   ├── src/contract.rs
│   ├── schemas/
│   └── fixtures/
├── similarity-cli/         # AnalysisOutput JSON CLI
├── similarity-core-py/     # PyO3/Python adapter
├── src-tauri/              # Desktop adapter plus persistent file pipeline
│   └── src/
│       ├── commands.rs
│       ├── pipeline.rs
│       ├── display_state.rs
│       ├── results_catalog.rs
│       └── app_settings.rs
├── src/                    # Static JS/CSS desktop UI and exports
├── test-data/              # Small manual-test manuscript
├── ARCHITECTURE.md
├── CURRENT-STATE.md
└── .kiro/specs/            # Requirements and integration contract
```

### First run

On first launch the desktop app downloads an architecture-specific, quantized
`all-MiniLM-L6-v2` ONNX model (~23 MB) from Hugging Face and caches it in the Tauri app
data directory. Subsequent launches skip the download.

## Project status

The project is a **credible reusable analysis platform and an incomplete desktop
product**:

- core, contract, CLI, Python binding, RF multi-pass orchestration, file analysis,
  persistence, text preview, and exports are implemented;
- semantic embedding quality is not production-validated because tokenization is still
  a placeholder;
- direct grid drill-down, custom tooltips, high-zoom dithering, and the frontend
  tolerance-mask path are not complete;
- the desktop file pipeline and in-memory headless pipeline still need convergence.

See [`CURRENT-STATE.md`](./CURRENT-STATE.md) for the verified capability review and
[`ARCHITECTURE.md`](./ARCHITECTURE.md) for crate boundaries and the recommended
migration. The original
[`Similarity Map - Design Specification.md`](./Similarity%20Map%20-%20Design%20Specification.md)
is retained as a historical/aspirational desktop specification.

### Desktop Romance Factory workflow

The desktop app can load a Romance Factory story, select a chapter, and run one of
three presets:

- `act_fine` — two act-scoped passes;
- `chapter_coarse` — two chapter-scoped passes;
- `full_multi_pass` — all four passes.

The UI can export the merged `AnalysisOutput` v1 JSON and a paste-ready
`generate:similarity_map:` YAML block. RF chapter runs are currently in-memory and do
not create restorable LanceDB sessions.

### Romance Factory JSON export (`AnalysisOutput` v1)

Pipeline-consumable analysis output for the RF surgical editor is defined in [`.kiro/specs/similarity-map/integration-contract.md`](./.kiro/specs/similarity-map/integration-contract.md). Rust types: `similarity-core/src/contract.rs`. JSON Schema: `similarity-core/schemas/analysis_output_v1.schema.json`. Example fixture: `similarity-core/fixtures/analysis_output_v1.example.json`.

### Headless CLI (`similarity-cli`)

For debugging and pre-PyO3 pipeline integration, run repetition analysis without the Tauri UI:

```bash
cd similarity-map

# Romance Factory story chapter (loads drafts/chapter_NN.json or chapters/chapter_NN.md)
cargo run -p similarity-cli -- analyze \
  --story-path ../stories/my_novel \
  --chapter 1 \
  --pass-config similarity-cli/fixtures/pass_config_smoke.yaml \
  --test-embedder   # omit in production; use ONNX model instead

# JSON stdin: { "text", "scope_manifest", "params" }
cargo run -p similarity-cli -- analyze --test-embedder < request.json

# RF-style multi-pass bundle (YAML excerpt under generate:similarity_map:)
cargo run -p similarity-cli -- analyze \
  --story-path ../stories/my_novel \
  --chapter 3 \
  --pass-config similarity-cli/fixtures/pass_config_smoke.yaml \
  --test-embedder > chapter_03.repetition.json
```

**Output:** pretty-printed `AnalysisOutput` v1 JSON on stdout (contract in `integration-contract.md`). Errors go to stderr.

**Flags:**

| Flag | Description |
|---|---|
| `--story-path` + `--chapter` | Load RF chapter prose and build `scope_manifest` automatically |
| `--input-file` | Read `{ text, scope_manifest, params }` JSON from a file |
| `--pass-config` | YAML pass bundle (`min_repetitions`, `passes[]` with `window_size` / `stride`) |
| `--expand-sentences` / `--no-expand-sentences` | Clip spans to sentence boundaries (default: expand) |
| `--model-path` | ONNX model path (or set `SIMILARITY_MAP_MODEL_PATH`) |
| `--window-size`, `--stride`, … | Single-pass overrides when `--pass-config` is omitted |

**Production runs** require the `all-MiniLM-L6-v2` ONNX model (same as the desktop app). Point `--model-path` at the cached file or set `SIMILARITY_MAP_DATA_DIR` / `SIMILARITY_MAP_MODEL_PATH`.

### PyO3 Python bindings (`similarity-core-py`)

Direct pipeline integration without Tauri or subprocess CLI:

```bash
cd similarity-map/similarity-core-py
pip install maturin   # once
maturin develop       # builds and installs editable `similarity_core` package
```

```python
import similarity_core

result = similarity_core.analyze_prose(
    text,
    scope_manifest,   # dict — act/paragraph index from build_scope_manifest
    params,             # dict — window_size, stride, min_repetitions, …
    test_embedder=True, # omit in production; uses ONNX model
)
# result is AnalysisOutput v1 (JSON-serializable dict)

result = similarity_core.analyze_prose_multi_pass(
    text,
    scope_manifest,
    pass_config,        # dict — MultiPassConfig (passes[], min_repetitions, …)
    test_embedder=True,
)
```

**Model path resolution** (production ONNX runs):

| Env var | Effect |
|---|---|
| `SIMILARITY_MAP_MODEL_PATH` | Full path to `all-MiniLM-L6-v2.onnx` |
| `SIMILARITY_MAP_MODEL_DIR` | Directory containing the model (or `models/` subdir) |
| `SIMILARITY_MAP_DATA_DIR` | Headless app-data root (default: `~/Library/Application Support` on macOS) |

The desktop cache includes Tauri's identifier directory
(`~/Library/Application Support/com.similarity-map.app/` on macOS). Headless adapters
do not automatically append that identifier; set `SIMILARITY_MAP_MODEL_PATH` when
sharing the desktop-cached model.

Tests (offline, no ONNX):

```bash
cd similarity-map/similarity-core-py
maturin develop
pytest tests/test_analyze_prose.py -v
```

### Headless pipeline & ONNX Runtime

Headless runs (CLI, PyO3 in Romance Factory, CI) need the **ONNX Runtime native shared library** in addition to the downloaded `.onnx` embedding model. The Rust `ort` crate uses **dynamic loading** (`load-dynamic`); the dylib must exist before any session API runs.

**Version requirement:** `similarity-core` depends on `ort` 2.0.0-rc.12, which targets **ONNX Runtime 1.24.x**. Do not use 1.20.x or other mismatched builds — embedding often stalls at 0% with no useful error.

#### Install paths (probed automatically)

`similarity-core/src/ort_runtime.rs` searches in order:

1. **`ORT_DYLIB_PATH`** — full path to the shared library (always preferred in CI and non-standard installs)
2. Platform defaults (only if `ORT_DYLIB_PATH` is unset):

| OS | Default probe paths |
|---|---|
| **macOS** | `/opt/homebrew/lib/libonnxruntime.dylib`, `/usr/local/lib/libonnxruntime.dylib` |
| **Linux** | `/usr/lib/x86_64-linux-gnu/libonnxruntime.so`, `/usr/lib/aarch64-linux-gnu/libonnxruntime.so`, `/usr/local/lib/libonnxruntime.so`, `/usr/lib/libonnxruntime.so` |

#### macOS install

```bash
brew install onnxruntime
export ORT_DYLIB_PATH="$(brew --prefix onnxruntime)/lib/libonnxruntime.dylib"
```

Apple Silicon Homebrew also installs under `/opt/homebrew/lib/` (included in auto-probe).

#### Linux install

**Option A — GitHub release (recommended for CI and pinned version):**

```bash
ORT_VERSION=1.24.2
curl -fsSL "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz" \
  | tar xz -C /tmp
sudo cp /tmp/onnxruntime-linux-x64-${ORT_VERSION}/lib/libonnxruntime.so* /usr/local/lib/
export ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
```

Use `onnxruntime-linux-aarch64-${ORT_VERSION}.tgz` on arm64.

**Option B — distro package** (verify version ≥ 1.24 before relying on it):

```bash
# Debian/Ubuntu — package version varies; prefer Option A if embedding hangs
sudo apt install libonnxruntime libonnxruntime-dev
export ORT_DYLIB_PATH=/usr/lib/x86_64-linux-gnu/libonnxruntime.so
```

The external Romance Factory monorepo provides
`scripts/ci_install_onnxruntime.sh` for macOS Homebrew or Linux x64/aarch64 release
installation. That script is not part of this standalone repository.

#### Embedding model (`.onnx` file)

Separate from ONNX Runtime — this is the MiniLM weights file (~23 MB quantized):

| Env var | Effect |
|---|---|
| `SIMILARITY_MAP_MODEL_PATH` | Full path to `all-MiniLM-L6-v2.onnx` |
| `SIMILARITY_MAP_MODEL_DIR` | Directory containing the model (or `models/` subdir) |
| `SIMILARITY_MAP_DATA_DIR` | Headless app-data root (`~/Library/Application Support` on macOS, `~/.local/share` on Linux) |

**CI / headless download example:**

```bash
export SIMILARITY_MAP_MODEL_DIR="$HOME/.cache/similarity-map/models"
mkdir -p "$SIMILARITY_MAP_MODEL_DIR"
curl -fsSL \
  "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model_quint8_avx2.onnx" \
  -o "$SIMILARITY_MAP_MODEL_DIR/all-MiniLM-L6-v2.onnx"
```

Use `model_qint8_arm64.onnx` on Apple Silicon if you download manually from Hugging Face.

#### Romance Factory CI

The external Romance Factory monorepo optionally runs
`.github/workflows/similarity-map-ci.yml` and documents it at
`docs/design/similarity-map-onnx-ci.md`. Those paths are not present in this standalone
checkout. This repository currently has no in-tree CI workflow.

#### Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `ONNX Runtime shared library (…) not found` | Dylib missing | Install 1.24.x; set `ORT_DYLIB_PATH` |
| Progress stuck at 0% during embed | ORT **version mismatch** | Use ONNX Runtime 1.24.x, not 1.20.x |
| `Failed to load ONNX Runtime from …` | Wrong arch or corrupt dylib | Reinstall; confirm `file "$ORT_DYLIB_PATH"` matches your CPU |
| `ONNX model not found at …` | Model not cached | Set `SIMILARITY_MAP_MODEL_DIR` or download `.onnx` (see above) |
| PyO3 `import similarity_core` fails | Extension not built | Run `maturin develop` in `similarity-core-py/`; the external RF monorepo also has a build script |
| `test_embedder=True` works, production fails | Missing ORT or model | Expected — offline tests skip native deps |

## Documentation

- [`CURRENT-STATE.md`](./CURRENT-STATE.md) — verified utility, test results, and known gaps
- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — crate boundaries and data flows
- [Integration contract](./.kiro/specs/similarity-map/integration-contract.md) —
  canonical `AnalysisOutput` v1 shape and merge rules
- [`AGENTS.md`](./AGENTS.md) — contributor and automation environment guide
- [Historical design specification](./Similarity%20Map%20-%20Design%20Specification.md) —
  original desktop intent, including deferred behavior


## License

MIT — see [LICENSE](./LICENSE).
