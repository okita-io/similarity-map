use tauri::{Emitter, Manager};

use std::collections::HashSet;

use crate::benchmark;
use crate::cancellation;
use crate::importer;
use crate::model;
use crate::ort_runtime;
use crate::pipeline::{self, PipelineConfig};
use crate::types::*;
use crate::windowing;

// === Session Management ===

/// Called on document open. Returns existing sessions for the file.
#[tauri::command]
pub async fn check_document_session(
    app_handle: tauri::AppHandle,
    path: String,
) -> Result<DocumentSessionState, AppError> {
    use crate::hash::compute_document_hash;
    use crate::storage::Storage;

    // 1. Open storage
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| {
            AppError::Storage(crate::types::StorageError {
                message: format!("Failed to resolve app data directory: {}", e),
            })
        })?;
    let db_path = app_data_dir.join("similarity_map_db");
    let store = Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    // 2. Query jobs for this document path
    let batches = store.get_jobs_for_document(&path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to query jobs: {}", e),
        })
    })?;

    // 3. Parse RecordBatch results into JobRecords
    let jobs = Storage::parse_job_records(&batches);

    // 4. Compute current document hash for edit detection
    let file_path = std::path::Path::new(&path);
    let current_hash = compute_document_hash(file_path).ok();

    // 5. Find the most recent complete job (with matching document_hash)
    let complete_job = jobs
        .iter()
        .filter(|j| j.status == "complete")
        .filter(|j| {
            // Only include if document_hash matches current file
            match &current_hash {
                Some(hash) => j.document_hash == *hash,
                None => false,
            }
        })
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .map(|j| {
            // page_count is derived from windows_total and settings, but we can
            // query pages table. For now, use a pages query.
            // We'll get page count asynchronously below.
            j.clone()
        });

    // 6. Find the most recent partial job
    let partial_job = jobs
        .iter()
        .filter(|j| j.status == "partial")
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
        .cloned();

    // 7. Build CompleteJobInfo if we have a complete job
    let complete_job_info = if let Some(ref job) = complete_job {
        // Get page count from pages table
        let page_count = match store.get_pages_for_job(&job.job_id).await {
            Ok(page_batches) => {
                page_batches.iter().map(|b| b.num_rows() as u32).sum()
            }
            Err(_) => 0,
        };

        Some(CompleteJobInfo {
            job_id: job.job_id.clone(),
            created_at: job.created_at.clone(),
            page_count,
            window_size: job.window_size,
            stride: job.stride,
            tokens_per_page: job.tokens_per_page,
            pagination_mode: job.pagination_mode.clone(),
        })
    } else {
        None
    };

    // 8. Build PartialJobInfo if we have a partial job
    let partial_job_info = if let Some(ref job) = partial_job {
        let pct = if job.windows_total > 0 {
            job.windows_committed as f32 / job.windows_total as f32
        } else {
            0.0
        };

        Some(PartialJobInfo {
            job_id: job.job_id.clone(),
            windows_committed: job.windows_committed,
            windows_total: job.windows_total,
            pct,
            cancelled_at: job.updated_at.clone(),
            window_size: job.window_size,
            stride: job.stride,
            tokens_per_page: job.tokens_per_page,
        })
    } else {
        None
    };

    Ok(DocumentSessionState {
        complete_job: complete_job_info,
        partial_job: partial_job_info,
    })
}

/// Re-rasters from stored LanceDB data. Streams page-ready events.
#[tauri::command]
pub async fn restore_session(
    app_handle: tauri::AppHandle,
    job_id: String,
) -> Result<RestoreHandle, AppError> {
    use crate::display_state::load_display_state;
    use crate::events;
    use crate::job_data::load_job_render_data;
    use crate::rasterizer::{encode_canvas_base64, rasterize_page};

    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| {
            AppError::Storage(crate::types::StorageError {
                message: format!("Failed to resolve app data directory: {}", e),
            })
        })?;

    let db_path = app_data_dir.join("similarity_map_db");
    let store = crate::storage::Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    let render_data = load_job_render_data(&store, &job_id).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to load job render data: {}", e),
        })
    })?;

    let page_sub_grids = render_data.page_sub_grids;
    let page_count = render_data.page_count;

    let display_state = load_display_state(&app_data_dir, &job_id);
    let threshold = display_state.tolerance;
    let gamma = display_state.gamma;
    let hidden: HashSet<i32> = display_state.hidden_clusters.iter().copied().collect();

    let total_pages = page_sub_grids.len();

    for (idx, grid) in page_sub_grids.iter().enumerate() {
        // Emit progress event with stage "rasterizing"
        let pct = if total_pages > 0 {
            (idx as f32) / (total_pages as f32)
        } else {
            1.0
        };
        let _ = app_handle.emit(
            events::PROGRESS,
            serde_json::json!({
                "job_id": job_id,
                "stage": "rasterizing",
                "pct": pct,
                "windows_done": idx as u32,
                "windows_total": total_pages as u32,
                "eta_seconds": 0.0,
            }),
        );

        // Rasterize the page
        let canvas = rasterize_page(grid, gamma, threshold, &hidden);
        let b64 = encode_canvas_base64(&canvas);

        // Emit page-ready event
        let _ = app_handle.emit(
            events::PAGE_READY,
            serde_json::json!({
                "job_id": job_id,
                "page": grid.page,
                "canvas_rgba_b64": b64,
            }),
        );
    }

    // Emit final progress event at 100%
    let _ = app_handle.emit(
        events::PROGRESS,
        serde_json::json!({
            "job_id": job_id,
            "stage": "rasterizing",
            "pct": 1.0,
            "windows_done": total_pages as u32,
            "windows_total": total_pages as u32,
            "eta_seconds": 0.0,
        }),
    );

    // ─── Step 8: Return RestoreHandle ────────────────────────────────────
    Ok(RestoreHandle {
        job_id,
        page_count,
        display_state,
    })
}

/// Deletes all data for a job (windows, job record, display state JSON).
#[tauri::command]
pub async fn discard_job(app_handle: tauri::AppHandle, job_id: String) -> Result<(), AppError> {
    use crate::display_state;
    use crate::storage::Storage;

    // 1. Resolve app data directory
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| {
            AppError::Storage(crate::types::StorageError {
                message: format!("Failed to resolve app data directory: {}", e),
            })
        })?;

    // 2. Open storage and delete windows, pages, and job record
    let db_path = app_data_dir.join("similarity_map_db");
    let store = Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    store.delete_job_data(&job_id).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to delete job data: {}", e),
        })
    })?;

    // 3. Delete the associated display state JSON file
    display_state::delete_display_state(&app_data_dir, &job_id)?;

    Ok(())
}

// === Model Management ===

/// Verifies model presence; triggers download if missing.
#[tauri::command]
pub async fn ensure_embedding_model(app_handle: tauri::AppHandle) -> Result<ModelStatus, AppError> {
    crate::log_info!(app_handle, "command", "ensure_embedding_model invoked");

    if let Err(e) = ort_runtime::ensure_loaded() {
        crate::log_error!(
            app_handle,
            "command",
            "ONNX Runtime not available: {:?}",
            e
        );
        return Err(e);
    }
    crate::log_info!(app_handle, "command", "ONNX Runtime ready");

    // Resolve the app data directory
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| {
            AppError::Model(ModelError {
                message: format!("Failed to resolve app data directory: {}", e),
                recoverable: false,
            })
        })?;
    crate::log_info!(
        app_handle,
        "command",
        "app_data_dir={}",
        app_data_dir.display()
    );

    let emit_handle = app_handle.clone();
    let progress_callback = move |pct: f32, bytes_received: u64, total_bytes: u64| {
        let _ = emit_handle.emit(
            crate::events::MODEL_DOWNLOAD_PROGRESS,
            serde_json::json!({
                "pct": (pct * 100.0).clamp(0.0, 100.0),
                "bytes_received": bytes_received,
                "total_bytes": total_bytes,
            }),
        );
    };

    let app_for_log = app_handle.clone();
    let model_file = match model::ensure_model(&app_data_dir, progress_callback).await {
        Ok(p) => {
            crate::log_info!(app_for_log, "command", "model ready at {}", p.display());
            p
        }
        Err(e) => {
            crate::log_error!(app_for_log, "command", "ensure_model failed: {:?}", e);
            return Err(e);
        }
    };

    // Emit model-ready event
    let _ = app_handle.emit(
        crate::events::MODEL_READY,
        serde_json::json!({
            "path": model_file.to_string_lossy(),
        }),
    );

    // Get file size for status
    let size_bytes = std::fs::metadata(&model_file)
        .map(|m| m.len())
        .unwrap_or(0);
    let size_mb = size_bytes as f32 / (1024.0 * 1024.0);

    Ok(ModelStatus {
        present: true,
        path: model_file.to_string_lossy().to_string(),
        size_mb,
    })
}

// === Analysis ===

/// Returns live estimates without starting analysis.
#[tauri::command]
pub async fn estimate_analysis(
    app_handle: tauri::AppHandle,
    path: String,
    window_size: u32,
    stride: u32,
    tokens_per_page: Option<u32>,
) -> Result<AnalysisEstimate, AppError> {
    let file_path = std::path::Path::new(&path);

    // Import the document to get page data and total token count
    let pages = if file_path
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
    {
        importer::import_pdf(file_path)?
    } else {
        let text = std::fs::read_to_string(file_path).map_err(|e| {
            AppError::Import(crate::types::ImportError {
                message: format!("Failed to read file: {}", e),
                path: Some(path.clone()),
            })
        })?;
        let tpp = tokens_per_page.unwrap_or(400);
        importer::paginate_by_token_count(&text, tpp)?
    };

    let page_count = pages.len() as u32;

    // Compute total tokens across all pages
    let total_tokens: u32 = pages.iter().map(|p| p.token_count).sum();

    // Estimate window count using the formula from the windowing module
    let window_count = windowing::estimate_window_count(total_tokens, window_size, stride);

    // Try to load benchmark result for time estimation
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| {
            AppError::Model(ModelError {
                message: format!("Failed to resolve app data directory: {}", e),
                recoverable: false,
            })
        })?;

    // Load cached benchmark - if unavailable, return estimate with 0 throughput
    // indicating "estimate unavailable"
    let benchmark_result = benchmark::load_cached_benchmark(&app_data_dir);

    let (eta_seconds, benchmark_windows_per_sec) = match benchmark_result {
        Some(b) if b.windows_per_sec > 0.0 => {
            let eta = benchmark::estimate_eta(window_count, b.windows_per_sec);
            (eta, b.windows_per_sec)
        }
        _ => {
            // Benchmark unavailable - return 0 to signal "estimate unavailable"
            (0.0, 0.0)
        }
    };

    Ok(AnalysisEstimate {
        page_count,
        window_count,
        eta_seconds,
        benchmark_windows_per_sec,
    })
}

/// Starts the full pipeline. Streams progress + page-ready events.
#[tauri::command]
pub async fn analyze_document(
    app_handle: tauri::AppHandle,
    path: String,
    window_size: u32,
    stride: u32,
    tokens_per_page: Option<u32>,
    chapter_break_regex: Option<String>,
    min_repetitions: u32,
    min_samples: u32,
    enable_hdbscan: Option<bool>,
    link_subphrases: Option<bool>,
) -> Result<AnalysisHandle, AppError> {
    crate::log_info!(
        app_handle,
        "command",
        "analyze_document invoked: path={} window_size={} stride={}",
        path,
        window_size,
        stride
    );
    let config = PipelineConfig {
        path,
        window_size,
        stride,
        tokens_per_page,
        chapter_break_regex,
        min_repetitions,
        min_samples,
        enable_hdbscan: enable_hdbscan.unwrap_or(true),
        link_subphrases: link_subphrases.unwrap_or(false),
    };

    let app_for_log = app_handle.clone();
    match pipeline::run_pipeline(config, app_handle).await {
        Ok(handle) => {
            crate::log_info!(
                app_for_log,
                "command",
                "analyze_document complete: job_id={} pages={} windows={}",
                handle.job_id,
                handle.page_count,
                handle.window_count
            );
            Ok(handle)
        }
        Err(e) => {
            crate::log_error!(app_for_log, "command", "analyze_document failed: {:?}", e);
            Err(e)
        }
    }
}

/// Stops at next batch boundary, commits completed work.
#[tauri::command]
pub async fn cancel_analysis(app_handle: tauri::AppHandle, job_id: String) -> Result<CancelResult, AppError> {
    let registry = cancellation::global_registry();

    // Trigger cancellation for the job
    let found = registry.cancel(&job_id).await;

    if !found {
        // Job is not currently running — check if it exists in storage
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| {
                AppError::Storage(crate::types::StorageError {
                    message: format!("Failed to resolve app data directory: {}", e),
                })
            })?;

        let db_path = app_data_dir.join("similarity_map_db");
        let store = crate::storage::Storage::open(&db_path).await.map_err(|e| {
            AppError::Storage(crate::types::StorageError {
                message: format!("Failed to open storage: {}", e),
            })
        })?;

        // Try to get the window count for this job to report accurate state
        let windows_committed = store.get_window_count(&job_id).await.unwrap_or(0);
        let status = if windows_committed > 0 {
            "partial".to_string()
        } else {
            "discarded".to_string()
        };

        return Ok(CancelResult {
            windows_committed,
            status,
        });
    }

    // Cancellation was triggered. The pipeline will stop at the next batch boundary.
    // We need to wait briefly for the pipeline to acknowledge and update the job status.
    // The pipeline handles status update itself, but we return an optimistic result.
    // Give the pipeline a moment to finish its current batch and update status.
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Read the current job state from storage
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| {
            AppError::Storage(crate::types::StorageError {
                message: format!("Failed to resolve app data directory: {}", e),
            })
        })?;

    let db_path = app_data_dir.join("similarity_map_db");
    let store = crate::storage::Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;

    let windows_committed = store.get_window_count(&job_id).await.unwrap_or(0);
    let status = if windows_committed > 0 {
        "partial".to_string()
    } else {
        "discarded".to_string()
    };

    Ok(CancelResult {
        windows_committed,
        status,
    })
}

/// Resumes embedding from windows_committed.
#[tauri::command]
pub async fn resume_analysis(
    app_handle: tauri::AppHandle,
    job_id: String,
) -> Result<AnalysisHandle, AppError> {
    pipeline::resume_pipeline(job_id, app_handle).await
}

// === Display ===

/// Targeted re-raster for cluster filter or gamma changes.
#[tauri::command]
pub async fn raster_pages(
    app_handle: tauri::AppHandle,
    job_id: String,
    pages: Vec<u32>,
    threshold: f32,
    gamma: f32,
    hidden_clusters: Vec<i32>,
) -> Result<Vec<PageRasterPayload>, AppError> {
    use crate::job_data::load_job_render_data;
    use crate::rasterizer::{encode_canvas_base64, rasterize_selected_pages};
    use crate::storage::Storage;

    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let db_path = app_data_dir.join("similarity_map_db");
    let store = Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    let render_data = load_job_render_data(&store, &job_id).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to load job render data: {}", e),
        })
    })?;

    let hidden: HashSet<i32> = hidden_clusters.into_iter().collect();
    let canvases = rasterize_selected_pages(
        &render_data.page_sub_grids,
        &pages,
        gamma,
        threshold,
        &hidden,
    );

    Ok(canvases
        .into_iter()
        .map(|canvas| PageRasterPayload {
            page: canvas.page,
            canvas_rgba_b64: encode_canvas_base64(&canvas),
        })
        .collect())
}

/// Returns detail data for a specific sub-cell click.
#[tauri::command]
pub async fn get_page_detail(
    job_id: String,
    page: u32,
    row: u8,
    col: u8,
    threshold: f32,
) -> Result<SubCellDetail, AppError> {
    let _ = (job_id, page, row, col, threshold);
    todo!()
}

/// Returns the cluster registry for a completed job.
#[tauri::command]
pub async fn get_cluster_registry(
    app_handle: tauri::AppHandle,
    job_id: String,
) -> Result<ClusterRegistry, AppError> {
    use crate::centroid::build_cluster_registry;
    use crate::job_data::parse_window_data_from_batches;
    use crate::storage::Storage;

    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let db_path = app_data_dir.join("similarity_map_db");
    let store = Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    let window_batches = store.get_windows_for_job(&job_id).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to load windows: {}", e),
        })
    })?;

    let window_data_list = parse_window_data_from_batches(&window_batches);
    Ok(build_cluster_registry(&window_data_list))
}

/// Persists display state (tolerance, gamma, hidden clusters, zoom, scroll).
#[tauri::command]
pub async fn save_display_state(
    app_handle: tauri::AppHandle,
    state: DisplayState,
) -> Result<(), AppError> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| {
            AppError::Session(crate::types::SessionError {
                message: format!("Failed to resolve app data directory: {}", e),
            })
        })?;

    crate::display_state::save_display_state(&app_data_dir, &state)
}

// === Saved Results Management ===

async fn load_synced_results_catalog(
    app_handle: &tauri::AppHandle,
    document_path: &str,
) -> Result<crate::results_catalog::DocumentResultsCatalog, AppError> {
    use crate::hash::compute_document_hash;
    use crate::results_catalog::{load_catalog, save_catalog, sync_catalog_with_jobs};
    use crate::storage::Storage;
    use std::collections::{HashMap, HashSet};

    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let db_path = app_data_dir.join("similarity_map_db");
    let store = Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    let batches = store.get_jobs_for_document(document_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to query jobs: {}", e),
        })
    })?;
    let jobs = Storage::parse_job_records(&batches);

    let current_hash = compute_document_hash(std::path::Path::new(document_path)).ok();
    let valid_job_ids: HashSet<String> = jobs
        .iter()
        .filter(|job| job.status == "complete")
        .filter(|job| match &current_hash {
            Some(hash) => job.document_hash == *hash,
            None => true,
        })
        .map(|job| job.job_id.clone())
        .collect();

    let mut page_counts = HashMap::new();
    for job_id in &valid_job_ids {
        if let Ok(page_batches) = store.get_pages_for_job(job_id).await {
            let count = page_batches.iter().map(|batch| batch.num_rows() as u32).sum();
            page_counts.insert(job_id.clone(), count);
        }
    }

    let mut catalog = load_catalog(&app_data_dir, document_path);
    catalog.document_path = document_path.to_string();
    sync_catalog_with_jobs(&mut catalog, &jobs, &page_counts, &valid_job_ids);
    save_catalog(&app_data_dir, &catalog)?;

    Ok(catalog)
}

/// List saved analysis results for a document, syncing any completed jobs from storage.
#[tauri::command]
pub async fn list_document_results(
    app_handle: tauri::AppHandle,
    path: String,
) -> Result<crate::results_catalog::DocumentResultsList, AppError> {
    let catalog = load_synced_results_catalog(&app_handle, &path).await?;
    Ok(crate::results_catalog::to_list(&catalog))
}

/// Rename a saved result entry.
#[tauri::command]
pub async fn save_document_result(
    app_handle: tauri::AppHandle,
    path: String,
    result_id: String,
    name: String,
) -> Result<crate::results_catalog::DocumentResultsList, AppError> {
    use crate::results_catalog::{load_catalog, rename_result, save_catalog, to_list};

    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let mut catalog = load_catalog(&app_data_dir, &path);
    rename_result(&mut catalog, &result_id, &name)?;
    save_catalog(&app_data_dir, &catalog)?;
    Ok(to_list(&catalog))
}

/// Save the current analysis under a new result name.
#[tauri::command]
pub async fn save_document_result_as(
    app_handle: tauri::AppHandle,
    path: String,
    job_id: String,
    name: String,
) -> Result<crate::results_catalog::DocumentResultsList, AppError> {
    use crate::results_catalog::{add_result_alias, load_catalog, save_catalog, set_active_result, to_list};
    use crate::storage::Storage;

    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let db_path = app_data_dir.join("similarity_map_db");
    let store = Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    let job = store.get_job_by_id(&job_id).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to load job: {}", e),
        })
    })?;
    let job = job.ok_or_else(|| AppError::Session(crate::types::SessionError {
        message: format!("Job not found: {job_id}"),
    }))?;

    if job.status != "complete" {
        return Err(AppError::Session(crate::types::SessionError {
            message: "Only completed analyses can be saved as results".to_string(),
        }));
    }

    let page_count = store
        .get_pages_for_job(&job_id)
        .await
        .map_err(|e| AppError::Storage(crate::types::StorageError {
            message: format!("Failed to load pages: {}", e),
        }))?
        .iter()
        .map(|batch| batch.num_rows() as u32)
        .sum();

    let mut catalog = load_catalog(&app_data_dir, &path);
    let entry = add_result_alias(&mut catalog, &job, page_count, &name)?;
    set_active_result(&mut catalog, &entry.result_id)?;
    save_catalog(&app_data_dir, &catalog)?;
    Ok(to_list(&catalog))
}

/// Delete a saved result and discard its job data when no aliases remain.
#[tauri::command]
pub async fn delete_document_result(
    app_handle: tauri::AppHandle,
    path: String,
    result_id: String,
) -> Result<crate::results_catalog::DocumentResultsList, AppError> {
    use crate::display_state;
    use crate::results_catalog::{load_catalog, remove_result, save_catalog, to_list};
    use crate::storage::Storage;

    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let mut catalog = load_catalog(&app_data_dir, &path);
    let discard_job_id = remove_result(&mut catalog, &result_id)?;
    save_catalog(&app_data_dir, &catalog)?;

    if let Some(job_id) = discard_job_id {
        let db_path = app_data_dir.join("similarity_map_db");
        let store = Storage::open(&db_path).await.map_err(|e| {
            AppError::Storage(crate::types::StorageError {
                message: format!("Failed to open storage: {}", e),
            })
        })?;
        store.ensure_tables().await.map_err(|e| {
            AppError::Storage(crate::types::StorageError {
                message: format!("Failed to ensure tables: {}", e),
            })
        })?;
        store.delete_job_data(&job_id).await.map_err(|e| {
            AppError::Storage(crate::types::StorageError {
                message: format!("Failed to delete job data: {}", e),
            })
        })?;
        display_state::delete_display_state(&app_data_dir, &job_id)?;
    }

    Ok(to_list(&catalog))
}

/// Mark a saved result as active in the catalog (does not load it).
#[tauri::command]
pub async fn set_active_document_result(
    app_handle: tauri::AppHandle,
    path: String,
    result_id: String,
) -> Result<crate::results_catalog::DocumentResultsList, AppError> {
    use crate::results_catalog::{load_catalog, save_catalog, set_active_result, to_list};

    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let mut catalog = load_catalog(&app_data_dir, &path);
    set_active_result(&mut catalog, &result_id)?;
    save_catalog(&app_data_dir, &catalog)?;
    Ok(to_list(&catalog))
}
