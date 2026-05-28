use tauri::{Emitter, Manager};

use std::collections::HashSet;

use crate::benchmark;
use crate::cancellation;
use crate::importer;
use crate::model;
use crate::pipeline::{self, PipelineConfig};
use crate::rasterizer;
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
    use crate::centroid::{build_cluster_registry, WindowData};
    use crate::display_state::load_display_state;
    use crate::events;
    use crate::rasterizer::{encode_canvas_base64, rasterize_page};
    use crate::subcell::{build_page_sub_grids, WindowSubCellData};

    use arrow_array::{
        Array, FixedSizeListArray, Float32Array, Int32Array, StringArray, UInt32Array,
    };

    // ─── Step 1: Open storage ────────────────────────────────────────────
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

    // ─── Step 2: Load window data for the job ────────────────────────────
    let window_batches = store.get_windows_for_job(&job_id).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to load windows: {}", e),
        })
    })?;

    // Parse window records from RecordBatches
    let mut window_data_list: Vec<WindowData> = Vec::new();
    let mut subcell_data_list: Vec<WindowSubCellData> = Vec::new();

    for batch in &window_batches {
        if batch.num_rows() == 0 {
            continue;
        }

        let window_ids = batch
            .column_by_name("window_id")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let window_indices = batch
            .column_by_name("window_index")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap();
        let pages_col = batch
            .column_by_name("page")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap();
        let char_starts = batch
            .column_by_name("char_start")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap();
        let char_ends = batch
            .column_by_name("char_end")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap();
        let texts = batch
            .column_by_name("text")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let cluster_ids = batch
            .column_by_name("cluster_id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        let sims = batch
            .column_by_name("sim_to_centroid")
            .unwrap()
            .as_any()
            .downcast_ref::<Float32Array>()
            .unwrap();
        let embeddings_col = batch
            .column_by_name("embedding")
            .unwrap()
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .unwrap();

        for i in 0..batch.num_rows() {
            let window_id = window_ids.value(i).to_string();
            let window_index = window_indices.value(i);
            let page = pages_col.value(i);
            let char_start = char_starts.value(i);
            let char_end = char_ends.value(i);
            let text = texts.value(i).to_string();
            let cluster_id = cluster_ids.value(i);
            let sim_to_centroid = sims.value(i);

            let embedding = embeddings_col
                .value(i)
                .as_any()
                .downcast_ref::<Float32Array>()
                .unwrap()
                .values()
                .to_vec();

            window_data_list.push(WindowData {
                window_id: window_id.clone(),
                window_index,
                page,
                cluster_id,
                embedding,
                text: text.clone(),
            });

            subcell_data_list.push(WindowSubCellData {
                window_id,
                page,
                char_start,
                char_end,
                cluster_id,
                sim_to_centroid,
            });
        }
    }

    // ─── Step 3: Load page data to get char counts ───────────────────────
    let page_batches = store.get_pages_for_job(&job_id).await.map_err(|e| {
        AppError::Storage(crate::types::StorageError {
            message: format!("Failed to load pages: {}", e),
        })
    })?;

    let mut page_char_counts: std::collections::HashMap<u32, u32> =
        std::collections::HashMap::new();
    let mut page_count: u32 = 0;

    for batch in &page_batches {
        if batch.num_rows() == 0 {
            continue;
        }
        let page_nums = batch
            .column_by_name("page")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap();
        let char_counts = batch
            .column_by_name("char_count")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap();

        for i in 0..batch.num_rows() {
            let page_num = page_nums.value(i);
            let char_count = char_counts.value(i);
            page_char_counts.insert(page_num, char_count);
            page_count += 1;
        }
    }

    // ─── Step 4: Build cluster registry ──────────────────────────────────
    let _cluster_registry = build_cluster_registry(&window_data_list);

    // ─── Step 5: Build PageSubGrids ──────────────────────────────────────
    let page_sub_grids = build_page_sub_grids(&subcell_data_list, &page_char_counts);

    // ─── Step 6: Load display state ──────────────────────────────────────
    let display_state = load_display_state(&app_data_dir, &job_id);
    let threshold = display_state.tolerance;
    let gamma = display_state.gamma;
    let hidden: HashSet<i32> = display_state.hidden_clusters.into_iter().collect();

    // ─── Step 7: Rasterize pages and stream events ───────────────────────
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
    job_id: String,
    pages: Vec<u32>,
    threshold: f32,
    gamma: f32,
    hidden_clusters: Vec<i32>,
) -> Result<Vec<PageCanvas>, AppError> {
    // TODO: Retrieve stored PageSubGrid data from LanceDB for the given job_id.
    // Once the full storage retrieval pipeline is wired, this will:
    // 1. Load window data for the job from LanceDB
    // 2. Build PageSubGrids from the window data
    // For now, this placeholder will be replaced when the pipeline is complete.
    let _job_id = job_id;
    let all_grids: Vec<PageSubGrid> = todo!("Load PageSubGrids from storage for job");

    // Core rasterization logic: filter to requested pages and rasterize
    let hidden: HashSet<i32> = hidden_clusters.into_iter().collect();
    let canvases = rasterizer::rasterize_selected_pages(&all_grids, &pages, gamma, threshold, &hidden);
    Ok(canvases)
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
pub async fn get_cluster_registry(job_id: String) -> Result<ClusterRegistry, AppError> {
    let _ = job_id;
    todo!()
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
