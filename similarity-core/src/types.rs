use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Core Domain Types ───────────────────────────────────────────────────────

/// A single page from the imported document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    /// 1-based page number
    pub page_num: u32,
    pub text: String,
    pub char_offset_in_doc: u32,
    pub char_count: u32,
    pub token_count: u32,
    pub pagination_mode: PaginationMode,
}

/// How the document was split into pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaginationMode {
    Pdf,
    Token,
    Chapter,
}

/// A text window — the atomic unit of comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    /// UUID
    pub window_id: String,
    /// Sequential index within job (0-based)
    pub window_index: u32,
    /// 1-based page number
    pub page: u32,
    /// Character offset from start of page text
    pub char_start: u32,
    /// End of window in page text (exclusive)
    pub char_end: u32,
    /// Character offset from start of full document
    pub doc_char_start: u32,
    /// Raw window text
    pub text: String,
}

/// 20×20 grid of sub-cells for one page.
/// Cells stored as a flat Vec of 400 SubCells in row-major order (index = row * 20 + col).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSubGrid {
    pub page: u32,
    pub cells: Vec<SubCell>,
}

impl PageSubGrid {
    /// Number of rows and columns in the grid.
    pub const GRID_SIZE: usize = 20;
    /// Total number of cells (20×20 = 400).
    pub const CELL_COUNT: usize = Self::GRID_SIZE * Self::GRID_SIZE;

    /// Create a new empty grid for the given page.
    pub fn new(page: u32) -> Self {
        Self {
            page,
            cells: vec![SubCell::default(); Self::CELL_COUNT],
        }
    }

    /// Access a cell by row and column.
    pub fn cell(&self, row: usize, col: usize) -> &SubCell {
        &self.cells[row * Self::GRID_SIZE + col]
    }

    /// Mutably access a cell by row and column.
    pub fn cell_mut(&mut self, row: usize, col: usize) -> &mut SubCell {
        &mut self.cells[row * Self::GRID_SIZE + col]
    }
}

/// A single sub-cell in the 20×20 page grid.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubCell {
    /// Clusters present, sorted by sim_to_centroid desc. Capped at 8.
    pub clusters: Vec<SubCellCluster>,
}

/// A cluster entry within a sub-cell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCellCluster {
    pub cluster_id: i32,
    pub sim_to_centroid: f32,
    /// Best-matching window for tooltip lookup
    pub window_id: String,
}

// ─── Cluster Types ───────────────────────────────────────────────────────────

/// Cluster metadata registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRegistry {
    pub clusters: HashMap<i32, ClusterInfo>,
}

/// Metadata for a single cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterInfo {
    pub cluster_id: i32,
    /// Golden-ratio assigned: (id × 0.6180339887) mod 1.0
    pub hue: f32,
    /// Mean of member embeddings (384-dim)
    pub centroid: Vec<f32>,
    /// Highest cosine sim to centroid
    pub most_central_window_id: String,
    pub most_central_window_text: String,
    pub member_count: u32,
    /// Distinct repetition instances after merging overlapping windows.
    pub instance_count: u32,
    /// Sorted page numbers where cluster appears
    pub pages: Vec<u32>,
}

/// A re-rasterized page canvas returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRasterPayload {
    pub page: u32,
    pub canvas_rgba_b64: String,
}

// ─── Display Types ───────────────────────────────────────────────────────────

/// Flat RGBA pixel array for a single page cell.
/// Pixels stored as Vec<u8> of length 1600 (20×20×4 RGBA, row-major).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageCanvas {
    pub page: u32,
    /// 20×20×4 RGBA, row-major (1600 bytes)
    pub pixels: Vec<u8>,
}

impl PageCanvas {
    /// Expected byte length of the pixel buffer (20×20×4 = 1600).
    pub const PIXEL_BYTE_LEN: usize = 20 * 20 * 4;

    /// Create a new canvas with transparent pixels.
    pub fn new(page: u32) -> Self {
        Self {
            page,
            pixels: vec![0u8; Self::PIXEL_BYTE_LEN],
        }
    }
}

/// Display state persisted as sidecar JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayState {
    pub job_id: String,
    /// 0.75–1.00, default 0.88
    pub tolerance: f32,
    /// 0.5–3.0, default 1.5
    pub gamma: f32,
    pub hidden_clusters: Vec<i32>,
    /// Default 1.0
    pub zoom: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    /// ISO 8601 timestamp
    pub saved_at: String,
}

impl Default for DisplayState {
    fn default() -> Self {
        Self {
            job_id: String::new(),
            tolerance: 0.88,
            gamma: 1.5,
            hidden_clusters: Vec::new(),
            zoom: 1.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            saved_at: String::new(),
        }
    }
}

// ─── IPC Response Types ──────────────────────────────────────────────────────

/// Response from check_document_session command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSessionState {
    pub complete_job: Option<CompleteJobInfo>,
    pub partial_job: Option<PartialJobInfo>,
}

/// Info about a fully completed analysis job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteJobInfo {
    pub job_id: String,
    pub created_at: String,
    pub page_count: u32,
    pub window_size: u32,
    pub stride: u32,
    pub tokens_per_page: Option<u32>,
    pub pagination_mode: String,
}

/// Info about a partially completed (cancelled/interrupted) job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialJobInfo {
    pub job_id: String,
    pub windows_committed: u32,
    pub windows_total: u32,
    pub pct: f32,
    pub cancelled_at: String,
    pub window_size: u32,
    pub stride: u32,
    pub tokens_per_page: Option<u32>,
}

/// Handle returned when starting or resuming analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisHandle {
    pub job_id: String,
    pub page_count: u32,
    pub window_count: u32,
    pub pagination_mode: String,
}

/// Pre-analysis cost estimate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisEstimate {
    pub page_count: u32,
    pub window_count: u32,
    pub eta_seconds: f32,
    pub benchmark_windows_per_sec: f32,
}

/// Status of the embedding model on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStatus {
    pub present: bool,
    pub path: String,
    pub size_mb: f32,
}

/// Result of cancelling an in-progress analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelResult {
    pub windows_committed: u32,
    /// "partial" or "discarded"
    pub status: String,
}

/// Handle returned when restoring a session from LanceDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreHandle {
    pub job_id: String,
    pub page_count: u32,
    pub display_state: DisplayState,
}

/// Detail data for a specific sub-cell click.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCellDetail {
    pub window_text: String,
    pub cluster_id: i32,
    pub similarity: f32,
    pub matches: Vec<WindowMatch>,
}

/// A matching window found in another location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowMatch {
    pub page: u32,
    pub window_text: String,
    pub similarity: f32,
    pub sub_cell_row: u8,
    pub sub_cell_col: u8,
}

// ─── Error Types ─────────────────────────────────────────────────────────────

/// Application-wide error type covering all error categories from the design.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "category", content = "detail")]
pub enum AppError {
    /// Corrupt PDF, empty file, unreadable file
    Import(ImportError),
    /// Invalid regex, out-of-range parameters
    Validation(ValidationError),
    /// Missing model, corrupt model, download failure
    Model(ModelError),
    /// Individual window embedding failure
    Embedding(EmbeddingError),
    /// All windows assigned as noise
    Clustering(ClusteringError),
    /// LanceDB write failure, disk full
    Storage(StorageError),
    /// Missing display state JSON, corrupt session data
    Session(SessionError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportError {
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelError {
    pub message: String,
    pub recoverable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingError {
    pub message: String,
    pub window_indices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteringError {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageError {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionError {
    pub message: String,
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Import(e) => write!(f, "Import error: {}", e.message),
            AppError::Validation(e) => {
                write!(f, "Validation error ({}): {}", e.field, e.message)
            }
            AppError::Model(e) => write!(f, "Model error: {}", e.message),
            AppError::Embedding(e) => write!(f, "Embedding error: {}", e.message),
            AppError::Clustering(e) => write!(f, "Clustering error: {}", e.message),
            AppError::Storage(e) => write!(f, "Storage error: {}", e.message),
            AppError::Session(e) => write!(f, "Session error: {}", e.message),
        }
    }
}

impl std::error::Error for AppError {}
