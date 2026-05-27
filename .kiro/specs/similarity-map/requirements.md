# Requirements Document

## Introduction

The Similarity Map is a Tauri 2 desktop application (Rust backend + Vanilla JS frontend) that detects and visualizes exact and fuzzy phrase repetition within a long-form manuscript. It renders a portrait-oriented page grid (10 columns × up to 30 rows) where each cell is a 20×20 pixel canvas encoding a sub-grid of repetition clusters. The tool gives authors and editors a single-glance "repetition fingerprint" of an entire book with the ability to drill into any page to see what is repeated, how closely, and where on the page.

## Glossary

- **Similarity_Map**: The overall application and its visual output — an interactive grid visualization of manuscript repetition
- **Page_Cell**: A single 20×20 pixel canvas in the macro-grid representing one page of the document
- **Sub_Cell**: One pixel within a Page_Cell representing approximately 0.25% of a page's text in reading order
- **Macro_Grid**: The 10-column × up to 30-row arrangement of Page_Cells
- **Window**: The atomic unit of comparison — an overlapping text segment of configurable token length carrying character offsets
- **Cluster**: A group of Windows with mutual high cosine similarity, representing a recurring phrase, motif, or structural pattern
- **Cluster_Registry**: An in-memory data structure mapping cluster IDs to hue, centroid vector, most central window, member count, and page list
- **Embedding_Engine**: The ONNX Runtime component running the all-MiniLM-L6-v2 model to convert Windows into 384-dimensional float vectors
- **LanceDB_Store**: The local vector database persisting embeddings, metadata, and cluster assignments between sessions
- **HDBSCAN_Clusterer**: The density-based clustering algorithm that finds natural clusters without a fixed k
- **KMeans_Stabilizer**: The secondary clustering step that assigns stable integer IDs to HDBSCAN-discovered clusters
- **Sub_Cell_Mapper**: The component that maps each Window to a (row, col) position in its page's 20×20 sub-grid
- **HSV_Color_Mapper**: The component that assigns hue (cluster identity), saturation (fixed 1.0), and value (proximity to centroid) to each cluster
- **Canvas_Rasterizer**: The Rust component that produces a 20×20 RGBA pixel array per page from sub-cell cluster data
- **Grid_Renderer**: The frontend component that composites Page_Cell canvases into the Macro_Grid using CSS scaling
- **Import_Settings_Panel**: The UI panel where users configure analysis parameters before committing to a run
- **Display_Settings**: Controls (Tolerance, Cluster Filter, Gamma) that update visualization without re-running the analysis pipeline
- **Job**: A single analysis run (complete or partial) recorded in LanceDB with a unique job_id
- **Tolerance**: A similarity threshold (0.75–1.00) controlling which sub-cells are visible
- **Gamma**: A contrast exponent (0.5–3.0) applied to the value channel of the HSV color encoding
- **Stride**: The number of words the sliding window advances between consecutive Windows
- **Benchmark_Sample**: A fixed 128-window probe embedded at first launch to measure throughput for time estimates

## Requirements

### Requirement 1: Document Import — PDF

**User Story:** As an author, I want to import a PDF manuscript so that the system preserves natural page breaks for analysis.

#### Acceptance Criteria

1. WHEN a PDF file is provided, THE Import_Settings_Panel SHALL extract text from each PDF page and create one Page_Cell per PDF page, maintaining a one-to-one mapping between PDF pages and Page_Cells
2. WHEN a PDF file is imported, THE Import_Settings_Panel SHALL preserve the natural page boundaries exactly as defined in the PDF, with each Page_Cell's text content corresponding to the full extracted text of its source PDF page
3. WHILE a PDF file is loaded, THE Import_Settings_Panel SHALL disable the Tokens per Page slider
4. IF a PDF file cannot be parsed or contains no extractable text on any page, THEN THE Similarity_Map SHALL display an error message identifying the failure reason and return the user to the Import_Settings_Panel without starting analysis
5. WHEN a PDF page yields no extractable text, THE Import_Settings_Panel SHALL create an empty Page_Cell for that page and exclude it from Window generation
6. IF a PDF file contains more than 300 pages, THEN THE Import_Settings_Panel SHALL display a warning indicating only the first 300 pages will be analyzed and truncate the import at 300 Page_Cells

### Requirement 2: Document Import — Plain Text Pagination

**User Story:** As an author, I want to import plain-text manuscripts with configurable page sizes so that I can control the granularity of the analysis grid.

#### Acceptance Criteria

1. WHEN a plain-text file is provided, THE Import_Settings_Panel SHALL split the text into pages using whitespace tokenization at the configured Tokens per Page boundary (range 200–2000, default 400, step 10)
2. WHEN paginating plain text, THE Import_Settings_Panel SHALL preserve character-accurate char_start and char_end fields for each page such that concatenating all page spans reproduces the original file content exactly
3. WHEN the last page contains fewer tokens than the configured Tokens per Page value, THE Import_Settings_Panel SHALL include the shorter final page without padding
4. WHEN Tokens per Page is less than 4 times the configured phrase length, THE Import_Settings_Panel SHALL display a non-blocking inline warning below the Tokens per Page slider indicating sub-cells may be sparsely populated
5. IF a plain-text file is empty or contains only whitespace characters, THEN THE Similarity_Map SHALL display an error message indicating the file contains no analyzable text and SHALL NOT create any pages

### Requirement 3: Document Import — Chapter Break Detection

**User Story:** As an author, I want to use chapter or scene break markers as page boundaries so that the grid aligns with the narrative structure of my manuscript.

#### Acceptance Criteria

1. WHERE a chapter break regex is configured, THE Import_Settings_Panel SHALL use matching lines as page boundaries instead of token-count pagination, including the matching line as the first content of the new page
2. WHERE a chapter break regex is configured, THE Import_Settings_Panel SHALL apply the configured Tokens per Page value as a maximum page size cap, splitting chapters that exceed the cap into additional pages using token-count pagination at the overflow point
3. THE Import_Settings_Panel SHALL provide a default chapter break regex of `^Chapter\s+\d+`
4. WHEN the chapter break regex field is left blank, THE Import_Settings_Panel SHALL fall back to token-count pagination only
5. IF the chapter break regex is syntactically invalid, THEN THE Import_Settings_Panel SHALL display an inline error message indicating the regex is invalid and SHALL prevent analysis from starting until the regex is corrected or cleared

### Requirement 4: Text Windowing

**User Story:** As an author, I want the system to generate overlapping text windows so that phrase repetition at configurable granularity is detected.

#### Acceptance Criteria

1. THE Window engine SHALL generate overlapping text windows of the configured token length (5–1500 tokens) sliding across each page with the configured Stride (1–200 words), where tokens are defined by whitespace splitting
2. THE Window engine SHALL record the page-relative character offset (char_start, char_end) of each Window, where char_start is the zero-based index of the first character and char_end is the index one past the last character of the Window text within its page
3. THE Window engine SHALL prevent Windows from crossing page boundaries
4. WHEN the remaining text on a page is shorter than one full Window, THE Window engine SHALL include the shorter terminal segment as a final Window if it contains at least 3 tokens
5. IF the remaining text on a page is shorter than 3 tokens, THEN THE Window engine SHALL discard that segment and produce no additional Window for that page
6. THE Window engine SHALL assign a zero-based sequential window_index to each Window across the entire Job in page order (page 0 windows first, then page 1, etc.) for resume tracking
7. IF a page contains fewer than 3 tokens total, THEN THE Window engine SHALL produce zero Windows for that page and proceed to the next page

### Requirement 5: Embedding

**User Story:** As an author, I want each text window converted to a dense vector so that semantically similar phrases can be identified by proximity in embedding space.

#### Acceptance Criteria

1. THE Embedding_Engine SHALL convert each Window text into a 384-dimensional float32 vector using the all-MiniLM-L6-v2 ONNX model and L2-normalize each output vector to unit length
2. THE Embedding_Engine SHALL run locally via ONNX Runtime with no network calls after the model is downloaded
3. WHEN the embedding model is not present on first launch, THE Embedding_Engine SHALL auto-download the model (22 MB) from Hugging Face and cache it in the app data directory
4. THE Embedding_Engine SHALL process Windows in batches of 32 and commit each completed batch to the LanceDB_Store
5. WHILE the model download is in progress, THE Embedding_Engine SHALL emit progress events with percentage, bytes received, and total bytes
6. WHEN a Window's token count exceeds the model's maximum sequence length of 256 tokens, THE Embedding_Engine SHALL truncate the input to 256 tokens before embedding
7. IF embedding fails for one or more Windows in a batch, THEN THE Embedding_Engine SHALL skip the failed Windows, log the window_index of each failure, and continue processing the remaining batches

### Requirement 6: Benchmark and Time Estimation

**User Story:** As an author, I want to see estimated processing time before committing to analysis so that I can tune settings to fit my time budget.

#### Acceptance Criteria

1. WHEN the application launches for the first time, THE Embedding_Engine SHALL embed a fixed 128-window probe and record throughput in windows per second
2. THE Import_Settings_Panel SHALL display a live window-count estimate that updates within 100 milliseconds as sliders are adjusted, computed as `floor((total_tokens - window_size) / stride)`
3. THE Import_Settings_Panel SHALL display an estimated embedding time derived from the benchmark throughput multiplied by the projected window count
4. WHEN the estimated time exceeds 30 minutes, THE Import_Settings_Panel SHALL display a nudge suggesting the user increase Stride
5. THE Similarity_Map SHALL cache the benchmark result in app data and update it whenever the model changes
6. IF the benchmark probe fails, THEN THE Import_Settings_Panel SHALL display "estimate unavailable" in place of the time estimate and SHALL NOT block analysis from starting

### Requirement 7: HDBSCAN Clustering

**User Story:** As an author, I want the system to automatically discover natural clusters of repeated content without requiring me to specify the number of clusters.

#### Acceptance Criteria

1. WHEN all embeddings for a Job have been committed to the LanceDB_Store, THE HDBSCAN_Clusterer SHALL cluster all embeddings using density-based clustering with no fixed k
2. THE HDBSCAN_Clusterer SHALL assign Windows that do not meet the minimum cluster density as noise with label -1
3. THE HDBSCAN_Clusterer SHALL accept a Min Repetitions parameter (range 2–20, default 3) converted internally to min_cluster_size via `min_reps × max(1, floor(phrase_length / stride))`
4. THE HDBSCAN_Clusterer SHALL accept a Min Samples parameter (range 1–10, default 3) passed directly as the HDBSCAN min_samples parameter
5. THE HDBSCAN_Clusterer SHALL produce non-negative integer cluster labels for all non-noise Windows
6. IF HDBSCAN assigns all Windows as noise (no clusters found), THEN THE HDBSCAN_Clusterer SHALL report a no-clusters-found outcome to the user indicating that Min Repetitions or Min Samples should be lowered
7. IF a Min Repetitions or Min Samples value outside its valid range is provided, THEN THE HDBSCAN_Clusterer SHALL reject the value and retain the previous valid setting

### Requirement 8: KMeans Stabilization

**User Story:** As an author, I want cluster identities to remain stable between runs so that color assignments are deterministic and recognizable.

#### Acceptance Criteria

1. THE KMeans_Stabilizer SHALL run on all non-noise Windows (hdbscan_label ≠ -1) with k equal to the number of distinct clusters HDBSCAN assigned
2. THE KMeans_Stabilizer SHALL use a fixed random seed and process Windows in window_index order so that identical inputs produce identical cluster_id assignments across runs
3. THE KMeans_Stabilizer SHALL assign one stable integer ID per HDBSCAN cluster without collapsing or merging any clusters, producing exactly k labeled clusters in the output
4. THE KMeans_Stabilizer SHALL only process HDBSCAN clusters with 3 or more member Windows, leaving smaller clusters unlabeled
5. IF HDBSCAN produces zero non-noise clusters, THEN THE KMeans_Stabilizer SHALL skip processing and produce no cluster assignments

### Requirement 9: Centroid Computation

**User Story:** As an author, I want each cluster to have a representative centroid so that similarity scores and display excerpts are meaningful.

#### Acceptance Criteria

1. WHEN KMeans assigns stable IDs, THE Cluster_Registry SHALL compute each cluster centroid as the element-wise mean of all member embeddings, producing one 384-dimensional float32 vector per cluster
2. WHEN a cluster centroid is computed, THE Cluster_Registry SHALL identify the most central window as the member window with the highest cosine similarity to that centroid, breaking ties by lowest window_index
3. THE Cluster_Registry SHALL store the most_central_window_id and its associated window text for use as the display excerpt in tooltips and the detail panel
4. THE Cluster_Registry SHALL store a cluster-to-pages index mapping each cluster_id to the sorted list of page numbers where at least one member window appears
5. IF a cluster's centroid has zero magnitude, THEN THE Cluster_Registry SHALL exclude that cluster from similarity scoring and assign no most_central_window_id

### Requirement 10: Sub-Cell Mapping

**User Story:** As an author, I want each window mapped to a spatial position within its page so that I can see where on the page repetition occurs.

#### Acceptance Criteria

1. THE Sub_Cell_Mapper SHALL compute each Window's sub-cell position by calculating `window_char_midpoint = floor((char_start + char_end) / 2)` and deriving a linear index via `floor(window_char_midpoint / page_char_count × 400)`, clamped to the range 0–399
2. THE Sub_Cell_Mapper SHALL convert the linear index to row and column via `row = floor(index / 20)` and `col = index mod 20`
3. THE Sub_Cell_Mapper SHALL exclude noise Windows (cluster label -1) from sub-cell storage
4. THE Sub_Cell_Mapper SHALL store all non-noise clusters present in each sub-cell sorted by sim_to_centroid descending, capped at 8 entries
5. THE Sub_Cell_Mapper SHALL map sub-cells in reading order: top-left (0,0) corresponds to the beginning of the page and bottom-right (19,19) to the end

### Requirement 11: HSV Color Encoding

**User Story:** As an author, I want clusters encoded with distinct colors where brightness indicates typicality so that I can visually distinguish repetition patterns at a glance.

#### Acceptance Criteria

1. THE HSV_Color_Mapper SHALL assign each cluster a unique hue using the golden-ratio conjugate distribution: `hue = (cluster_id × 0.6180339887) mod 1.0`
2. THE HSV_Color_Mapper SHALL fix saturation at 1.0 for all clustered Windows
3. THE HSV_Color_Mapper SHALL compute value as `max(0.0, cosine_similarity(window_embedding, cluster_centroid)) ^ gamma` where gamma defaults to 1.5, clamping negative similarities to zero
4. THE HSV_Color_Mapper SHALL render noise points (no cluster) as transparent (alpha = 0)
5. THE HSV_Color_Mapper SHALL render sub-cells with no Windows as transparent (alpha = 0, background shows through)
6. THE HSV_Color_Mapper SHALL set alpha to 1.0 for all pixels with a valid cluster color

### Requirement 12: Sub-Cell Color Blending

**User Story:** As an author, I want overlapping clusters in the same sub-cell to blend visually so that I can perceive multi-cluster overlap without losing the dominant signal.

#### Acceptance Criteria

1. WHEN a sub-cell contains a single cluster, THE Canvas_Rasterizer SHALL render the pixel as the cluster's HSV color converted to sRGB (hue from cluster identity, saturation 1.0, value from sim_to_centroid ^ gamma)
2. WHEN a sub-cell contains multiple clusters, THE Canvas_Rasterizer SHALL convert each cluster's HSV color to linear RGB, compute weights as `sim_to_centroid ^ gamma` for each cluster, normalize weights to sum to 1.0, blend the linear-RGB colors by normalized weight, and convert the result to sRGB for display
3. THE Canvas_Rasterizer SHALL cap the number of blended clusters at 8 per sub-cell, sorted by sim_to_centroid descending, discarding any beyond the top 8
4. WHEN the grid is zoomed to 100×200 px or larger per cell, THE Grid_Renderer SHALL apply spatial dithering patterns to sub-cells containing multiple clusters: 2 clusters use a checkerboard pattern `(px_row + px_col) mod 2`, 3 clusters use thirds scatter `(px_row * 3 + px_col * 7) mod 3`, 4 clusters use 2×2 quadrant tile `(px_row mod 2) * 2 + (px_col mod 2)`, and 5–8 clusters use modulo scatter `(px_row * 11 + px_col * 7) mod N`
5. WHEN the grid is zoomed below 100×200 px per cell, THE Grid_Renderer SHALL use the weighted color blend from criterion 2 instead of spatial dithering

### Requirement 13: Canvas Rasterization

**User Story:** As an author, I want each page pre-rendered as a compact pixel array so that the grid displays efficiently with minimal memory usage.

#### Acceptance Criteria

1. THE Canvas_Rasterizer SHALL produce a 20×20 pixel RGBA array (1600 bytes) for each page in row-major order (pixel offset = (row × 20 + col) × 4) after clustering completes
2. THE Canvas_Rasterizer SHALL iterate all 400 pixels, look up the sub-cell cluster list, apply similarity-weighted color blending, and write the RGBA value
3. IF a sub-cell contains no clusters or all clusters in the sub-cell have sim_to_centroid below the current Tolerance threshold, THEN THE Canvas_Rasterizer SHALL render that pixel as transparent (alpha = 0)
4. WHEN a page canvas is ready, THE Canvas_Rasterizer SHALL emit a `similarity-map:page-ready` Tauri event containing the job_id, 1-based page number, and the canvas pixel data as a base64-encoded RGBA string

### Requirement 14: Grid Rendering and Layout

**User Story:** As an author, I want the macro-grid to mirror the shape of a real book so that I can intuitively locate pages and see the overall repetition pattern.

#### Acceptance Criteria

1. THE Grid_Renderer SHALL arrange Page_Cells in a fixed 10-column layout with row count equal to `ceil(page_count / 10)`, up to a maximum of 30 rows (300 pages), filling the final row left-to-right with empty cells for any remainder
2. THE Grid_Renderer SHALL render each Page_Cell as a 20×20 pixel canvas using ImageBitmap compositing
3. WHILE the per-cell rendered size is below 100 px on its longest dimension, THE Grid_Renderer SHALL scale the entire grid via CSS with `image-rendering: pixelated` to keep sub-cell pixels crisp
4. THE Grid_Renderer SHALL apply a configurable cell gap (range 0–4 px, default 1 px) between Page_Cells using the background color
5. WHEN the per-cell rendered size reaches or exceeds 100 px on its longest dimension, THE Grid_Renderer SHALL switch to `image-rendering: auto` (bilinear) for detail viewing
6. IF the document contains more than 300 pages, THEN THE Grid_Renderer SHALL display only the first 300 pages in the grid and indicate the overflow count to the user

### Requirement 15: Import Settings Panel

**User Story:** As an author, I want a settings panel with all analysis parameters and live feedback so that I can tune the analysis before committing to a long run.

#### Acceptance Criteria

1. THE Import_Settings_Panel SHALL expose Tokens per Page (200–2000, default 400), Phrase Length (5–1500, default 20), Stride (1–200, default computed as `max(1, floor(phrase_length × 0.25))`), Min Repetitions (2–20, default 3), Min Samples (1–10, default 3), and Chapter Break regex (default `^Chapter\s+\d+`) as configurable controls
2. THE Import_Settings_Panel SHALL display a live window-count estimate and estimated embedding time that update within 100 milliseconds as any slider is adjusted
3. THE Import_Settings_Panel SHALL recalculate the default Stride as `max(1, floor(phrase_length × 0.25))` whenever Phrase Length changes, unless the user has manually overridden Stride
4. WHILE analysis is running, THE Import_Settings_Panel SHALL lock all controls and display only the progress view and Cancel button
5. WHEN the user clicks Analyze, THE Import_Settings_Panel SHALL transition to the progress view and invoke analyze_document with the current settings
6. IF the Chapter Break regex is syntactically invalid, THEN THE Import_Settings_Panel SHALL display an inline error below the regex field and disable the Analyze button until corrected

### Requirement 16: Display Settings — Tolerance

**User Story:** As an author, I want to adjust a similarity threshold in real time so that I can filter out weaker echoes and focus on strong repetition.

#### Acceptance Criteria

1. THE Grid_Renderer SHALL provide a Tolerance slider with range 0.75–1.00, step 0.01, and default value 0.88
2. WHEN the Tolerance slider is dragged, THE Grid_Renderer SHALL update a per-page alpha mask (1-bit per pixel) without any IPC round-trip to the backend
3. THE Grid_Renderer SHALL set pixel alpha to 1 if the highest-similarity cluster in that sub-cell exceeds the current Tolerance, else 0
4. WHEN the Tolerance slider is released, THE Grid_Renderer SHALL persist the new value to the display state JSON via the debounced write mechanism

### Requirement 17: Display Settings — Cluster Filter

**User Story:** As an author, I want to toggle individual clusters on and off so that I can isolate specific repetition patterns.

#### Acceptance Criteria

1. THE Grid_Renderer SHALL provide a toggle control for each cluster in the Cluster_Registry with all clusters enabled by default
2. WHEN a cluster is toggled off, THE Canvas_Rasterizer SHALL re-raster only the pages containing that cluster using the cluster-to-pages index, excluding that cluster from the color blend computation for affected sub-cells
3. WHEN a cluster is toggled back on, THE Canvas_Rasterizer SHALL re-raster only the pages containing that cluster using the cluster-to-pages index, re-including that cluster in the color blend computation for affected sub-cells
4. WHEN all clusters in a sub-cell are toggled off, THE Canvas_Rasterizer SHALL render that sub-cell as transparent

### Requirement 18: Display Settings — Gamma

**User Story:** As an author, I want to adjust the contrast curve so that I can emphasize or de-emphasize the brightness gradient between strong and weak cluster members.

#### Acceptance Criteria

1. THE Grid_Renderer SHALL provide a Gamma slider with range 0.5–3.0, step increment of 0.1, and default value of 1.5
2. WHEN the Gamma slider is changed, THE Canvas_Rasterizer SHALL re-raster all page canvases applying the updated gamma exponent to both the value channel computation (`V = sim_to_centroid ^ gamma`) and the blend weight computation (`weight = sim_to_centroid ^ gamma`) for multi-cluster sub-cells
3. WHEN the Gamma slider is changed, THE Grid_Renderer SHALL display the updated page canvases immediately after re-rasterization completes without requiring a manual refresh

### Requirement 19: Progress Tracking

**User Story:** As an author, I want to see detailed progress during analysis so that I know how long the process will take and which stage is active.

#### Acceptance Criteria

1. WHEN analysis is running, THE Similarity_Map SHALL emit a progress event after each embedding batch completes, containing stage name, percentage (0.0–1.0), windows done, windows total, and rolling ETA in seconds
2. THE Similarity_Map SHALL compute rolling ETA using a sliding window of the last 50 embedding batch durations; WHEN fewer than 50 batches have completed, THE Similarity_Map SHALL use all available batch durations collected so far
3. WHILE analysis is running, THE Import_Settings_Panel SHALL display progress as a multi-stage checklist with stages Paginating, Windowing, Embedding, Clustering, and Rasterizing — showing completed stages with a done indicator, the active stage with a progress bar and percentage, and pending stages as inactive
4. WHEN a page-ready event is received during analysis, THE Grid_Renderer SHALL draw the page canvas into the grid within the next animation frame so the map fills in progressively

### Requirement 20: Cancellation

**User Story:** As an author, I want to cancel a running analysis so that I can adjust settings without waiting for a long run to complete.

#### Acceptance Criteria

1. WHILE analysis is running, THE Import_Settings_Panel SHALL display a Cancel import button
2. WHEN Cancel import is clicked, THE Similarity_Map SHALL stop processing at the next batch boundary within 2 seconds
3. WHEN cancellation occurs, THE Similarity_Map SHALL commit all completed embedding batches to the LanceDB_Store before stopping
4. WHEN cancellation occurs with at least one committed batch, THE Similarity_Map SHALL mark the Job status as "partial"
5. WHEN cancellation occurs before any embeddings complete, THE Similarity_Map SHALL mark the Job status as "discarded"
6. WHEN cancellation completes, THE Import_Settings_Panel SHALL unlock all controls and restore the import setting values that were configured before the analysis started

### Requirement 21: Resume After Cancellation

**User Story:** As an author, I want to resume a cancelled analysis from where it left off so that I do not lose already-computed embeddings.

#### Acceptance Criteria

1. WHEN a partial Job exists with matching settings_hash and document_hash, THE Import_Settings_Panel SHALL display a resume banner showing percentage complete (windows_committed / windows_total) and storage used in MB
2. WHEN the user clicks Resume, THE Embedding_Engine SHALL skip already-embedded Windows (by window_index), continue embedding from windows_committed, and upon completion run the full clustering and rasterization pipeline to transition the Job status to "complete"
3. WHEN the user changes any import setting after cancellation, THE Import_Settings_Panel SHALL hide the Resume option and show only Start Fresh
4. WHEN Start Fresh is confirmed, THE Similarity_Map SHALL delete the partial Job's windows from LanceDB_Store and mark the Job as "discarded"
5. WHEN the document file has been edited since the partial Job started (document_hash mismatch), THE Similarity_Map SHALL auto-discard the partial Job and display an inline message indicating the document was edited and the partial analysis was discarded
6. WHILE a resumed analysis is in progress, THE Similarity_Map SHALL report progress percentage and ETA based on remaining windows (windows_total minus windows_committed) rather than total windows

### Requirement 22: Session Persistence and Restore

**User Story:** As an author, I want my completed analysis sessions persisted so that I can reopen a manuscript and instantly see the previous similarity map without re-running analysis.

#### Acceptance Criteria

1. WHEN a document is opened, THE Similarity_Map SHALL call check_document_session to detect complete and partial Jobs for that file
2. WHEN a complete Job is found with matching document_hash, THE Similarity_Map SHALL display a session dialog showing the Job creation date, page count, phrase length, and stride, and offering Restore Session or Generate New Map as actions
3. WHEN Restore Session is selected, THE Similarity_Map SHALL re-raster all page canvases from the stored LanceDB_Store data without re-embedding, emitting similarity-map:progress events with stage "rasterizing" and streaming similarity-map:page-ready events as each page completes
4. WHEN Restore Session completes, THE Similarity_Map SHALL load the display state JSON (tolerance, gamma, hidden clusters, zoom, scroll position) and restore the grid to its last viewed state
5. WHEN Generate New Map is selected, THE Similarity_Map SHALL discard all prior Jobs for that file and open the Import_Settings_Panel fresh
6. IF the display state JSON file is missing or unreadable when Restore Session completes, THEN THE Similarity_Map SHALL apply default display settings (tolerance 0.88, gamma 1.5, no hidden clusters, zoom 1.0, scroll position 0,0)
7. IF the session dialog is dismissed without selection (Escape key or click outside), THEN THE Similarity_Map SHALL behave as if Generate New Map was selected

### Requirement 23: Display State Persistence

**User Story:** As an author, I want my display settings (tolerance, gamma, hidden clusters, zoom, scroll) saved automatically so that restoring a session returns me to exactly where I left off.

#### Acceptance Criteria

1. THE Similarity_Map SHALL write display state to a sidecar JSON file at `$APPDATA/similarity-map/sessions/<job_id>.json` containing the current Tolerance value, Gamma value, list of hidden cluster IDs, zoom level, and scroll position (x, y offsets)
2. THE Similarity_Map SHALL debounce display state writes by 2 seconds after any change to Tolerance, Gamma, cluster filter toggles, zoom level, or scroll position
3. WHEN the application window is closed, THE Similarity_Map SHALL write the current display state to the sidecar JSON file before shutdown completes
4. WHEN a Job is discarded, THE Similarity_Map SHALL delete the associated display state JSON file
5. IF the display state JSON file is missing or cannot be parsed on session restore, THEN THE Similarity_Map SHALL apply default display settings (Tolerance 0.88, Gamma 1.5, no hidden clusters, zoom 1.0, scroll position 0,0)

### Requirement 24: Interactive Tooltips

**User Story:** As an author, I want to hover over the grid and see contextual information so that I can quickly identify what repetition exists at any point.

#### Acceptance Criteria

1. WHEN the user hovers on a macro-cell, THE Grid_Renderer SHALL display a tooltip showing the page number, up to 3 clusters with the highest sim_to_centroid values present on that page, and the maximum similarity score among all sub-cells on that page
2. WHEN the user hovers on a sub-cell at a zoom level where individual sub-cells are distinguishable, THE Grid_Renderer SHALL display a tooltip showing the text position as a percentage of the page (derived from sub-cell index), the cluster name, the sim_to_centroid score, and a text excerpt from the most_central_window truncated to 120 characters
3. IF the user hovers on a sub-cell or macro-cell that contains no clusters above the current Tolerance threshold, THEN THE Grid_Renderer SHALL not display a tooltip
4. WHEN the grid is at base scale where each sub-cell is 1 pixel, THE Grid_Renderer SHALL display only the macro-cell tooltip and SHALL switch to sub-cell tooltips when the cell is rendered at 5×5 pixels per sub-cell or larger

### Requirement 25: Detail Panel

**User Story:** As an author, I want to click on a page or sub-cell to see the full list of matching windows so that I can read the repeated text and navigate to counterpart pages.

#### Acceptance Criteria

1. WHEN the user clicks a macro-cell, THE Similarity_Map SHALL open a side panel listing all Windows on that page whose sim_to_centroid exceeds the current Tolerance, grouped by cluster, each entry showing the window text excerpt, cluster hue indicator, similarity score, and links to counterpart pages where the same cluster appears
2. WHEN the user clicks a sub-cell, THE Similarity_Map SHALL open or scroll the side panel to the section for that sub-cell's Window, displaying its text excerpt and the list of same-cluster Windows on other pages with their page number and similarity score
3. WHEN the user clicks a counterpart link in the side panel, THE Grid_Renderer SHALL scroll the grid to bring the target page's macro-cell into view, apply a 1.5-second pulse animation to that macro-cell, and render a visible outline on the corresponding sub-cell that persists until the user clicks elsewhere
4. IF the user clicks a sub-cell that contains no clustered Windows above the current Tolerance, THEN THE Similarity_Map SHALL not open the side panel and SHALL take no action
5. WHEN the side panel is open and the user clicks a different macro-cell or sub-cell, THE Similarity_Map SHALL update the side panel content to reflect the newly clicked cell without closing and reopening the panel

### Requirement 26: LanceDB Persistence

**User Story:** As an author, I want embeddings and cluster data persisted locally so that unchanged documents skip re-embedding on subsequent opens.

#### Acceptance Criteria

1. THE LanceDB_Store SHALL persist all Window embeddings, cluster assignments, sub-cell positions, and Job metadata (including job status and timestamps) between application sessions
2. THE LanceDB_Store SHALL store a settings_hash (SHA-256 of window_size, stride, tokens_per_page, min_repetitions, min_samples) per Job to determine resumability
3. THE LanceDB_Store SHALL store a document_hash (SHA-256 of file contents) per Job to detect file edits
4. WHEN a document is reopened and the most recent complete Job for that file has a matching document_hash and settings_hash, THE Similarity_Map SHALL skip all pipeline stages up to and including clustering, running only the rasterization stage
5. IF a document is reopened and no complete Job exists with matching document_hash and settings_hash, THEN THE Similarity_Map SHALL require a new analysis run via the Import_Settings_Panel

### Requirement 27: Embedding Model Management

**User Story:** As an author, I want the embedding model managed automatically so that the application works offline after initial setup.

#### Acceptance Criteria

1. WHEN the application starts, THE Similarity_Map SHALL call ensure_embedding_model to verify the model file is present and its SHA-256 hash matches the expected value
2. WHEN the model is not present, THE Similarity_Map SHALL download the all-MiniLM-L6-v2 ONNX model (22 MB) from Hugging Face, emitting progress events with percentage, bytes received, and total bytes
3. WHEN the model download completes, THE Similarity_Map SHALL cache the model in the app data directory for fully offline operation thereafter
4. IF the model download fails due to network error, THEN THE Similarity_Map SHALL display an error message with a Retry button and SHALL prevent analysis from starting until the model is available
5. IF the cached model file is corrupt (hash mismatch), THEN THE Similarity_Map SHALL delete the corrupt file and re-download the model

### Requirement 28: Performance — Memory Efficiency

**User Story:** As an author, I want the application to use minimal memory for the grid visualization so that it runs smoothly even for long manuscripts.

#### Acceptance Criteria

1. THE Canvas_Rasterizer SHALL use exactly 1600 bytes (20×20×4 RGBA) per page canvas
2. THE Grid_Renderer SHALL maintain at most one ImageBitmap object per page and SHALL NOT allocate new ImageBitmap objects during zoom, scroll, or tolerance adjustments
3. THE Grid_Renderer SHALL scale the grid via CSS transforms rather than re-rendering at different resolutions
4. WHEN a page canvas is re-rasterized due to a display setting change, THE Grid_Renderer SHALL release the previous ImageBitmap for that page before creating the replacement

### Requirement 29: Performance — Targeted Re-Rasterization

**User Story:** As an author, I want display setting changes to respond quickly so that exploring the visualization feels interactive.

#### Acceptance Criteria

1. WHEN the Tolerance slider is dragged, THE Grid_Renderer SHALL update the alpha mask with no backend IPC (frontend-only 400-pixel scan per page) and complete the update within 16 milliseconds for up to 300 pages
2. WHEN a cluster filter is toggled, THE Canvas_Rasterizer SHALL re-raster only the affected pages identified by the cluster-to-pages index
3. WHEN the Gamma slider is changed, THE Canvas_Rasterizer SHALL re-raster all page canvases (400 pixels per page in a tight loop)
4. WHEN the window is resized, THE Grid_Renderer SHALL apply CSS scaling only with no re-computation

### Requirement 30: Progressive Grid Population

**User Story:** As an author, I want to see the grid fill in as analysis progresses so that I get early visual feedback without waiting for the full run to complete.

#### Acceptance Criteria

1. WHEN a page's clustering and rasterization completes during analysis, THE Similarity_Map SHALL emit a page-ready event with the page number and canvas RGBA data
2. WHEN a page-ready event is received, THE Grid_Renderer SHALL draw the page canvas into the corresponding grid position (left-to-right, top-to-bottom by page number) within 100 milliseconds of event receipt, regardless of the order in which pages complete
3. WHILE analysis is in progress, THE Grid_Renderer SHALL display unprocessed page positions as transparent cells matching the grid background color, with the same dimensions and gap spacing as rendered Page_Cells
