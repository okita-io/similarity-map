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

/// Diagnostic log line emitted to the frontend log panel.
/// Payload: { level: "debug" | "info" | "warn" | "error", source: String, message: String }
pub const LOG: &str = "similarity-map:log";

/// Emit a log line to the frontend log panel.
///
/// Safe to call from any thread; failures are intentionally swallowed so logging
/// never breaks the calling code path.
pub fn emit_log(
    app_handle: &tauri::AppHandle,
    level: &str,
    source: &str,
    message: impl Into<String>,
) {
    use tauri::Emitter;
    let payload = serde_json::json!({
        "level": level,
        "source": source,
        "message": message.into(),
    });
    let _ = app_handle.emit(LOG, payload);
}

#[macro_export]
macro_rules! log_info {
    ($app:expr, $source:expr, $($arg:tt)*) => {
        $crate::events::emit_log(&$app, "info", $source, format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($app:expr, $source:expr, $($arg:tt)*) => {
        $crate::events::emit_log(&$app, "warn", $source, format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($app:expr, $source:expr, $($arg:tt)*) => {
        $crate::events::emit_log(&$app, "error", $source, format!($($arg)*))
    };
}
