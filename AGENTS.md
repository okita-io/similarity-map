# AGENTS.md

## Cursor Cloud specific instructions

**Similarity Map** is a single **Tauri 2 desktop app** (Rust + vanilla JS). There is no separate backend/frontend/dashboard split and no `npm`/`package.json` workflow.

### Services

| Service | Command | Notes |
|---|---|---|
| **Desktop app (primary)** | `export ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so && cd src-tauri && cargo run` | Or from repo root: `cargo tauri dev` (requires `cargo-tauri`) |
| **Optional frontend hot reload** | `python3 -m http.server 1420 --directory src` | Requires temporary `"devUrl": "http://localhost:1420"` in `src-tauri/tauri.conf.json` |

The app is a GUI on `DISPLAY=:1` in Cloud Agent VMs. Use tmux for long-running dev processes.

### System dependencies (Linux, one-time on VM)

These are **not** in the startup update script; install once if missing:

- **Rust stable** (1.96+): `rustup default stable` — required for `edition2024` transitive deps
- **Tauri CLI**: `cargo install tauri-cli --version "^2" --locked`
- **ONNX Runtime ≥ 1.24.2** (must match `ort` 2.0.0-rc.12): install to `/usr/local/lib` and set `ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so`. **Do not use 1.20.x** — embedding hangs/fails silently with API version mismatch.
- **protoc**: `apt install protobuf-compiler`
- **Tauri Linux libs**: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`, `build-essential`, `pkg-config`, `libssl-dev`, `libgtk-3-dev`

### Environment variables

```bash
export ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so   # or Homebrew path on macOS
export SIMILARITY_MAP_MODEL_DIR=/path/to/models            # headless / CI
export SIMILARITY_MAP_CI=1                                 # enable ONNX integration pytest
```

See README → **Headless pipeline & ONNX Runtime** for install paths and troubleshooting.

Optional build-time: `PROTOC` if `protoc` is not on `PATH`.

### Lint / test / build

See `README.md` for full detail. Common commands from repo root:

```bash
export ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
cd src-tauri && cargo test          # 31 unit tests
cargo clippy --workspace --all-targets
cargo fmt --check                   # may fail if code is unformatted
cd src-tauri && cargo run           # dev desktop app
cargo tauri build                   # release bundle (slow)
```

### Hello-world manual test

1. Start the app (see above).
2. **Open Document** → import `test-data/sample-manuscript.txt`.
3. Use **Phrase Length 5**, **Min Repetitions 2**, **Min Samples 1** for reliable clustering on the sample file.
4. Click **Analyze**; first run downloads `all-MiniLM-L6-v2.onnx` (~23 MB) to `$XDG_DATA_HOME/com.similarity-map.app/models/`.
5. Confirm log panel shows `ONNX session loaded` and embedding completes; text preview shows color-coded repeated phrases.

### Gotchas

- **ONNX version**: `similarity-core` uses `ort` 2.0.0-rc.12, which targets **ONNX Runtime 1.24.x**. Wrong dylib version causes embedding to stall at 0%.
- **Model download**: Requires network on first run; cached afterward under `~/.local/share/com.similarity-map.app/`.
- **Clustering on small samples**: Default phrase length (20) may yield "No clusters found" on the bundled test manuscript; shorten phrase length or lower min samples.
- **No ESLint/TypeScript**: Frontend is plain static JS in `src/`; Rust tests are the primary automated check.
