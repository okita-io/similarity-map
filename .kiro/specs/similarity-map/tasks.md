# Implementation Plan: Similarity Map

## Overview

This plan implements the Similarity Map Tauri 2 desktop application in dependency order: project scaffolding → storage layer → pipeline stages (import → windowing → embedding → clustering → mapping → rasterization) → frontend grid → session management → display settings → interactions. Each task is a self-contained unit that builds on previous work. Property-based tests use the `proptest` crate and are placed alongside the components they validate.

## Tasks

- [x] 1. Project scaffolding and core types
  - [x] 1.1 Initialize Tauri 2 project with Rust backend and Vanilla JS frontend
    - Create Tauri 2 app with `cargo-tauri` scaffolding
    - Configure `Cargo.toml` with dependencies: `tauri`, `serde`, `serde_json`, `uuid`, `sha2`, `tokio`, `lancedb`, `ort`, `proptest` (dev)
    - Set up frontend directory with `index.html`, `main.js`, `style.css`
    - Configure Tauri permissions for file system access and event emission
    - _Requirements: All (foundational)_

  - [x] 1.2 Define core Rust types and enums
    - Implement `Page`, `PaginationMode`, `Window`, `PageSubGrid`, `SubCell`, `SubCellCluster`
    - Implement `ClusterRegistry`, `ClusterInfo`, `PageCanvas`, `DisplayState`
    - Implement IPC response types: `DocumentSessionState`, `CompleteJobInfo`, `PartialJobInfo`
    - Implement `AnalysisHandle`, `AnalysisEstimate`, `ModelStatus`, `CancelResult`, `RestoreHandle`, `SubCellDetail`, `WindowMatch`
    - Implement `AppError` enum with all error categories from the design
    - Derive `Serialize`/`Deserialize` for all IPC-facing types
    - _Requirements: All (foundational types)_

  - [x] 1.3 Define Tauri command stubs and event constants
    - Register all 12 Tauri commands as async stubs returning `todo!()`
    - Define event name constants: `similarity-map:progress`, `similarity-map:page-ready`, `similarity-map:model-download-progress`, `similarity-map:model-ready`
    - Wire command registration in `main.rs`
    - _Requirements: All (IPC surface)_


- [x] 2. LanceDB storage layer
  - [x] 2.1 Implement LanceDB schema and connection management
    - Create `storage` module with LanceDB connection pool
    - Define `windows` table schema (window_id, job_id, window_index, page, char_start, char_end, doc_char_start, text, embedding, cluster_id, hdbscan_label, sim_to_centroid, sub_cell_row, sub_cell_col)
    - Define `pages` table schema (job_id, page, doc_char_start, doc_char_end, char_count, token_count, pagination_mode)
    - Define `jobs` table schema (job_id, document_path, document_hash, settings_hash, window_size, stride, tokens_per_page, pagination_mode, min_repetitions, min_samples, chapter_break_re, windows_total, windows_committed, status, created_at, updated_at)
    - Implement table creation and migration logic
    - _Requirements: 26.1, 26.2, 26.3_

  - [x] 2.2 Implement storage CRUD operations
    - Implement `insert_job`, `update_job_status`, `get_jobs_for_document`
    - Implement `batch_insert_windows`, `get_windows_for_job`, `get_window_count`
    - Implement `insert_pages`, `get_pages_for_job`
    - Implement `delete_job_data` (cascading delete of windows, pages, job record)
    - Implement `get_embeddings_for_job` (vector retrieval for clustering)
    - _Requirements: 26.1, 20.3, 21.4, 22.5_

  - [x] 2.3 Implement hash utilities for settings and document content
    - Implement `compute_document_hash(path) -> SHA-256` reading file contents
    - Implement `compute_settings_hash(window_size, stride, tokens_per_page, min_repetitions, min_samples) -> SHA-256` with deterministic serialization
    - _Requirements: 26.2, 26.3, 21.5_


  - [ ]* 2.4 Write property test for hash determinism
    - **Property 16: Settings and document hash determinism**
    - Verify identical parameters always produce the same settings_hash
    - Verify any parameter change produces a different settings_hash
    - Verify identical file content always produces the same document_hash
    - **Validates: Requirements 26.2, 26.3, 21.5**

- [x] 3. Importer / Paginator
  - [x] 3.1 Implement plain-text token-count pagination
    - Implement whitespace tokenization and page splitting at `tokens_per_page` boundary
    - Track character-accurate `doc_char_start` and `doc_char_end` for each page
    - Handle final short page without padding
    - Handle empty/whitespace-only files with error
    - _Requirements: 2.1, 2.2, 2.3, 2.5_

  - [ ]* 3.2 Write property test for pagination round-trip
    - **Property 1: Pagination character span round-trip**
    - Generate random Unicode text (1–50,000 chars) × tokens_per_page (200–2000)
    - Verify concatenating all page spans reproduces original document exactly
    - **Validates: Requirements 2.1, 2.2, 2.3**

  - [x] 3.3 Implement chapter break pagination
    - Implement regex-based chapter boundary detection
    - Apply tokens_per_page as maximum cap, splitting oversized chapters
    - Include matching line as first content of new page
    - Fall back to token-count pagination when regex is blank
    - Validate regex syntax and return error for invalid patterns
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

  - [ ]* 3.4 Write property test for chapter break max page size
    - **Property 2: Chapter break pagination respects max page size**
    - Generate text with random chapter markers × tokens_per_page
    - Verify every page contains at most `tokens_per_page` tokens
    - **Validates: Requirements 3.1, 3.2**


  - [x] 3.5 Implement PDF import
    - Integrate `pdf-extract` crate for text extraction per PDF page
    - Create one Page per PDF page preserving natural boundaries
    - Handle corrupt/unreadable PDFs with error message
    - Handle pages with no extractable text (empty Page, excluded from windowing)
    - Enforce 300-page maximum with warning
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6_

- [x] 4. Text Window Engine
  - [x] 4.1 Implement sliding window generation
    - Implement overlapping window generation with configurable window_size and stride
    - Record page-relative char_start and char_end for each window
    - Prevent windows from crossing page boundaries
    - Include terminal windows ≥ 3 tokens; discard segments < 3 tokens
    - Assign sequential zero-based window_index across entire job
    - Skip pages with fewer than 3 tokens
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 4.6, 4.7_

  - [ ]* 4.2 Write property test for window generation invariants
    - **Property 3: Window generation invariants**
    - Generate random page text × window_size (5–1500) × stride (1–200)
    - Verify: (a) non-terminal windows have exactly window_size tokens
    - Verify: (b) page_text[char_start..char_end] reproduces window text
    - Verify: (c) no char_end exceeds page char count
    - Verify: (d) window_index forms contiguous sequence from 0
    - **Validates: Requirements 4.1, 4.2, 4.3, 4.4, 4.6**

  - [x] 4.3 Implement window count estimation
    - Implement `estimate_window_count(total_tokens, window_size, stride) -> u32`
    - Formula: `floor((total_tokens - window_size) / stride) + 1`
    - Handle edge case where total_tokens <= window_size
    - _Requirements: 6.2_


  - [ ]* 4.4 Write property test for time estimate formula
    - **Property 6: Time estimate formula**
    - Generate random (window_count, throughput) pairs
    - Verify estimated time = window_count / benchmark_windows_per_sec
    - Verify window count estimate matches formula
    - **Validates: Requirements 6.2, 6.3**

- [x] 5. Checkpoint — Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 6. Embedding Engine
  - [x] 6.1 Implement ONNX model management
    - Integrate `ort` crate for ONNX Runtime
    - Implement `ensure_embedding_model` command: verify model presence and SHA-256 hash
    - Implement model download from Hugging Face with progress events
    - Cache model in app data directory
    - Handle corrupt model (delete and re-download)
    - Handle download failure with retry mechanism
    - _Requirements: 27.1, 27.2, 27.3, 27.4, 27.5, 5.2, 5.3_

  - [x] 6.2 Implement batch embedding pipeline
    - Load all-MiniLM-L6-v2 ONNX model via `ort`
    - Process windows in batches of 32
    - Truncate inputs exceeding 256 tokens before inference
    - L2-normalize output vectors to unit length (384-dim float32)
    - Commit each batch to LanceDB immediately after inference
    - Skip failed windows, log window_index, continue processing
    - Emit progress events after each batch
    - _Requirements: 5.1, 5.4, 5.5, 5.6, 5.7_

  - [ ]* 6.3 Write property test for embedding dimensions and normalization
    - **Property 4: Embedding produces unit-length 384-dimensional vectors**
    - Generate random text strings (1–2000 tokens)
    - Verify output is 384-dim float32 with L2 norm within 1e-5 of 1.0
    - **Validates: Requirements 5.1, 5.6**

  - [ ]* 6.4 Write property test for batch commit pattern
    - **Property 5: Embedding batch commit pattern**
    - Generate random window counts (1–1000)
    - Verify batches of exactly 32 (final batch = N mod 32)
    - Verify total committed count equals N
    - **Validates: Requirements 5.4**


  - [x] 6.5 Implement benchmark probe and time estimation
    - Embed fixed 128-window probe on first launch
    - Record throughput (windows/sec) in app data
    - Implement `estimate_analysis` command using benchmark rate × window count
    - Display "estimate unavailable" if benchmark fails
    - Cache benchmark result, update when model changes
    - _Requirements: 6.1, 6.3, 6.4, 6.5, 6.6_

- [x] 7. HDBSCAN Clustering
  - [x] 7.1 Implement HDBSCAN clustering
    - Integrate HDBSCAN Rust implementation (or Python FFI)
    - Accept min_repetitions (2–20) and min_samples (1–10) parameters
    - Derive min_cluster_size: `min_reps × max(1, floor(phrase_length / stride))`
    - Assign cluster labels (≥0) or noise (-1) per window
    - Handle no-clusters-found case with user-facing message
    - Validate parameter ranges, reject invalid values
    - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 7.6, 7.7_

  - [ ]* 7.2 Write property test for min_cluster_size derivation
    - **Property 7: HDBSCAN min_cluster_size derivation**
    - Generate random (min_repetitions, phrase_length, stride) triples
    - Verify derived value = min_repetitions × max(1, floor(phrase_length / stride))
    - **Validates: Requirements 7.3**

- [x] 8. KMeans Stabilization
  - [x] 8.1 Implement KMeans stabilizer
    - Run KMeans on non-noise windows with k = number of HDBSCAN clusters with ≥3 members
    - Use fixed random seed and process in window_index order
    - Assign one stable integer ID per cluster without merging
    - Skip processing if zero non-noise clusters
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5_

  - [ ]* 8.2 Write property test for KMeans determinism
    - **Property 8: KMeans stabilization determinism and completeness**
    - Generate random embedding sets with known cluster structure
    - Verify: (a) exactly k distinct cluster IDs produced
    - Verify: (b) identical assignments on repeated runs
    - Verify: (c) no cluster merging
    - **Validates: Requirements 8.1, 8.2, 8.3, 8.4**


- [x] 9. Centroid Computation
  - [x] 9.1 Implement centroid computation and cluster registry
    - Compute element-wise mean of member embeddings per cluster
    - Identify most_central_window_id (highest cosine sim to centroid, ties broken by lowest window_index)
    - Build cluster-to-pages index (sorted page numbers)
    - Store centroid, most_central_window_text, member_count
    - Handle zero-magnitude centroid edge case
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5_

  - [ ]* 9.2 Write property test for centroid correctness
    - **Property 9: Centroid computation correctness**
    - Generate random 384-dim vectors grouped into clusters (N ≥ 3)
    - Verify centroid = element-wise mean of members
    - Verify most_central_window_id has highest cosine sim (ties by lowest index)
    - Verify cluster-to-pages = sorted set of member page numbers
    - **Validates: Requirements 9.1, 9.2, 9.4**

- [x] 10. Sub-Cell Mapper
  - [x] 10.1 Implement sub-cell position mapping
    - Compute midpoint = floor((char_start + char_end) / 2)
    - Compute linear_index = clamp(floor(midpoint / page_char_count × 400), 0, 399)
    - Derive row = linear_index / 20, col = linear_index % 20
    - Exclude noise windows (cluster_id = -1)
    - Build PageSubGrid per page with cluster lists sorted by sim_to_centroid desc, capped at 8
    - _Requirements: 10.1, 10.2, 10.3, 10.4, 10.5_

  - [ ]* 10.2 Write property test for sub-cell mapping
    - **Property 10: Sub-cell mapping correctness**
    - Generate random (char_start, char_end, page_char_count) triples
    - Verify position formula produces correct row/col
    - Verify noise windows excluded
    - Verify cluster list sorted desc by sim_to_centroid, capped at 8
    - **Validates: Requirements 10.1, 10.2, 10.3, 10.4, 10.5**

- [x] 11. Checkpoint — Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.


- [x] 12. HSV Color Mapper
  - [x] 12.1 Implement HSV color encoding
    - Implement golden-ratio hue assignment: `(cluster_id × 0.6180339887) mod 1.0`
    - Fix saturation at 1.0 for all clustered windows
    - Compute value: `max(0.0, sim_to_centroid) ^ gamma`
    - Implement HSV to linear RGB conversion
    - Implement linear RGB to sRGB conversion
    - Set alpha = 1.0 for valid clusters, alpha = 0 for noise/empty
    - _Requirements: 11.1, 11.2, 11.3, 11.4, 11.5, 11.6_

  - [ ]* 12.2 Write property test for HSV color encoding
    - **Property 11: HSV color encoding rules**
    - Generate random (cluster_id, sim_to_centroid, gamma) triples
    - Verify hue formula, saturation = 1.0, value formula
    - Verify alpha = 1.0 for valid clusters, alpha = 0 for noise/empty
    - **Validates: Requirements 11.1, 11.2, 11.3, 11.4, 11.5, 11.6**

- [x] 13. Canvas Rasterizer
  - [x] 13.1 Implement similarity-weighted color blending
    - Implement `blend_sub_cell` function for single and multi-cluster sub-cells
    - Apply weights as `sim_to_centroid ^ gamma`, normalize to sum 1.0
    - Blend in linear RGB space, convert result to sRGB
    - Cap at 8 clusters per sub-cell (sorted by sim desc)
    - Render transparent for empty/below-threshold/all-hidden sub-cells
    - _Requirements: 12.1, 12.2, 12.3, 12.5_

  - [ ]* 13.2 Write property test for canvas rasterization color blending
    - **Property 12: Canvas rasterization color blending**
    - Generate random sub-cell cluster lists (1–12 entries) × gamma × threshold
    - Verify single cluster = direct HSV→sRGB
    - Verify multi-cluster = similarity-weighted linear-RGB blend
    - Verify below-threshold/hidden = transparent
    - Verify output is exactly 1600 bytes
    - **Validates: Requirements 12.1, 12.2, 12.3, 13.1, 13.3**

  - [x] 13.3 Implement page canvas rasterization loop
    - Implement `rasterize_page` iterating 20×20 grid
    - Apply threshold, gamma, and hidden_clusters filtering
    - Produce 1600-byte RGBA array (row-major)
    - Emit `similarity-map:page-ready` event with base64-encoded canvas
    - _Requirements: 13.1, 13.2, 13.3, 13.4_


  - [x] 13.4 Implement `raster_pages` command for targeted re-rasterization
    - Accept job_id, page list, threshold, gamma, hidden_clusters
    - Re-raster only specified pages from stored sub-grid data
    - Return Vec<PageCanvas> for affected pages
    - _Requirements: 17.2, 17.3, 18.2, 29.2, 29.3_

- [x] 14. Pipeline orchestration and analyze_document command
  - [x] 14.1 Implement full pipeline orchestration
    - Wire stages: Import → Window → Embed → HDBSCAN → KMeans → Centroid → SubCell → Color → Raster
    - Implement `analyze_document` command invoking full pipeline
    - Emit progress events at each stage transition and after each embedding batch
    - Stream `page-ready` events as pages complete rasterization
    - Compute rolling ETA from sliding window of last 50 batch durations
    - _Requirements: 19.1, 19.2, 19.3, 19.4, 30.1_

  - [x] 14.2 Implement cancellation support
    - Implement `cancel_analysis` command stopping at next batch boundary
    - Commit all completed batches before stopping
    - Set job status to "partial" (≥1 batch) or "discarded" (0 batches)
    - Record accurate windows_committed count
    - _Requirements: 20.1, 20.2, 20.3, 20.4, 20.5, 20.6_

  - [ ]* 14.3 Write property test for cancellation invariants
    - **Property 17: Cancellation preserves committed work**
    - Simulate cancellation at random points during embedding
    - Verify committed batches remain intact in storage
    - Verify job status is "partial" or "discarded" correctly
    - Verify windows_committed count is accurate
    - **Validates: Requirements 20.3, 20.4**

  - [x] 14.4 Implement resume support
    - Implement `resume_analysis` command
    - Skip windows with window_index < windows_committed
    - Continue embedding from windows_committed onward
    - Run full clustering + rasterization on completion
    - Report progress as (current - M) / (N - M)
    - Auto-discard on document_hash mismatch
    - _Requirements: 21.1, 21.2, 21.3, 21.4, 21.5, 21.6_


  - [ ]* 14.5 Write property test for resume correctness
    - **Property 18: Resume skips completed work**
    - Generate random partial job states (M committed out of N total)
    - Verify only windows with index ≥ M are embedded
    - Verify progress = (current - M) / (N - M)
    - **Validates: Requirements 21.2, 21.6**

- [x] 15. Checkpoint — Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 16. Session management commands
  - [x] 16.1 Implement check_document_session command
    - Query jobs table for complete and partial jobs matching document path
    - Return DocumentSessionState with complete_job and partial_job info
    - Check document_hash for edit detection
    - _Requirements: 22.1, 22.2, 26.4, 26.5_

  - [x] 16.2 Implement restore_session command
    - Load embeddings and cluster data from LanceDB
    - Re-run rasterization pipeline (SubCell → Color → Raster)
    - Stream page-ready events as pages complete
    - Emit progress events with stage "rasterizing"
    - _Requirements: 22.3, 22.4, 26.4_

  - [x] 16.3 Implement discard_job command
    - Delete windows, pages, and job record from LanceDB
    - Delete associated display state JSON file
    - _Requirements: 22.5, 23.4_

  - [x] 16.4 Implement display state persistence
    - Write DisplayState to sidecar JSON at `$APPDATA/similarity-map/sessions/<job_id>.json`
    - Implement debounced write (2-second delay after changes)
    - Write on application window close
    - Load display state on session restore, apply defaults if missing/corrupt
    - _Requirements: 23.1, 23.2, 23.3, 23.4, 23.5, 22.4, 22.6_

  - [ ]* 16.5 Write property test for display state round-trip
    - **Property 15: Display state persistence round-trip**
    - Generate random DisplayState structs (valid ranges)
    - Verify serialize → deserialize produces identical state
    - Verify missing/corrupt JSON applies defaults
    - **Validates: Requirements 22.4, 23.1, 22.6**


- [x] 17. Frontend — Grid Renderer
  - [x] 17.1 Implement grid layout and page cell rendering
    - Create 10-column CSS grid layout with configurable gap (0–4px, default 1px)
    - Render each page cell as 20×20 canvas using ImageBitmap
    - Apply `image-rendering: pixelated` at base scale
    - Switch to `image-rendering: auto` when cell ≥ 100px on longest dimension
    - Handle progressive population via page-ready events
    - Display unprocessed positions as transparent cells
    - _Requirements: 14.1, 14.2, 14.3, 14.4, 14.5, 14.6, 30.2, 30.3_

  - [x] 17.2 Implement tolerance alpha mask (frontend-only)
    - Implement per-page 1-bit alpha mask (400 pixels per page)
    - Set pixel alpha = 1 if highest sim_to_centroid in sub-cell > tolerance, else 0
    - Update mask on slider drag with no backend IPC
    - Target < 16ms for 300 pages
    - _Requirements: 16.1, 16.2, 16.3, 16.4, 29.1_

  - [ ]* 17.3 Write property test for tolerance mask correctness
    - **Property 13: Tolerance mask correctness**
    - Generate random sub-cell data × tolerance values (0.75–1.00)
    - Verify pixel visible iff highest sim_to_centroid > tolerance
    - Verify no backend IPC required
    - **Validates: Requirements 16.2, 16.3**

  - [x] 17.4 Implement zoom and CSS scaling
    - Scale grid via CSS transforms (no re-computation on resize)
    - Maintain one ImageBitmap per page (no allocation on zoom/scroll)
    - Release previous ImageBitmap before creating replacement on re-raster
    - _Requirements: 28.1, 28.2, 28.3, 28.4, 29.4_

  - [x] 17.5 Implement spatial dithering at zoom
    - Apply dithering patterns when cell ≥ 100×200px
    - 2 clusters: checkerboard `(px_row + px_col) mod 2`
    - 3 clusters: thirds scatter `(px_row * 3 + px_col * 7) mod 3`
    - 4 clusters: 2×2 quadrant tile `(px_row mod 2) * 2 + (px_col mod 2)`
    - 5–8 clusters: modulo scatter `(px_row * 11 + px_col * 7) mod N`
    - Use weighted blend below 100×200px threshold
    - _Requirements: 12.4, 12.5_


- [x] 18. Frontend — Import Settings Panel
  - [x] 18.1 Implement import settings UI controls
    - Create sliders: Tokens per Page (200–2000), Phrase Length (5–1500), Stride (1–200), Min Repetitions (2–20), Min Samples (1–10)
    - Create Chapter Break regex text field with default `^Chapter\s+\d+`
    - Auto-compute default Stride as `max(1, floor(phrase_length × 0.25))` on Phrase Length change
    - Display live window-count estimate and embedding time (update within 100ms)
    - Show warning when tokens_per_page < 4× phrase_length
    - Show nudge when estimated time > 30 minutes
    - Disable Tokens per Page for PDF imports
    - _Requirements: 15.1, 15.2, 15.3, 15.6, 2.4, 6.4, 1.3_

  - [x] 18.2 Implement progress view and cancellation UI
    - Transition to progress view on Analyze click
    - Lock all controls during analysis
    - Display multi-stage checklist (Paginating, Windowing, Embedding, Clustering, Rasterizing)
    - Show active stage with progress bar, percentage, and rolling ETA
    - Display Cancel import button
    - Restore settings on cancellation
    - Show resume banner for partial jobs with percentage and storage used
    - _Requirements: 15.4, 15.5, 19.3, 20.1, 20.6, 21.1_

- [x] 19. Frontend — Display Settings
  - [x] 19.1 Implement display settings controls
    - Tolerance slider (0.75–1.00, step 0.01, default 0.88)
    - Gamma slider (0.5–3.0, step 0.1, default 1.5)
    - Cluster filter toggles (one per cluster, all enabled by default)
    - Wire Tolerance to frontend-only mask update
    - Wire Cluster Filter to targeted `raster_pages` IPC call
    - Wire Gamma to full `raster_pages` IPC call (all pages)
    - Debounce display state persistence (2 seconds)
    - _Requirements: 16.1, 16.4, 17.1, 18.1, 18.2, 18.3, 23.2_

  - [ ]* 19.2 Write property test for cluster filter targeted re-rasterization
    - **Property 14: Cluster filter targeted re-rasterization**
    - Generate random cluster registries × toggle events
    - Verify re-rasterized pages = exactly pages in cluster-to-pages index
    - Verify all-hidden sub-cells render transparent
    - **Validates: Requirements 17.2, 17.3, 17.4, 29.2**


- [x] 20. Checkpoint — Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 21. Frontend — Detail Panel and Tooltips
  - [x] 21.1 Implement tooltip manager
    - Macro-cell tooltips at base zoom: page number, top 3 clusters, max similarity
    - Sub-cell tooltips at ≥ 5×5 px per sub-cell: position %, cluster name, sim score, excerpt (120 chars)
    - No tooltip for sub-cells/macro-cells with no clusters above Tolerance
    - _Requirements: 24.1, 24.2, 24.3, 24.4_

  - [x] 21.2 Implement detail panel
    - Side panel listing windows above Tolerance, grouped by cluster
    - Show window text excerpt, cluster hue indicator, similarity score, counterpart page links
    - Implement `get_page_detail` command for sub-cell click data
    - Scroll/update panel on new cell click without close/reopen
    - No action for clicks on empty/below-threshold sub-cells
    - _Requirements: 25.1, 25.2, 25.4, 25.5_

  - [x] 21.3 Implement counterpart navigation
    - Scroll grid to target page on counterpart link click
    - Apply 1.5-second pulse animation to target macro-cell
    - Render visible outline on corresponding sub-cell until next click
    - _Requirements: 25.3_

- [x] 22. Frontend — Session dialog and model download UI
  - [x] 22.1 Implement session restore dialog
    - Show dialog on document open when complete session found
    - Display job creation date, page count, phrase length, stride
    - Offer "Restore Session" and "Generate New Map" actions
    - Treat Escape/click-outside as "Generate New Map"
    - _Requirements: 22.1, 22.2, 22.3, 22.5, 22.7_

  - [x] 22.2 Implement model download progress UI
    - Show download progress bar with percentage and bytes
    - Display error with Retry button on download failure
    - Block analysis until model available
    - _Requirements: 27.2, 27.4, 5.5_

- [x] 23. Final checkpoint — Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.


## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation at logical boundaries
- Property tests use the `proptest` crate (Rust) with minimum 100 iterations per property
- Unit tests validate specific examples and edge cases alongside property tests
- The frontend uses Vanilla JS + Canvas 2D with no framework dependencies
- All IPC uses Tauri 2 commands (request/response) and events (streaming)
- LanceDB provides local persistence with no server dependency

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1"] },
    { "id": 1, "tasks": ["1.2", "1.3"] },
    { "id": 2, "tasks": ["2.1"] },
    { "id": 3, "tasks": ["2.2", "2.3"] },
    { "id": 4, "tasks": ["2.4", "3.1"] },
    { "id": 5, "tasks": ["3.2", "3.3", "3.5"] },
    { "id": 6, "tasks": ["3.4", "4.1"] },
    { "id": 7, "tasks": ["4.2", "4.3"] },
    { "id": 8, "tasks": ["4.4", "6.1"] },
    { "id": 9, "tasks": ["6.2", "6.5"] },
    { "id": 10, "tasks": ["6.3", "6.4"] },
    { "id": 11, "tasks": ["7.1"] },
    { "id": 12, "tasks": ["7.2", "8.1"] },
    { "id": 13, "tasks": ["8.2", "9.1"] },
    { "id": 14, "tasks": ["9.2", "10.1"] },
    { "id": 15, "tasks": ["10.2", "12.1"] },
    { "id": 16, "tasks": ["12.2", "13.1"] },
    { "id": 17, "tasks": ["13.2", "13.3"] },
    { "id": 18, "tasks": ["13.4", "14.1"] },
    { "id": 19, "tasks": ["14.2", "14.4"] },
    { "id": 20, "tasks": ["14.3", "14.5"] },
    { "id": 21, "tasks": ["16.1", "16.4"] },
    { "id": 22, "tasks": ["16.2", "16.3", "16.5"] },
    { "id": 23, "tasks": ["17.1"] },
    { "id": 24, "tasks": ["17.2", "17.4"] },
    { "id": 25, "tasks": ["17.3", "17.5", "18.1"] },
    { "id": 26, "tasks": ["18.2", "19.1"] },
    { "id": 27, "tasks": ["19.2", "21.1"] },
    { "id": 28, "tasks": ["21.2", "21.3"] },
    { "id": 29, "tasks": ["22.1", "22.2"] }
  ]
}
```
