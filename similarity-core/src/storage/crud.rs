//! CRUD operations for the LanceDB storage layer.
//!
//! Provides insert, query, update, and delete operations for the `jobs`, `windows`,
//! and `pages` tables.

use std::sync::Arc;

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatch, StringArray, UInt32Array,
    UInt8Array,
};
use arrow_schema::{ArrowError, DataType, Field};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

use super::schema::EMBEDDING_DIM;
use super::{Storage, StorageError, TABLE_JOBS, TABLE_PAGES, TABLE_WINDOWS};

/// Parameters for inserting a new job record.
pub struct InsertJobParams {
    pub job_id: String,
    pub document_path: String,
    pub document_hash: String,
    pub settings_hash: String,
    pub window_size: u32,
    pub stride: u32,
    pub tokens_per_page: Option<u32>,
    pub pagination_mode: String,
    pub min_repetitions: u32,
    pub min_samples: u32,
    pub chapter_break_re: Option<String>,
    pub windows_total: u32,
    pub windows_committed: u32,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A retrieved job record from the jobs table.
#[derive(Debug, Clone)]
pub struct JobRecord {
    pub job_id: String,
    pub document_path: String,
    pub document_hash: String,
    pub settings_hash: String,
    pub window_size: u32,
    pub stride: u32,
    pub tokens_per_page: Option<u32>,
    pub pagination_mode: String,
    pub min_repetitions: u32,
    pub min_samples: u32,
    pub chapter_break_re: Option<String>,
    pub windows_total: u32,
    pub windows_committed: u32,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Parameters for inserting a batch of window records.
pub struct WindowRecord {
    pub window_id: String,
    pub job_id: String,
    pub window_index: u32,
    pub page: u32,
    pub char_start: u32,
    pub char_end: u32,
    pub doc_char_start: u32,
    pub text: String,
    pub embedding: Vec<f32>,
    pub cluster_id: i32,
    pub hdbscan_label: i32,
    pub sim_to_centroid: f32,
    pub sub_cell_row: u8,
    pub sub_cell_col: u8,
}

/// Parameters for inserting page records.
pub struct PageRecord {
    pub job_id: String,
    pub page: u32,
    pub doc_char_start: u32,
    pub doc_char_end: u32,
    pub char_count: u32,
    pub token_count: u32,
    pub pagination_mode: String,
}

/// A retrieved embedding with its window metadata.
pub struct EmbeddingRecord {
    pub window_id: String,
    pub window_index: u32,
    pub page: u32,
    pub cluster_id: i32,
    pub embedding: Vec<f32>,
}

/// Convert an ArrowError into our StorageError.
fn arrow_err(e: ArrowError) -> StorageError {
    StorageError::Lance(lancedb::Error::from(e))
}

/// Convert a generic arrow error from stream collection.
fn stream_err(e: impl std::error::Error + Send + Sync + 'static) -> StorageError {
    StorageError::Lance(lancedb::Error::Runtime {
        message: e.to_string(),
    })
}

impl Storage {
    // ─── Job Operations ──────────────────────────────────────────────────

    /// Insert a new job record into the jobs table.
    pub async fn insert_job(&self, params: InsertJobParams) -> Result<(), StorageError> {
        let table = self.open_table(TABLE_JOBS).await?;
        let schema = Arc::new(super::schema::jobs_schema());

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec![params.job_id.as_str()])),
                Arc::new(StringArray::from(vec![params.document_path.as_str()])),
                Arc::new(StringArray::from(vec![params.document_hash.as_str()])),
                Arc::new(StringArray::from(vec![params.settings_hash.as_str()])),
                Arc::new(UInt32Array::from(vec![params.window_size])),
                Arc::new(UInt32Array::from(vec![params.stride])),
                Arc::new(UInt32Array::from(vec![
                    params.tokens_per_page.unwrap_or(0),
                ])),
                Arc::new(StringArray::from(vec![params.pagination_mode.as_str()])),
                Arc::new(UInt32Array::from(vec![params.min_repetitions])),
                Arc::new(UInt32Array::from(vec![params.min_samples])),
                Arc::new(StringArray::from(vec![params
                    .chapter_break_re
                    .as_deref()
                    .unwrap_or("")])),
                Arc::new(UInt32Array::from(vec![params.windows_total])),
                Arc::new(UInt32Array::from(vec![params.windows_committed])),
                Arc::new(StringArray::from(vec![params.status.as_str()])),
                Arc::new(StringArray::from(vec![params.created_at.as_str()])),
                Arc::new(StringArray::from(vec![params.updated_at.as_str()])),
            ],
        )
        .map_err(arrow_err)?;

        table.add(vec![batch]).execute().await?;
        Ok(())
    }

    /// Update a job's status and windows_committed count.
    pub async fn update_job_status(
        &self,
        job_id: &str,
        status: &str,
        windows_committed: u32,
        updated_at: &str,
    ) -> Result<(), StorageError> {
        let table = self.open_table(TABLE_JOBS).await?;
        let filter = format!("job_id = '{}'", job_id);

        // Read existing record
        let stream = table.query().only_if(filter.clone()).execute().await?;
        let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(stream_err)?;

        if batches.is_empty() || batches[0].num_rows() == 0 {
            return Err(StorageError::Lance(lancedb::Error::InvalidInput {
                message: format!("Job not found: {}", job_id),
            }));
        }

        // Delete old record then re-insert with updated fields
        table.delete(&filter).await?;

        let old = &batches[0];
        let schema = old.schema();

        // Build updated record reusing existing columns
        let columns: Vec<Arc<dyn Array>> = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, field)| match field.name().as_str() {
                "status" => Arc::new(StringArray::from(vec![status])) as Arc<dyn Array>,
                "windows_committed" => {
                    Arc::new(UInt32Array::from(vec![windows_committed])) as Arc<dyn Array>
                }
                "updated_at" => Arc::new(StringArray::from(vec![updated_at])) as Arc<dyn Array>,
                _ => old.column(i).slice(0, 1),
            })
            .collect();

        let new_batch = RecordBatch::try_new(schema, columns).map_err(arrow_err)?;
        table.add(vec![new_batch]).execute().await?;
        Ok(())
    }

    /// Get all jobs for a given document path.
    pub async fn get_jobs_for_document(
        &self,
        document_path: &str,
    ) -> Result<Vec<RecordBatch>, StorageError> {
        let table = self.open_table(TABLE_JOBS).await?;
        let filter = format!("document_path = '{}'", document_path);
        let stream = table.query().only_if(filter).execute().await?;
        let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(stream_err)?;
        Ok(batches)
    }

    /// Parse all job records from a set of RecordBatches.
    pub fn parse_job_records(batches: &[RecordBatch]) -> Vec<JobRecord> {
        let mut records = Vec::new();
        for batch in batches {
            if batch.num_rows() == 0 {
                continue;
            }
            let job_ids = batch.column_by_name("job_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
            let document_paths = batch.column_by_name("document_path").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
            let document_hashes = batch.column_by_name("document_hash").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
            let settings_hashes = batch.column_by_name("settings_hash").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
            let window_sizes = batch.column_by_name("window_size").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
            let strides = batch.column_by_name("stride").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
            let tokens_per_pages = batch.column_by_name("tokens_per_page").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
            let pagination_modes = batch.column_by_name("pagination_mode").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
            let min_repetitions_col = batch.column_by_name("min_repetitions").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
            let min_samples_col = batch.column_by_name("min_samples").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
            let chapter_break_res = batch.column_by_name("chapter_break_re").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
            let windows_totals = batch.column_by_name("windows_total").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
            let windows_committeds = batch.column_by_name("windows_committed").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
            let statuses = batch.column_by_name("status").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
            let created_ats = batch.column_by_name("created_at").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
            let updated_ats = batch.column_by_name("updated_at").unwrap().as_any().downcast_ref::<StringArray>().unwrap();

            for i in 0..batch.num_rows() {
                let tpp_val = tokens_per_pages.value(i);
                let tokens_per_page = if tpp_val == 0 { None } else { Some(tpp_val) };

                let chapter_re_val = chapter_break_res.value(i).to_string();
                let chapter_break_re = if chapter_re_val.is_empty() { None } else { Some(chapter_re_val) };

                records.push(JobRecord {
                    job_id: job_ids.value(i).to_string(),
                    document_path: document_paths.value(i).to_string(),
                    document_hash: document_hashes.value(i).to_string(),
                    settings_hash: settings_hashes.value(i).to_string(),
                    window_size: window_sizes.value(i),
                    stride: strides.value(i),
                    tokens_per_page,
                    pagination_mode: pagination_modes.value(i).to_string(),
                    min_repetitions: min_repetitions_col.value(i),
                    min_samples: min_samples_col.value(i),
                    chapter_break_re,
                    windows_total: windows_totals.value(i),
                    windows_committed: windows_committeds.value(i),
                    status: statuses.value(i).to_string(),
                    created_at: created_ats.value(i).to_string(),
                    updated_at: updated_ats.value(i).to_string(),
                });
            }
        }
        records
    }

    /// Get a single job record by job_id.
    /// Returns None if the job does not exist.
    pub async fn get_job_by_id(
        &self,
        job_id: &str,
    ) -> Result<Option<JobRecord>, StorageError> {
        let table = self.open_table(TABLE_JOBS).await?;
        let filter = format!("job_id = '{}'", job_id);
        let stream = table.query().only_if(filter).execute().await?;
        let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(stream_err)?;

        if batches.is_empty() || batches[0].num_rows() == 0 {
            return Ok(None);
        }

        let batch = &batches[0];
        let job_ids = batch.column_by_name("job_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let document_paths = batch.column_by_name("document_path").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let document_hashes = batch.column_by_name("document_hash").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let settings_hashes = batch.column_by_name("settings_hash").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let window_sizes = batch.column_by_name("window_size").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
        let strides = batch.column_by_name("stride").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
        let tokens_per_pages = batch.column_by_name("tokens_per_page").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
        let pagination_modes = batch.column_by_name("pagination_mode").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let min_repetitions_col = batch.column_by_name("min_repetitions").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
        let min_samples_col = batch.column_by_name("min_samples").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
        let chapter_break_res = batch.column_by_name("chapter_break_re").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let windows_totals = batch.column_by_name("windows_total").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
        let windows_committeds = batch.column_by_name("windows_committed").unwrap().as_any().downcast_ref::<UInt32Array>().unwrap();
        let statuses = batch.column_by_name("status").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let created_ats = batch.column_by_name("created_at").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let updated_ats = batch.column_by_name("updated_at").unwrap().as_any().downcast_ref::<StringArray>().unwrap();

        let tpp_val = tokens_per_pages.value(0);
        let tokens_per_page = if tpp_val == 0 { None } else { Some(tpp_val) };

        let chapter_re_val = chapter_break_res.value(0).to_string();
        let chapter_break_re = if chapter_re_val.is_empty() { None } else { Some(chapter_re_val) };

        Ok(Some(JobRecord {
            job_id: job_ids.value(0).to_string(),
            document_path: document_paths.value(0).to_string(),
            document_hash: document_hashes.value(0).to_string(),
            settings_hash: settings_hashes.value(0).to_string(),
            window_size: window_sizes.value(0),
            stride: strides.value(0),
            tokens_per_page,
            pagination_mode: pagination_modes.value(0).to_string(),
            min_repetitions: min_repetitions_col.value(0),
            min_samples: min_samples_col.value(0),
            chapter_break_re,
            windows_total: windows_totals.value(0),
            windows_committed: windows_committeds.value(0),
            status: statuses.value(0).to_string(),
            created_at: created_ats.value(0).to_string(),
            updated_at: updated_ats.value(0).to_string(),
        }))
    }

    // ─── Window Operations ───────────────────────────────────────────────

    /// Insert a batch of window records with embeddings into the windows table.
    pub async fn batch_insert_windows(
        &self,
        windows: &[WindowRecord],
    ) -> Result<(), StorageError> {
        if windows.is_empty() {
            return Ok(());
        }

        let table = self.open_table(TABLE_WINDOWS).await?;
        let schema = Arc::new(super::schema::windows_schema());

        let window_ids: Vec<&str> = windows.iter().map(|w| w.window_id.as_str()).collect();
        let job_ids: Vec<&str> = windows.iter().map(|w| w.job_id.as_str()).collect();
        let window_indices: Vec<u32> = windows.iter().map(|w| w.window_index).collect();
        let pages: Vec<u32> = windows.iter().map(|w| w.page).collect();
        let char_starts: Vec<u32> = windows.iter().map(|w| w.char_start).collect();
        let char_ends: Vec<u32> = windows.iter().map(|w| w.char_end).collect();
        let doc_char_starts: Vec<u32> = windows.iter().map(|w| w.doc_char_start).collect();
        let texts: Vec<&str> = windows.iter().map(|w| w.text.as_str()).collect();
        let cluster_ids: Vec<i32> = windows.iter().map(|w| w.cluster_id).collect();
        let hdbscan_labels: Vec<i32> = windows.iter().map(|w| w.hdbscan_label).collect();
        let sims: Vec<f32> = windows.iter().map(|w| w.sim_to_centroid).collect();
        let rows: Vec<u8> = windows.iter().map(|w| w.sub_cell_row).collect();
        let cols: Vec<u8> = windows.iter().map(|w| w.sub_cell_col).collect();

        // Build the FixedSizeList embedding column
        let all_embeddings: Vec<f32> = windows
            .iter()
            .flat_map(|w| w.embedding.iter().copied())
            .collect();
        let values = Float32Array::from(all_embeddings);
        let embedding_field = Arc::new(Field::new("item", DataType::Float32, true));
        let embedding_array = FixedSizeListArray::try_new(
            embedding_field,
            EMBEDDING_DIM,
            Arc::new(values),
            None,
        )
        .map_err(arrow_err)?;

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(window_ids)),
                Arc::new(StringArray::from(job_ids)),
                Arc::new(UInt32Array::from(window_indices)),
                Arc::new(UInt32Array::from(pages)),
                Arc::new(UInt32Array::from(char_starts)),
                Arc::new(UInt32Array::from(char_ends)),
                Arc::new(UInt32Array::from(doc_char_starts)),
                Arc::new(StringArray::from(texts)),
                Arc::new(embedding_array) as Arc<dyn Array>,
                Arc::new(Int32Array::from(cluster_ids)),
                Arc::new(Int32Array::from(hdbscan_labels)),
                Arc::new(Float32Array::from(sims)),
                Arc::new(UInt8Array::from(rows)),
                Arc::new(UInt8Array::from(cols)),
            ],
        )
        .map_err(arrow_err)?;

        table.add(vec![batch]).execute().await?;
        Ok(())
    }

    /// Delete all window rows for a job (pages and job record are untouched).
    pub async fn delete_windows_for_job(&self, job_id: &str) -> Result<(), StorageError> {
        let table = self.open_table(TABLE_WINDOWS).await?;
        let filter = format!("job_id = '{}'", job_id);
        table.delete(&filter).await?;
        Ok(())
    }

    /// Get all windows for a given job_id.
    pub async fn get_windows_for_job(
        &self,
        job_id: &str,
    ) -> Result<Vec<RecordBatch>, StorageError> {
        let table = self.open_table(TABLE_WINDOWS).await?;
        let filter = format!("job_id = '{}'", job_id);
        let stream = table.query().only_if(filter).execute().await?;
        let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(stream_err)?;
        Ok(batches)
    }

    /// Get the count of windows for a given job_id.
    pub async fn get_window_count(&self, job_id: &str) -> Result<u32, StorageError> {
        let batches = self.get_windows_for_job(job_id).await?;
        let count: usize = batches.iter().map(|b| b.num_rows()).sum();
        Ok(count as u32)
    }

    /// Retrieve embedding vectors and metadata for all windows of a job.
    /// Used for clustering operations.
    pub async fn get_embeddings_for_job(
        &self,
        job_id: &str,
    ) -> Result<Vec<EmbeddingRecord>, StorageError> {
        let table = self.open_table(TABLE_WINDOWS).await?;
        let filter = format!("job_id = '{}'", job_id);
        let stream = table
            .query()
            .only_if(filter)
            .select(lancedb::query::Select::Columns(vec![
                "window_id".to_string(),
                "window_index".to_string(),
                "page".to_string(),
                "cluster_id".to_string(),
                "embedding".to_string(),
            ]))
            .execute()
            .await?;
        let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(stream_err)?;

        let mut records = Vec::new();
        for batch in &batches {
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
            let pages = batch
                .column_by_name("page")
                .unwrap()
                .as_any()
                .downcast_ref::<UInt32Array>()
                .unwrap();
            let cluster_ids = batch
                .column_by_name("cluster_id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap();
            let embeddings_col = batch
                .column_by_name("embedding")
                .unwrap()
                .as_any()
                .downcast_ref::<FixedSizeListArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                let embedding_values = embeddings_col
                    .value(i)
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .unwrap()
                    .values()
                    .to_vec();

                records.push(EmbeddingRecord {
                    window_id: window_ids.value(i).to_string(),
                    window_index: window_indices.value(i),
                    page: pages.value(i),
                    cluster_id: cluster_ids.value(i),
                    embedding: embedding_values,
                });
            }
        }
        Ok(records)
    }

    // ─── Page Operations ─────────────────────────────────────────────────

    /// Insert page records into the pages table.
    pub async fn insert_pages(&self, pages: &[PageRecord]) -> Result<(), StorageError> {
        if pages.is_empty() {
            return Ok(());
        }

        let table = self.open_table(TABLE_PAGES).await?;
        let schema = Arc::new(super::schema::pages_schema());

        let job_ids: Vec<&str> = pages.iter().map(|p| p.job_id.as_str()).collect();
        let page_nums: Vec<u32> = pages.iter().map(|p| p.page).collect();
        let doc_char_starts: Vec<u32> = pages.iter().map(|p| p.doc_char_start).collect();
        let doc_char_ends: Vec<u32> = pages.iter().map(|p| p.doc_char_end).collect();
        let char_counts: Vec<u32> = pages.iter().map(|p| p.char_count).collect();
        let token_counts: Vec<u32> = pages.iter().map(|p| p.token_count).collect();
        let modes: Vec<&str> = pages.iter().map(|p| p.pagination_mode.as_str()).collect();

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(job_ids)),
                Arc::new(UInt32Array::from(page_nums)),
                Arc::new(UInt32Array::from(doc_char_starts)),
                Arc::new(UInt32Array::from(doc_char_ends)),
                Arc::new(UInt32Array::from(char_counts)),
                Arc::new(UInt32Array::from(token_counts)),
                Arc::new(StringArray::from(modes)),
            ],
        )
        .map_err(arrow_err)?;

        table.add(vec![batch]).execute().await?;
        Ok(())
    }

    /// Get all pages for a given job_id.
    pub async fn get_pages_for_job(
        &self,
        job_id: &str,
    ) -> Result<Vec<RecordBatch>, StorageError> {
        let table = self.open_table(TABLE_PAGES).await?;
        let filter = format!("job_id = '{}'", job_id);
        let stream = table.query().only_if(filter).execute().await?;
        let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(stream_err)?;
        Ok(batches)
    }

    // ─── Delete Operations ───────────────────────────────────────────────

    /// Delete all data for a job: windows, pages, and the job record itself.
    /// This is a cascading delete.
    pub async fn delete_job_data(&self, job_id: &str) -> Result<(), StorageError> {
        let filter = format!("job_id = '{}'", job_id);

        // Delete windows for this job
        let windows_table = self.open_table(TABLE_WINDOWS).await?;
        windows_table.delete(&filter).await?;

        // Delete pages for this job
        let pages_table = self.open_table(TABLE_PAGES).await?;
        pages_table.delete(&filter).await?;

        // Delete the job record itself
        let jobs_table = self.open_table(TABLE_JOBS).await?;
        jobs_table.delete(&filter).await?;

        Ok(())
    }
}
