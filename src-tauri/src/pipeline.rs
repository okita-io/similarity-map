//! Full analysis pipeline orchestrator.
//!
//! Wires stages: Import → Window → Embed → HDBSCAN → KMeans → Centroid → SubCell → Color → Raster
//! Emits progress events at each stage transition and after each embedding batch.
//! Streams `page-ready` events as pages complete rasterization.
//! Computes rolling ETA from a sliding window of the last 50 batch durations.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::time::Instant;

use tauri::{Emitter, Manager};
use uuid::Uuid;

use crate::cancellation::{self};
use crate::centroid::{build_cluster_registry, WindowData};
use crate::clustering::{derive_min_cluster_size, run_hdbscan, stabilize_clusters, validate_clustering_params};
use crate::embedding::{EmbeddingEngine, DEFAULT_BATCH_SIZE};
use crate::events;
use crate::hash::{compute_document_hash, compute_settings_hash};
use crate::importer;
use crate::model;
use crate::rasterizer::{encode_canvas_base64, rasterize_page};
use crate::storage::{InsertJobParams, PageRecord, Storage, WindowRecord};
use crate::subcell::{build_page_sub_grids, WindowSubCellData};
use crate::types::*;
use crate::windowing::generate_windows;

/// Configuration for the analysis pipeline.
pub struct PipelineConfig {
    pub path: String,
    pub window_size: u32,
    pub stride: u32,
    pub tokens_per_page: Option<u32>,
    pub chapter_break_regex: Option<String>,
    pub min_repetitions: u32,
    pub min_samples: u32,
}

/// Rolling ETA estimator using a sliding window of the last N batch durations.
struct EtaEstimator {
    /// Sliding window of recent batch durations in seconds.
    durations: VecDeque<f64>,
    /// Maximum window size.
    max_window: usize,
}

impl EtaEstimator {
    fn new(max_window: usize) -> Self {
        Self {
            durations: VecDeque::with_capacity(max_window),
            max_window,
        }
    }

    /// Record a batch duration.
    fn record(&mut self, duration_secs: f64) {
        if self.durations.len() >= self.max_window {
            self.durations.pop_front();
        }
        self.durations.push_back(duration_secs);
    }

    /// Estimate remaining time given the number of remaining batches.
    fn estimate_remaining(&self, remaining_batches: usize) -> f64 {
        if self.durations.is_empty() {
            return 0.0;
        }
        let avg: f64 = self.durations.iter().sum::<f64>() / self.durations.len() as f64;
        avg * remaining_batches as f64
    }
}

/// Pipeline stage names for progress events.
#[derive(Debug, Clone, Copy)]
enum Stage {
    Import,
    Windowing,
    Embedding,
    Clustering,
    Stabilization,
    Centroid,
    SubCell,
    Rasterization,
    Complete,
}

impl Stage {
    fn as_str(&self) -> &'static str {
        match self {
            Stage::Import => "import",
            Stage::Windowing => "windowing",
            Stage::Embedding => "embedding",
            Stage::Clustering => "clustering",
            Stage::Stabilization => "stabilization",
            Stage::Centroid => "centroid",
            Stage::SubCell => "subcell",
            Stage::Rasterization => "rasterization",
            Stage::Complete => "complete",
        }
    }
}

/// Emit a progress event for stage transitions.
fn emit_stage_progress(
    app_handle: &tauri::AppHandle,
    job_id: &str,
    stage: Stage,
    pct: f32,
    windows_done: u32,
    windows_total: u32,
    eta_seconds: f64,
) {
    let _ = app_handle.emit(
        events::PROGRESS,
        serde_json::json!({
            "job_id": job_id,
            "stage": stage.as_str(),
            "pct": pct,
            "windows_done": windows_done,
            "windows_total": windows_total,
            "eta_seconds": eta_seconds,
        }),
    );
}

/// Emit a page-ready event with the rasterized canvas.
fn emit_page_ready(app_handle: &tauri::AppHandle, job_id: &str, page: u32, canvas_b64: &str) {
    let _ = app_handle.emit(
        events::PAGE_READY,
        serde_json::json!({
            "job_id": job_id,
            "page": page,
            "canvas_rgba_b64": canvas_b64,
        }),
    );
}

/// Run the full analysis pipeline.
///
/// Orchestrates: Import → Window → Embed → HDBSCAN → KMeans → Centroid → SubCell → Raster
///
/// Emits `similarity-map:progress` events at each stage transition and after each embedding batch.
/// Streams `similarity-map:page-ready` events as pages complete rasterization.
pub async fn run_pipeline(
    config: PipelineConfig,
    app_handle: tauri::AppHandle,
) -> Result<AnalysisHandle, AppError> {
    let job_id = Uuid::new_v4().to_string();

    // ─── Stage 1: Validate Parameters ────────────────────────────────────────
    validate_params(&config)?;

    // ─── Stage 2: Import / Paginate ──────────────────────────────────────────
    emit_stage_progress(&app_handle, &job_id, Stage::Import, 0.0, 0, 0, 0.0);

    let file_path = Path::new(&config.path);
    let pages = import_document(file_path, &config)?;
    let page_count = pages.len() as u32;

    let pagination_mode = if pages.is_empty() {
        "token".to_string()
    } else {
        format!("{:?}", pages[0].pagination_mode).to_lowercase()
    };

    // ─── Stage 3: Generate Windows ───────────────────────────────────────────
    emit_stage_progress(&app_handle, &job_id, Stage::Windowing, 0.0, 0, 0, 0.0);

    let windows = generate_windows(&pages, config.window_size, config.stride);
    let window_count = windows.len() as u32;

    if window_count == 0 {
        return Err(AppError::Import(ImportError {
            message: "Document produced no analyzable windows".to_string(),
            path: Some(config.path.clone()),
        }));
    }

    // ─── Stage 4: Create Job Record in LanceDB ───────────────────────────────
    let document_hash = compute_document_hash(file_path).map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to compute document hash: {}", e),
        })
    })?;

    let settings_hash = compute_settings_hash(
        config.window_size,
        config.stride,
        config.tokens_per_page,
        config.min_repetitions,
        config.min_samples,
    );

    let now = chrono::Utc::now().to_rfc3339();

    // Get storage instance
    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let db_path = app_data_dir.join("similarity_map_db");
    let store = Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    // Insert job record
    store
        .insert_job(InsertJobParams {
            job_id: job_id.clone(),
            document_path: config.path.clone(),
            document_hash,
            settings_hash,
            window_size: config.window_size,
            stride: config.stride,
            tokens_per_page: config.tokens_per_page,
            pagination_mode: pagination_mode.clone(),
            min_repetitions: config.min_repetitions,
            min_samples: config.min_samples,
            chapter_break_re: config.chapter_break_regex.clone(),
            windows_total: window_count,
            windows_committed: 0,
            status: "running".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .await
        .map_err(|e| {
            AppError::Storage(StorageError {
                message: format!("Failed to insert job record: {}", e),
            })
        })?;

    // Insert page records
    let page_records: Vec<PageRecord> = pages
        .iter()
        .map(|p| PageRecord {
            job_id: job_id.clone(),
            page: p.page_num,
            doc_char_start: p.char_offset_in_doc,
            doc_char_end: p.char_offset_in_doc + p.char_count,
            char_count: p.char_count,
            token_count: p.token_count,
            pagination_mode: pagination_mode.clone(),
        })
        .collect();

    store.insert_pages(&page_records).await.map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to insert page records: {}", e),
        })
    })?;

    // ─── Stage 5: Embed Windows in Batches ───────────────────────────────────
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Embedding,
        0.0,
        0,
        window_count,
        0.0,
    );

    let model_path = model::model_path(&app_data_dir);
    let mut engine = EmbeddingEngine::new(&model_path)?;

    let batch_size = DEFAULT_BATCH_SIZE;
    let total_batches = (windows.len() + batch_size - 1) / batch_size;
    let mut eta_estimator = EtaEstimator::new(50);
    let mut windows_done: u32 = 0;

    // Register cancellation token for this job
    let cancel_token = cancellation::global_registry().register(&job_id).await;

    // Store embeddings alongside window data for later clustering
    let mut all_embeddings: Vec<Vec<f32>> = vec![Vec::new(); windows.len()];

    for batch_idx in 0..total_batches {
        // ─── Check for cancellation before processing this batch ───
        if cancel_token.is_cancelled() {
            // Determine status based on committed work
            let status = if windows_done > 0 { "partial" } else { "discarded" };
            let updated_at = chrono::Utc::now().to_rfc3339();
            let _ = store
                .update_job_status(&job_id, status, windows_done, &updated_at)
                .await;

            // Unregister the cancellation token
            cancellation::global_registry().unregister(&job_id).await;

            return Ok(AnalysisHandle {
                job_id,
                page_count,
                window_count,
                pagination_mode,
            });
        }

        let batch_start = Instant::now();
        let chunk_start = batch_idx * batch_size;
        let chunk_end = (chunk_start + batch_size).min(windows.len());
        let batch_windows = &windows[chunk_start..chunk_end];

        let texts: Vec<&str> = batch_windows.iter().map(|w| w.text.as_str()).collect();

        match engine.embed_batch(&texts) {
            Ok(embeddings) => {
                // Build window records for this batch (initially with placeholder cluster data)
                let mut records: Vec<WindowRecord> = Vec::with_capacity(embeddings.len());

                for (i, embedding) in embeddings.into_iter().enumerate() {
                    let w = &batch_windows[i];
                    let global_idx = chunk_start + i;
                    all_embeddings[global_idx] = embedding.clone();

                    records.push(WindowRecord {
                        window_id: w.window_id.clone(),
                        job_id: job_id.clone(),
                        window_index: w.window_index,
                        page: w.page,
                        char_start: w.char_start,
                        char_end: w.char_end,
                        doc_char_start: w.doc_char_start,
                        text: w.text.clone(),
                        embedding,
                        cluster_id: -1,       // placeholder until clustering
                        hdbscan_label: -1,    // placeholder
                        sim_to_centroid: 0.0,  // placeholder
                        sub_cell_row: 0,       // placeholder
                        sub_cell_col: 0,       // placeholder
                    });
                }

                // Commit batch to LanceDB
                store.batch_insert_windows(&records).await.map_err(|e| {
                    AppError::Storage(StorageError {
                        message: format!("Failed to commit embedding batch: {}", e),
                    })
                })?;

                windows_done += records.len() as u32;
            }
            Err(e) => {
                // Log failed batch and continue (per design: skip failed windows)
                log::warn!(
                    "Embedding batch {} failed: {}",
                    batch_idx,
                    e
                );
            }
        }

        // Record batch duration and compute ETA
        let batch_duration = batch_start.elapsed().as_secs_f64();
        eta_estimator.record(batch_duration);
        let remaining_batches = total_batches - (batch_idx + 1);
        let eta = eta_estimator.estimate_remaining(remaining_batches);

        let pct = windows_done as f32 / window_count as f32;

        emit_stage_progress(
            &app_handle,
            &job_id,
            Stage::Embedding,
            pct,
            windows_done,
            window_count,
            eta,
        );

        // Update job progress
        let updated_at = chrono::Utc::now().to_rfc3339();
        let _ = store
            .update_job_status(&job_id, "running", windows_done, &updated_at)
            .await;
    }

    // Unregister the cancellation token now that embedding is complete
    cancellation::global_registry().unregister(&job_id).await;

    // ─── Stage 6: HDBSCAN Clustering ─────────────────────────────────────────
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Clustering,
        0.0,
        windows_done,
        window_count,
        0.0,
    );

    let min_cluster_size = derive_min_cluster_size(
        config.min_repetitions,
        config.window_size,
        config.stride,
    );

    let hdbscan_labels = run_hdbscan(&all_embeddings, min_cluster_size, config.min_samples)?;

    // ─── Stage 7: KMeans Stabilization ───────────────────────────────────────
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Stabilization,
        0.0,
        windows_done,
        window_count,
        0.0,
    );

    let window_indices: Vec<u32> = windows.iter().map(|w| w.window_index).collect();
    let stable_labels = stabilize_clusters(&all_embeddings, &hdbscan_labels, &window_indices);

    // ─── Stage 8: Compute Centroids and Build Cluster Registry ───────────────
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Centroid,
        0.0,
        windows_done,
        window_count,
        0.0,
    );

    let window_data: Vec<WindowData> = windows
        .iter()
        .enumerate()
        .map(|(i, w)| WindowData {
            window_id: w.window_id.clone(),
            window_index: w.window_index,
            page: w.page,
            cluster_id: stable_labels[i],
            embedding: all_embeddings[i].clone(),
            text: w.text.clone(),
        })
        .collect();

    let cluster_registry = build_cluster_registry(&window_data);

    // Compute sim_to_centroid for each window
    let mut sim_to_centroids: Vec<f32> = vec![0.0; windows.len()];
    for (i, wd) in window_data.iter().enumerate() {
        if wd.cluster_id >= 0 {
            if let Some(info) = cluster_registry.clusters.get(&wd.cluster_id) {
                sim_to_centroids[i] = cosine_similarity(&wd.embedding, &info.centroid);
            }
        }
    }

    // ─── Stage 9: Map Windows to Sub-Cells ───────────────────────────────────
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::SubCell,
        0.0,
        windows_done,
        window_count,
        0.0,
    );

    // Build page char counts map
    let page_char_counts: HashMap<u32, u32> = pages
        .iter()
        .map(|p| (p.page_num, p.char_count))
        .collect();

    let subcell_data: Vec<WindowSubCellData> = windows
        .iter()
        .enumerate()
        .map(|(i, w)| WindowSubCellData {
            window_id: w.window_id.clone(),
            page: w.page,
            char_start: w.char_start,
            char_end: w.char_end,
            cluster_id: stable_labels[i],
            sim_to_centroid: sim_to_centroids[i],
        })
        .collect();

    let page_sub_grids = build_page_sub_grids(&subcell_data, &page_char_counts);

    // ─── Stage 10: Rasterize Pages ───────────────────────────────────────────
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Rasterization,
        0.0,
        windows_done,
        window_count,
        0.0,
    );

    let default_threshold = 0.88_f32;
    let default_gamma = 1.5_f32;
    let hidden: HashSet<i32> = HashSet::new();

    for grid in &page_sub_grids {
        let canvas = rasterize_page(grid, default_gamma, default_threshold, &hidden);
        let b64 = encode_canvas_base64(&canvas);
        emit_page_ready(&app_handle, &job_id, grid.page, &b64);
    }

    // ─── Stage 11: Update Job Status to Complete ─────────────────────────────
    let updated_at = chrono::Utc::now().to_rfc3339();
    store
        .update_job_status(&job_id, "complete", windows_done, &updated_at)
        .await
        .map_err(|e| {
            AppError::Storage(StorageError {
                message: format!("Failed to update job status: {}", e),
            })
        })?;

    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Complete,
        1.0,
        windows_done,
        window_count,
        0.0,
    );

    // ─── Return AnalysisHandle ───────────────────────────────────────────────
    Ok(AnalysisHandle {
        job_id,
        page_count,
        window_count,
        pagination_mode,
    })
}

/// Compute resume progress as (current - M) / (N - M).
///
/// Where M = windows_committed (already done) and N = windows_total.
/// Returns 0.0 if N <= M (nothing to do).
pub fn compute_resume_progress(current: u32, windows_committed: u32, windows_total: u32) -> f32 {
    let remaining = windows_total.saturating_sub(windows_committed);
    if remaining == 0 {
        return 1.0;
    }
    let done_in_session = current.saturating_sub(windows_committed);
    done_in_session as f32 / remaining as f32
}

/// Resume a partially completed analysis pipeline.
///
/// Loads the job record, verifies the document hash, re-generates windows,
/// skips already-embedded windows, embeds the remaining, then runs full
/// clustering + rasterization.
///
/// Emits `similarity-map:progress` events with progress based on remaining windows.
/// Streams `similarity-map:page-ready` events as pages complete rasterization.
pub async fn resume_pipeline(
    job_id: String,
    app_handle: tauri::AppHandle,
) -> Result<AnalysisHandle, AppError> {
    // ─── Step 1: Open storage and load job record ────────────────────────
    let app_data_dir = app_handle.path().app_data_dir().map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to resolve app data directory: {}", e),
        })
    })?;

    let db_path = app_data_dir.join("similarity_map_db");
    let store = Storage::open(&db_path).await.map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to open storage: {}", e),
        })
    })?;
    store.ensure_tables().await.map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to ensure tables: {}", e),
        })
    })?;

    let job = store.get_job_by_id(&job_id).await.map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to query job: {}", e),
        })
    })?;

    let job = job.ok_or_else(|| {
        AppError::Session(SessionError {
            message: format!("Job not found: {}", job_id),
        })
    })?;

    // Verify job is in "partial" status
    if job.status != "partial" {
        return Err(AppError::Session(SessionError {
            message: format!(
                "Cannot resume job with status '{}'. Only 'partial' jobs can be resumed.",
                job.status
            ),
        }));
    }

    let windows_committed = job.windows_committed;
    let windows_total = job.windows_total;

    // ─── Step 2: Verify document hash ────────────────────────────────────
    let file_path = Path::new(&job.document_path);
    let current_hash = compute_document_hash(file_path).map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to compute document hash: {}", e),
        })
    })?;

    if current_hash != job.document_hash {
        // Auto-discard: document was edited since partial job started
        let updated_at = chrono::Utc::now().to_rfc3339();
        let _ = store
            .update_job_status(&job_id, "discarded", windows_committed, &updated_at)
            .await;

        return Err(AppError::Session(SessionError {
            message: "Document has been edited since the partial analysis started. The partial job has been discarded.".to_string(),
        }));
    }

    // ─── Step 3: Re-import and re-window the document ────────────────────
    emit_stage_progress(&app_handle, &job_id, Stage::Import, 0.0, 0, 0, 0.0);

    let config = PipelineConfig {
        path: job.document_path.clone(),
        window_size: job.window_size,
        stride: job.stride,
        tokens_per_page: job.tokens_per_page,
        chapter_break_regex: job.chapter_break_re.clone(),
        min_repetitions: job.min_repetitions,
        min_samples: job.min_samples,
    };

    let pages = import_document(file_path, &config)?;
    let page_count = pages.len() as u32;

    let pagination_mode = if pages.is_empty() {
        "token".to_string()
    } else {
        format!("{:?}", pages[0].pagination_mode).to_lowercase()
    };

    emit_stage_progress(&app_handle, &job_id, Stage::Windowing, 0.0, 0, 0, 0.0);

    let windows = generate_windows(&pages, config.window_size, config.stride);
    let window_count = windows.len() as u32;

    if window_count == 0 {
        return Err(AppError::Import(ImportError {
            message: "Document produced no analyzable windows on re-import".to_string(),
            path: Some(config.path.clone()),
        }));
    }

    // ─── Step 4: Update job status to running ────────────────────────────
    let updated_at = chrono::Utc::now().to_rfc3339();
    store
        .update_job_status(&job_id, "running", windows_committed, &updated_at)
        .await
        .map_err(|e| {
            AppError::Storage(StorageError {
                message: format!("Failed to update job status: {}", e),
            })
        })?;

    // ─── Step 5: Embed remaining windows (skip window_index < windows_committed) ─
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Embedding,
        0.0,
        windows_committed,
        windows_total,
        0.0,
    );

    let model_path = model::model_path(&app_data_dir);
    let mut engine = EmbeddingEngine::new(&model_path)?;

    // Filter to only windows that need embedding
    let remaining_windows: Vec<&Window> = windows
        .iter()
        .filter(|w| w.window_index >= windows_committed)
        .collect();

    let batch_size = DEFAULT_BATCH_SIZE;
    let total_remaining_batches = (remaining_windows.len() + batch_size - 1) / batch_size;
    let mut eta_estimator = EtaEstimator::new(50);
    let mut windows_done: u32 = windows_committed;

    // We need all embeddings for clustering later — load already-committed ones from storage
    // and embed the remaining ones
    let mut all_embeddings: Vec<Vec<f32>> = vec![Vec::new(); windows.len()];

    // Load existing embeddings from storage for already-committed windows
    let existing_records = store.get_embeddings_for_job(&job_id).await.map_err(|e| {
        AppError::Storage(StorageError {
            message: format!("Failed to load existing embeddings: {}", e),
        })
    })?;

    for record in &existing_records {
        let idx = record.window_index as usize;
        if idx < all_embeddings.len() {
            all_embeddings[idx] = record.embedding.clone();
        }
    }

    // Embed remaining windows in batches
    for batch_idx in 0..total_remaining_batches {
        let batch_start = Instant::now();
        let chunk_start = batch_idx * batch_size;
        let chunk_end = (chunk_start + batch_size).min(remaining_windows.len());
        let batch_windows = &remaining_windows[chunk_start..chunk_end];

        let texts: Vec<&str> = batch_windows.iter().map(|w| w.text.as_str()).collect();

        match engine.embed_batch(&texts) {
            Ok(embeddings) => {
                let mut records: Vec<WindowRecord> = Vec::with_capacity(embeddings.len());

                for (i, embedding) in embeddings.into_iter().enumerate() {
                    let w = batch_windows[i];
                    let global_idx = w.window_index as usize;
                    all_embeddings[global_idx] = embedding.clone();

                    records.push(WindowRecord {
                        window_id: w.window_id.clone(),
                        job_id: job_id.clone(),
                        window_index: w.window_index,
                        page: w.page,
                        char_start: w.char_start,
                        char_end: w.char_end,
                        doc_char_start: w.doc_char_start,
                        text: w.text.clone(),
                        embedding,
                        cluster_id: -1,
                        hdbscan_label: -1,
                        sim_to_centroid: 0.0,
                        sub_cell_row: 0,
                        sub_cell_col: 0,
                    });
                }

                // Commit batch to LanceDB
                store.batch_insert_windows(&records).await.map_err(|e| {
                    AppError::Storage(StorageError {
                        message: format!("Failed to commit embedding batch: {}", e),
                    })
                })?;

                windows_done += records.len() as u32;
            }
            Err(e) => {
                log::warn!(
                    "Embedding batch {} failed during resume: {}",
                    batch_idx,
                    e
                );
            }
        }

        // Record batch duration and compute ETA
        let batch_duration = batch_start.elapsed().as_secs_f64();
        eta_estimator.record(batch_duration);
        let remaining_batches = total_remaining_batches - (batch_idx + 1);
        let eta = eta_estimator.estimate_remaining(remaining_batches);

        // Progress as (current - M) / (N - M)
        let pct = compute_resume_progress(windows_done, windows_committed, windows_total);

        emit_stage_progress(
            &app_handle,
            &job_id,
            Stage::Embedding,
            pct,
            windows_done,
            windows_total,
            eta,
        );

        // Update job progress
        let updated_at = chrono::Utc::now().to_rfc3339();
        let _ = store
            .update_job_status(&job_id, "running", windows_done, &updated_at)
            .await;
    }

    // ─── Step 6: Full clustering + rasterization ─────────────────────────
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Clustering,
        0.0,
        windows_done,
        windows_total,
        0.0,
    );

    let min_cluster_size = derive_min_cluster_size(
        config.min_repetitions,
        config.window_size,
        config.stride,
    );

    let hdbscan_labels = run_hdbscan(&all_embeddings, min_cluster_size, config.min_samples)?;

    // KMeans Stabilization
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Stabilization,
        0.0,
        windows_done,
        windows_total,
        0.0,
    );

    let window_indices: Vec<u32> = windows.iter().map(|w| w.window_index).collect();
    let stable_labels = stabilize_clusters(&all_embeddings, &hdbscan_labels, &window_indices);

    // Centroid computation
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Centroid,
        0.0,
        windows_done,
        windows_total,
        0.0,
    );

    let window_data: Vec<WindowData> = windows
        .iter()
        .enumerate()
        .map(|(i, w)| WindowData {
            window_id: w.window_id.clone(),
            window_index: w.window_index,
            page: w.page,
            cluster_id: stable_labels[i],
            embedding: all_embeddings[i].clone(),
            text: w.text.clone(),
        })
        .collect();

    let cluster_registry = build_cluster_registry(&window_data);

    // Compute sim_to_centroid for each window
    let mut sim_to_centroids: Vec<f32> = vec![0.0; windows.len()];
    for (i, wd) in window_data.iter().enumerate() {
        if wd.cluster_id >= 0 {
            if let Some(info) = cluster_registry.clusters.get(&wd.cluster_id) {
                sim_to_centroids[i] = cosine_similarity(&wd.embedding, &info.centroid);
            }
        }
    }

    // Sub-cell mapping
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::SubCell,
        0.0,
        windows_done,
        windows_total,
        0.0,
    );

    let page_char_counts: HashMap<u32, u32> = pages
        .iter()
        .map(|p| (p.page_num, p.char_count))
        .collect();

    let subcell_data: Vec<WindowSubCellData> = windows
        .iter()
        .enumerate()
        .map(|(i, w)| WindowSubCellData {
            window_id: w.window_id.clone(),
            page: w.page,
            char_start: w.char_start,
            char_end: w.char_end,
            cluster_id: stable_labels[i],
            sim_to_centroid: sim_to_centroids[i],
        })
        .collect();

    let page_sub_grids = build_page_sub_grids(&subcell_data, &page_char_counts);

    // Rasterization
    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Rasterization,
        0.0,
        windows_done,
        windows_total,
        0.0,
    );

    let default_threshold = 0.88_f32;
    let default_gamma = 1.5_f32;
    let hidden: HashSet<i32> = HashSet::new();

    for grid in &page_sub_grids {
        let canvas = rasterize_page(grid, default_gamma, default_threshold, &hidden);
        let b64 = encode_canvas_base64(&canvas);
        emit_page_ready(&app_handle, &job_id, grid.page, &b64);
    }

    // ─── Step 7: Update job status to complete ───────────────────────────
    let updated_at = chrono::Utc::now().to_rfc3339();
    store
        .update_job_status(&job_id, "complete", windows_done, &updated_at)
        .await
        .map_err(|e| {
            AppError::Storage(StorageError {
                message: format!("Failed to update job status: {}", e),
            })
        })?;

    emit_stage_progress(
        &app_handle,
        &job_id,
        Stage::Complete,
        1.0,
        windows_done,
        windows_total,
        0.0,
    );

    Ok(AnalysisHandle {
        job_id,
        page_count,
        window_count,
        pagination_mode,
    })
}

/// Validate pipeline configuration parameters.
fn validate_params(config: &PipelineConfig) -> Result<(), AppError> {
    // Validate file path
    if config.path.is_empty() {
        return Err(AppError::Validation(ValidationError {
            field: "path".to_string(),
            message: "Document path cannot be empty".to_string(),
        }));
    }

    // Validate window_size (5–1500)
    if config.window_size < 5 || config.window_size > 1500 {
        return Err(AppError::Validation(ValidationError {
            field: "window_size".to_string(),
            message: format!(
                "window_size must be between 5 and 1500, got {}",
                config.window_size
            ),
        }));
    }

    // Validate stride (1–200)
    if config.stride < 1 || config.stride > 200 {
        return Err(AppError::Validation(ValidationError {
            field: "stride".to_string(),
            message: format!("stride must be between 1 and 200, got {}", config.stride),
        }));
    }

    // Validate tokens_per_page if provided (200–2000)
    if let Some(tpp) = config.tokens_per_page {
        if tpp < 200 || tpp > 2000 {
            return Err(AppError::Validation(ValidationError {
                field: "tokens_per_page".to_string(),
                message: format!(
                    "tokens_per_page must be between 200 and 2000, got {}",
                    tpp
                ),
            }));
        }
    }

    // Validate chapter_break_regex if provided
    if let Some(ref regex_str) = config.chapter_break_regex {
        if !regex_str.is_empty() {
            regex::Regex::new(regex_str).map_err(|e| {
                AppError::Validation(ValidationError {
                    field: "chapter_break_regex".to_string(),
                    message: format!("Invalid regex pattern: {}", e),
                })
            })?;
        }
    }

    // Validate clustering params
    validate_clustering_params(config.min_repetitions, config.min_samples)?;

    Ok(())
}

/// Import and paginate the document based on file type and config.
fn import_document(file_path: &Path, config: &PipelineConfig) -> Result<Vec<Page>, AppError> {
    let is_pdf = file_path
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);

    if is_pdf {
        importer::import_pdf(file_path)
    } else {
        let text = std::fs::read_to_string(file_path).map_err(|e| {
            AppError::Import(ImportError {
                message: format!("Failed to read file: {}", e),
                path: Some(config.path.clone()),
            })
        })?;

        // Use chapter break pagination if regex is provided
        if let Some(ref regex_str) = config.chapter_break_regex {
            if !regex_str.is_empty() {
                let tpp = config.tokens_per_page.unwrap_or(400);
                return importer::paginate_by_chapter_break(&text, regex_str, tpp);
            }
        }

        // Default to token-count pagination
        let tpp = config.tokens_per_page.unwrap_or(400);
        importer::paginate_by_token_count(&text, tpp)
    }
}

/// Compute cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denom = norm_a * norm_b;
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eta_estimator_empty() {
        let est = EtaEstimator::new(50);
        assert_eq!(est.estimate_remaining(10), 0.0);
    }

    #[test]
    fn test_eta_estimator_single_sample() {
        let mut est = EtaEstimator::new(50);
        est.record(2.0);
        // 10 remaining batches × 2.0 avg = 20.0
        assert!((est.estimate_remaining(10) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_eta_estimator_multiple_samples() {
        let mut est = EtaEstimator::new(50);
        est.record(1.0);
        est.record(3.0);
        // avg = 2.0, 5 remaining → 10.0
        assert!((est.estimate_remaining(5) - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_eta_estimator_sliding_window() {
        let mut est = EtaEstimator::new(3);
        est.record(10.0);
        est.record(10.0);
        est.record(10.0);
        // Window full: [10, 10, 10], avg = 10
        assert!((est.estimate_remaining(1) - 10.0).abs() < 1e-6);

        // Add a new sample, oldest should be evicted
        est.record(1.0);
        // Window: [10, 10, 1], avg = 7.0
        assert!((est.estimate_remaining(1) - 7.0).abs() < 1e-6);
    }

    #[test]
    fn test_validate_params_valid() {
        let config = PipelineConfig {
            path: "/some/file.txt".to_string(),
            window_size: 20,
            stride: 5,
            tokens_per_page: Some(400),
            chapter_break_regex: None,
            min_repetitions: 3,
            min_samples: 3,
        };
        assert!(validate_params(&config).is_ok());
    }

    #[test]
    fn test_validate_params_empty_path() {
        let config = PipelineConfig {
            path: "".to_string(),
            window_size: 20,
            stride: 5,
            tokens_per_page: None,
            chapter_break_regex: None,
            min_repetitions: 3,
            min_samples: 3,
        };
        assert!(validate_params(&config).is_err());
    }

    #[test]
    fn test_validate_params_window_size_too_small() {
        let config = PipelineConfig {
            path: "/file.txt".to_string(),
            window_size: 4,
            stride: 5,
            tokens_per_page: None,
            chapter_break_regex: None,
            min_repetitions: 3,
            min_samples: 3,
        };
        assert!(validate_params(&config).is_err());
    }

    #[test]
    fn test_validate_params_stride_too_large() {
        let config = PipelineConfig {
            path: "/file.txt".to_string(),
            window_size: 20,
            stride: 201,
            tokens_per_page: None,
            chapter_break_regex: None,
            min_repetitions: 3,
            min_samples: 3,
        };
        assert!(validate_params(&config).is_err());
    }

    #[test]
    fn test_validate_params_invalid_regex() {
        let config = PipelineConfig {
            path: "/file.txt".to_string(),
            window_size: 20,
            stride: 5,
            tokens_per_page: None,
            chapter_break_regex: Some("[invalid(".to_string()),
            min_repetitions: 3,
            min_samples: 3,
        };
        assert!(validate_params(&config).is_err());
    }

    #[test]
    fn test_validate_params_tokens_per_page_out_of_range() {
        let config = PipelineConfig {
            path: "/file.txt".to_string(),
            window_size: 20,
            stride: 5,
            tokens_per_page: Some(100),
            chapter_break_regex: None,
            min_repetitions: 3,
            min_samples: 3,
        };
        assert!(validate_params(&config).is_err());
    }

    #[test]
    fn test_stage_names() {
        assert_eq!(Stage::Import.as_str(), "import");
        assert_eq!(Stage::Windowing.as_str(), "windowing");
        assert_eq!(Stage::Embedding.as_str(), "embedding");
        assert_eq!(Stage::Clustering.as_str(), "clustering");
        assert_eq!(Stage::Stabilization.as_str(), "stabilization");
        assert_eq!(Stage::Centroid.as_str(), "centroid");
        assert_eq!(Stage::SubCell.as_str(), "subcell");
        assert_eq!(Stage::Rasterization.as_str(), "rasterization");
        assert_eq!(Stage::Complete.as_str(), "complete");
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    // ─── Resume Progress Tests ───────────────────────────────────────────

    #[test]
    fn test_resume_progress_at_start() {
        // M=50, N=100, current=50 → (50-50)/(100-50) = 0.0
        let pct = compute_resume_progress(50, 50, 100);
        assert!((pct - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_resume_progress_halfway() {
        // M=50, N=100, current=75 → (75-50)/(100-50) = 0.5
        let pct = compute_resume_progress(75, 50, 100);
        assert!((pct - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_resume_progress_complete() {
        // M=50, N=100, current=100 → (100-50)/(100-50) = 1.0
        let pct = compute_resume_progress(100, 50, 100);
        assert!((pct - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_resume_progress_nothing_to_do() {
        // M=100, N=100 → remaining=0, returns 1.0
        let pct = compute_resume_progress(100, 100, 100);
        assert!((pct - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_resume_progress_from_zero() {
        // M=0, N=200, current=100 → (100-0)/(200-0) = 0.5
        let pct = compute_resume_progress(100, 0, 200);
        assert!((pct - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_resume_progress_small_remaining() {
        // M=95, N=100, current=97 → (97-95)/(100-95) = 2/5 = 0.4
        let pct = compute_resume_progress(97, 95, 100);
        assert!((pct - 0.4).abs() < 1e-6);
    }

    #[test]
    fn test_resume_progress_current_below_committed_saturates() {
        // Edge case: current < M (shouldn't happen, but saturating_sub handles it)
        // M=50, N=100, current=30 → saturating_sub(30,50)=0, 0/(100-50)=0.0
        let pct = compute_resume_progress(30, 50, 100);
        assert!((pct - 0.0).abs() < 1e-6);
    }
}
