use std::collections::HashMap;

use crate::types::{PageSubGrid, SubCellCluster};

/// Maximum number of clusters stored per sub-cell.
const MAX_CLUSTERS_PER_CELL: usize = 8;

/// Input data for a single window used in sub-cell mapping.
pub struct WindowSubCellData {
    pub window_id: String,
    pub page: u32,
    pub char_start: u32,
    pub char_end: u32,
    pub cluster_id: i32,
    pub sim_to_centroid: f32,
}

const SUB_CELL_COUNT: u32 = 400;

/// Map a character offset on a page to a linear sub-cell index in [0, 399].
fn char_offset_to_linear_index(char_offset: u32, page_char_count: u32) -> u32 {
    if page_char_count == 0 {
        return 0;
    }
    let index =
        ((char_offset as f64 / page_char_count as f64) * SUB_CELL_COUNT as f64).floor() as u32;
    index.min(SUB_CELL_COUNT - 1)
}

/// Convert a linear sub-cell index to (row, col) in the 20×20 grid.
fn linear_index_to_row_col(linear_index: u32) -> (u8, u8) {
    let clamped = linear_index.min(SUB_CELL_COUNT - 1);
    ((clamped / 20) as u8, (clamped % 20) as u8)
}

/// Inclusive range of linear sub-cell indices covered by a window's character span.
///
/// `char_end` follows windowing semantics: it is the exclusive end index of the
/// window text slice (`text[char_start..char_end]`).
pub fn compute_sub_cell_span(char_start: u32, char_end: u32, page_char_count: u32) -> (u32, u32) {
    if page_char_count == 0 || char_end <= char_start {
        let idx = char_offset_to_linear_index(char_start, page_char_count.max(1));
        return (idx, idx);
    }

    let last_char = char_end.saturating_sub(1);
    let start = char_offset_to_linear_index(char_start, page_char_count);
    let end = char_offset_to_linear_index(last_char, page_char_count);
    (start.min(end), start.max(end))
}

/// Primary sub-cell for a window (midpoint), used for click targeting and storage.
pub fn compute_sub_cell(char_start: u32, char_end: u32, page_char_count: u32) -> (u8, u8) {
    let midpoint = (char_start + char_end) / 2;
    linear_index_to_row_col(char_offset_to_linear_index(midpoint, page_char_count))
}

/// Build a `PageSubGrid` for each page from the given window data.
///
/// - Excludes noise windows (cluster_id == -1).
/// - Each sub-cell's cluster list is sorted by `sim_to_centroid` descending.
/// - Each sub-cell is capped at 8 cluster entries.
pub fn build_page_sub_grids(
    windows: &[WindowSubCellData],
    page_char_counts: &HashMap<u32, u32>,
) -> Vec<PageSubGrid> {
    // Collect grids keyed by page number.
    let mut grids: HashMap<u32, PageSubGrid> = HashMap::new();

    for w in windows {
        // Exclude noise windows.
        if w.cluster_id == -1 {
            continue;
        }

        // Look up the page's character count; skip if not found.
        let &page_char_count = match page_char_counts.get(&w.page) {
            Some(count) => count,
            None => continue,
        };

        // Paint every sub-cell the window spans (not just its midpoint).
        let (span_start, span_end) =
            compute_sub_cell_span(w.char_start, w.char_end, page_char_count);

        // Get or create the grid for this page.
        let grid = grids
            .entry(w.page)
            .or_insert_with(|| PageSubGrid::new(w.page));

        for linear in span_start..=span_end {
            let (row, col) = linear_index_to_row_col(linear);
            let cell = grid.cell_mut(row as usize, col as usize);
            cell.clusters.push(SubCellCluster {
                cluster_id: w.cluster_id,
                sim_to_centroid: w.sim_to_centroid,
                window_id: w.window_id.clone(),
            });
        }
    }

    // Sort each sub-cell's clusters by sim_to_centroid descending and cap at 8.
    for grid in grids.values_mut() {
        for cell in grid.cells.iter_mut() {
            cell.clusters.sort_by(|a, b| {
                b.sim_to_centroid
                    .partial_cmp(&a.sim_to_centroid)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            cell.clusters.truncate(MAX_CLUSTERS_PER_CELL);
        }
    }

    // Collect into a Vec sorted by page number for deterministic output.
    let mut result: Vec<PageSubGrid> = grids.into_values().collect();
    result.sort_by_key(|g| g.page);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sub_cell_start_of_page() {
        // Window at the very start of a 1000-char page.
        // midpoint = (0 + 10) / 2 = 5
        // linear_index = floor(5 / 1000 * 400) = floor(2.0) = 2
        // row = 2 / 20 = 0, col = 2 % 20 = 2
        let (row, col) = compute_sub_cell(0, 10, 1000);
        assert_eq!(row, 0);
        assert_eq!(col, 2);
    }

    #[test]
    fn test_compute_sub_cell_end_of_page() {
        // Window at the end of a 1000-char page.
        // midpoint = (990 + 1000) / 2 = 995
        // linear_index = floor(995 / 1000 * 400) = floor(398.0) = 398
        // row = 398 / 20 = 19, col = 398 % 20 = 18
        let (row, col) = compute_sub_cell(990, 1000, 1000);
        assert_eq!(row, 19);
        assert_eq!(col, 18);
    }

    #[test]
    fn test_compute_sub_cell_middle_of_page() {
        // Window in the middle of a 400-char page.
        // midpoint = (190 + 210) / 2 = 200
        // linear_index = floor(200 / 400 * 400) = floor(200.0) = 200
        // row = 200 / 20 = 10, col = 200 % 20 = 0
        let (row, col) = compute_sub_cell(190, 210, 400);
        assert_eq!(row, 10);
        assert_eq!(col, 0);
    }

    #[test]
    fn test_compute_sub_cell_clamps_to_399() {
        // If midpoint >= page_char_count, linear_index would exceed 399.
        // midpoint = (1000 + 1010) / 2 = 1005
        // linear_index = floor(1005 / 1000 * 400) = floor(402.0) = 402 → clamped to 399
        // row = 399 / 20 = 19, col = 399 % 20 = 19
        let (row, col) = compute_sub_cell(1000, 1010, 1000);
        assert_eq!(row, 19);
        assert_eq!(col, 19);
    }

    #[test]
    fn test_compute_sub_cell_exact_boundary() {
        // midpoint exactly at page_char_count - 1 for a 400-char page.
        // midpoint = (398 + 400) / 2 = 399
        // linear_index = floor(399 / 400 * 400) = floor(399.0) = 399
        // row = 399 / 20 = 19, col = 399 % 20 = 19
        let (row, col) = compute_sub_cell(398, 400, 400);
        assert_eq!(row, 19);
        assert_eq!(col, 19);
    }

    #[test]
    fn test_noise_windows_excluded() {
        let windows = vec![
            WindowSubCellData {
                window_id: "w1".to_string(),
                page: 1,
                char_start: 0,
                char_end: 10,
                cluster_id: -1, // noise
                sim_to_centroid: 0.9,
            },
            WindowSubCellData {
                window_id: "w2".to_string(),
                page: 1,
                char_start: 0,
                char_end: 10,
                cluster_id: 1, // valid cluster
                sim_to_centroid: 0.8,
            },
        ];

        let mut page_char_counts = HashMap::new();
        page_char_counts.insert(1, 1000);

        let grids = build_page_sub_grids(&windows, &page_char_counts);
        assert_eq!(grids.len(), 1);

        // Only the non-noise window should appear.
        let total_clusters: usize = grids[0].cells.iter().map(|c| c.clusters.len()).sum();
        assert!(total_clusters > 0);
        // The window paints a span of sub-cells, but all entries should be for cluster 1.
        for cell in grids[0].cells.iter().filter(|c| !c.clusters.is_empty()) {
            assert_eq!(cell.clusters[0].cluster_id, 1);
        }
    }

    #[test]
    fn test_clusters_sorted_desc_by_sim_to_centroid() {
        // Place multiple windows in the same sub-cell with different similarities.
        let windows = vec![
            WindowSubCellData {
                window_id: "w1".to_string(),
                page: 1,
                char_start: 0,
                char_end: 10,
                cluster_id: 1,
                sim_to_centroid: 0.5,
            },
            WindowSubCellData {
                window_id: "w2".to_string(),
                page: 1,
                char_start: 0,
                char_end: 10,
                cluster_id: 2,
                sim_to_centroid: 0.9,
            },
            WindowSubCellData {
                window_id: "w3".to_string(),
                page: 1,
                char_start: 0,
                char_end: 10,
                cluster_id: 3,
                sim_to_centroid: 0.7,
            },
        ];

        let mut page_char_counts = HashMap::new();
        page_char_counts.insert(1, 1000);

        let grids = build_page_sub_grids(&windows, &page_char_counts);
        let cell = grids[0]
            .cells
            .iter()
            .find(|c| !c.clusters.is_empty())
            .unwrap();

        assert_eq!(cell.clusters.len(), 3);
        assert_eq!(cell.clusters[0].sim_to_centroid, 0.9);
        assert_eq!(cell.clusters[1].sim_to_centroid, 0.7);
        assert_eq!(cell.clusters[2].sim_to_centroid, 0.5);
    }

    #[test]
    fn test_cap_at_8_clusters() {
        // Place 10 windows in the same sub-cell with distinct similarities.
        let sims = [0.10, 0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 0.95];
        let windows: Vec<WindowSubCellData> = sims
            .iter()
            .enumerate()
            .map(|(i, &sim)| WindowSubCellData {
                window_id: format!("w{}", i),
                page: 1,
                char_start: 0,
                char_end: 10,
                cluster_id: i as i32,
                sim_to_centroid: sim as f32,
            })
            .collect();

        let mut page_char_counts = HashMap::new();
        page_char_counts.insert(1, 1000);

        let grids = build_page_sub_grids(&windows, &page_char_counts);
        let cell = grids[0]
            .cells
            .iter()
            .find(|c| !c.clusters.is_empty())
            .unwrap();

        // Should be capped at 8.
        assert_eq!(cell.clusters.len(), 8);
        // The top 8 by sim_to_centroid desc should be kept.
        assert_eq!(cell.clusters[0].sim_to_centroid, 0.95);
        assert_eq!(cell.clusters[7].sim_to_centroid, 0.30);
        // The two lowest (0.10, 0.20) should be dropped.
    }

    #[test]
    fn test_multiple_pages_handled() {
        let windows = vec![
            WindowSubCellData {
                window_id: "w1".to_string(),
                page: 1,
                char_start: 0,
                char_end: 50,
                cluster_id: 1,
                sim_to_centroid: 0.8,
            },
            WindowSubCellData {
                window_id: "w2".to_string(),
                page: 2,
                char_start: 100,
                char_end: 200,
                cluster_id: 2,
                sim_to_centroid: 0.7,
            },
            WindowSubCellData {
                window_id: "w3".to_string(),
                page: 3,
                char_start: 50,
                char_end: 100,
                cluster_id: 3,
                sim_to_centroid: 0.6,
            },
        ];

        let mut page_char_counts = HashMap::new();
        page_char_counts.insert(1, 500);
        page_char_counts.insert(2, 500);
        page_char_counts.insert(3, 500);

        let grids = build_page_sub_grids(&windows, &page_char_counts);

        // Should produce 3 grids, one per page.
        assert_eq!(grids.len(), 3);
        assert_eq!(grids[0].page, 1);
        assert_eq!(grids[1].page, 2);
        assert_eq!(grids[2].page, 3);

        // Each window paints every sub-cell in its character span.
        let page1_filled = grids[0]
            .cells
            .iter()
            .filter(|c| !c.clusters.is_empty())
            .count();
        assert_eq!(page1_filled, 40); // chars 0..50 on a 500-char page → indices 0..39
    }

    #[test]
    fn test_span_covers_half_page() {
        let page_char_count = 1000;
        let (start, end) = compute_sub_cell_span(0, 500, page_char_count);
        let cells_covered = end - start + 1;
        // ~50% of the page should cover ~200 of 400 sub-cells.
        assert!(
            cells_covered >= 180 && cells_covered <= 220,
            "expected ~half page, got {cells_covered}"
        );
    }

    #[test]
    fn test_span_full_page() {
        let page_char_count = 800;
        let (start, end) = compute_sub_cell_span(0, 800, page_char_count);
        assert_eq!(start, 0);
        assert_eq!(end, 399);
    }

    #[test]
    fn test_all_noise_produces_no_grids() {
        let windows = vec![
            WindowSubCellData {
                window_id: "w1".to_string(),
                page: 1,
                char_start: 0,
                char_end: 10,
                cluster_id: -1,
                sim_to_centroid: 0.9,
            },
            WindowSubCellData {
                window_id: "w2".to_string(),
                page: 1,
                char_start: 50,
                char_end: 60,
                cluster_id: -1,
                sim_to_centroid: 0.8,
            },
        ];

        let mut page_char_counts = HashMap::new();
        page_char_counts.insert(1, 1000);

        let grids = build_page_sub_grids(&windows, &page_char_counts);
        assert_eq!(grids.len(), 0);
    }

    #[test]
    fn test_position_formula_various_positions() {
        // Test several positions across a 2000-char page.
        let page_char_count = 2000;

        // Position at 25% of page:
        // midpoint = (490 + 510) / 2 = 500
        // linear_index = floor(500 / 2000 * 400) = floor(100.0) = 100
        // row = 100 / 20 = 5, col = 100 % 20 = 0
        let (row, col) = compute_sub_cell(490, 510, page_char_count);
        assert_eq!(row, 5);
        assert_eq!(col, 0);

        // Position at 75% of page:
        // midpoint = (1490 + 1510) / 2 = 1500
        // linear_index = floor(1500 / 2000 * 400) = floor(300.0) = 300
        // row = 300 / 20 = 15, col = 300 % 20 = 0
        let (row, col) = compute_sub_cell(1490, 1510, page_char_count);
        assert_eq!(row, 15);
        assert_eq!(col, 0);
    }
}
