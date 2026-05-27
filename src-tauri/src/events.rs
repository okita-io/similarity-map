/// Event emitted during analysis to report pipeline progress.
/// Payload: { job_id, stage, pct, windows_done, windows_total, eta_seconds }
pub const PROGRESS: &str = "similarity-map:progress";

/// Event emitted when a page has been rasterized and is ready for display.
/// Payload: { job_id, page, canvas_rgba_b64 }
pub const PAGE_READY: &str = "similarity-map:page-ready";

/// Event emitted during embedding model download to report progress.
/// Payload: { pct, bytes_received, total_bytes }
pub const MODEL_DOWNLOAD_PROGRESS: &str = "similarity-map:model-download-progress";

/// Event emitted when the embedding model download is complete and ready.
/// Payload: { path }
pub const MODEL_READY: &str = "similarity-map:model-ready";
