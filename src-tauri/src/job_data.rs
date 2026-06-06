//! Load persisted window/page data and build render-ready sub-grids.

use std::collections::HashMap;

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, StringArray, UInt32Array,
};

use crate::centroid::WindowData;
use crate::storage::{Storage, StorageError};
use crate::subcell::{build_page_sub_grids, WindowSubCellData};
use crate::types::PageSubGrid;

/// Page sub-grids and metadata loaded from LanceDB for a completed job.
pub struct JobRenderData {
    pub page_sub_grids: Vec<PageSubGrid>,
    pub page_count: u32,
}

/// Load windows and pages for a job, returning sub-grids for rasterization.
pub async fn load_job_render_data(
    store: &Storage,
    job_id: &str,
) -> Result<JobRenderData, StorageError> {
    let window_batches = store.get_windows_for_job(job_id).await?;
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

        for i in 0..batch.num_rows() {
            subcell_data_list.push(WindowSubCellData {
                window_id: window_ids.value(i).to_string(),
                page: pages_col.value(i),
                char_start: char_starts.value(i),
                char_end: char_ends.value(i),
                cluster_id: cluster_ids.value(i),
                sim_to_centroid: sims.value(i),
            });
        }
    }

    let page_batches = store.get_pages_for_job(job_id).await?;
    let mut page_char_counts: HashMap<u32, u32> = HashMap::new();
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

    let page_sub_grids = build_page_sub_grids(&subcell_data_list, &page_char_counts);

    Ok(JobRenderData {
        page_sub_grids,
        page_count,
    })
}

/// Parse window rows for cluster registry construction (embeddings + spans).
pub fn parse_window_data_from_batches(
    batches: &[arrow_array::RecordBatch],
) -> Vec<WindowData> {
    let mut window_data_list: Vec<WindowData> = Vec::new();

    for batch in batches {
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
        let doc_char_starts = batch
            .column_by_name("doc_char_start")
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
        let embeddings_col = batch
            .column_by_name("embedding")
            .unwrap()
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .unwrap();

        for i in 0..batch.num_rows() {
            let char_start = char_starts.value(i);
            let char_end = char_ends.value(i);
            let doc_char_start = doc_char_starts.value(i);

            let embedding = embeddings_col
                .value(i)
                .as_any()
                .downcast_ref::<Float32Array>()
                .unwrap()
                .values()
                .to_vec();

            window_data_list.push(WindowData {
                window_id: window_ids.value(i).to_string(),
                window_index: window_indices.value(i),
                page: pages_col.value(i),
                cluster_id: cluster_ids.value(i),
                embedding,
                text: texts.value(i).to_string(),
                doc_char_start,
                doc_char_end: doc_char_start + char_end.saturating_sub(char_start),
            });
        }
    }

    window_data_list
}
