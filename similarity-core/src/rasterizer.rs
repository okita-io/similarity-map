/// Canvas Rasterizer module.
///
/// Iterates the 20×20 sub-cell grid for each page, applies threshold/gamma/hidden
/// filtering via `blend_sub_cell`, and produces a 1600-byte RGBA pixel array.
/// Also provides batch rasterization and base64 encoding for the page-ready event.
use std::collections::HashSet;

use base64::{engine::general_purpose::STANDARD, Engine as _};

use crate::color::blend_sub_cell;
use crate::types::{PageCanvas, PageSubGrid};

/// Rasterize a single page's sub-cell grid into a 1600-byte RGBA canvas.
///
/// Iterates the 20×20 grid in row-major order. For each cell, calls
/// `blend_sub_cell` to produce the RGBA pixel value, applying threshold,
/// gamma, and hidden_clusters filtering.
pub fn rasterize_page(
    grid: &PageSubGrid,
    gamma: f32,
    threshold: f32,
    hidden: &HashSet<i32>,
) -> PageCanvas {
    let mut canvas = PageCanvas::new(grid.page);

    for row in 0..PageSubGrid::GRID_SIZE {
        for col in 0..PageSubGrid::GRID_SIZE {
            let sub_cell = grid.cell(row, col);
            let color = blend_sub_cell(&sub_cell.clusters, gamma, threshold, hidden);
            let offset = (row * PageSubGrid::GRID_SIZE + col) * 4;
            canvas.pixels[offset..offset + 4].copy_from_slice(&color);
        }
    }

    canvas
}

/// Rasterize multiple pages in sequence.
///
/// Returns one `PageCanvas` per input grid, preserving order.
pub fn rasterize_pages(
    grids: &[PageSubGrid],
    gamma: f32,
    threshold: f32,
    hidden: &HashSet<i32>,
) -> Vec<PageCanvas> {
    grids
        .iter()
        .map(|grid| rasterize_page(grid, gamma, threshold, hidden))
        .collect()
}

/// Rasterize only the specified pages from a set of pre-computed sub-grids.
///
/// Filters `grids` to include only those whose `page` field appears in `pages`,
/// then rasterizes each with the given gamma, threshold, and hidden clusters.
/// Returns `PageCanvas` results in the order they appear in `grids` (ascending page number
/// if grids are sorted).
///
/// This is the core logic behind the `raster_pages` Tauri command, enabling
/// targeted re-rasterization when cluster filters or gamma change without
/// re-running the full pipeline.
pub fn rasterize_selected_pages(
    grids: &[PageSubGrid],
    pages: &[u32],
    gamma: f32,
    threshold: f32,
    hidden: &HashSet<i32>,
) -> Vec<PageCanvas> {
    let page_set: HashSet<u32> = pages.iter().copied().collect();
    grids
        .iter()
        .filter(|grid| page_set.contains(&grid.page))
        .map(|grid| rasterize_page(grid, gamma, threshold, hidden))
        .collect()
}

/// Encode a PageCanvas's pixel buffer as a standard base64 string.
///
/// Used to produce the `canvas_rgba_b64` payload for the
/// `similarity-map:page-ready` event.
pub fn encode_canvas_base64(canvas: &PageCanvas) -> String {
    STANDARD.encode(&canvas.pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::EMPTY_BG_RGBA;
    use crate::types::SubCellCluster;

    /// Helper: create an empty 20×20 grid for a given page.
    fn empty_grid(page: u32) -> PageSubGrid {
        PageSubGrid::new(page)
    }

    /// Helper: create a grid with a single cluster at a specific cell.
    fn grid_with_cluster_at(
        page: u32,
        row: usize,
        col: usize,
        cluster_id: i32,
        sim: f32,
    ) -> PageSubGrid {
        let mut grid = PageSubGrid::new(page);
        grid.cell_mut(row, col).clusters.push(SubCellCluster {
            cluster_id,
            sim_to_centroid: sim,
            window_id: format!("win-{cluster_id}"),
        });
        grid
    }

    // --- rasterize_page tests ---

    #[test]
    fn test_empty_grid_produces_all_transparent() {
        let grid = empty_grid(1);
        let canvas = rasterize_page(&grid, 1.5, 0.75, &HashSet::new());

        // Every pixel should be the background color (no matches).
        assert_eq!(canvas.page, 1);
        assert_eq!(canvas.pixels.len(), PageCanvas::PIXEL_BYTE_LEN);
        for chunk in canvas.pixels.chunks(4) {
            assert_eq!(chunk, &EMPTY_BG_RGBA, "expected background pixel");
        }
    }

    #[test]
    fn test_canvas_is_exactly_1600_bytes() {
        let grid = empty_grid(5);
        let canvas = rasterize_page(&grid, 1.5, 0.75, &HashSet::new());
        assert_eq!(canvas.pixels.len(), 1600);
    }

    #[test]
    fn test_pixel_at_correct_offset_for_row_col() {
        // Place a cluster at row=3, col=7
        let grid = grid_with_cluster_at(1, 3, 7, 5, 0.9);
        let canvas = rasterize_page(&grid, 1.5, 0.75, &HashSet::new());

        let offset = (3 * 20 + 7) * 4;

        // The pixel at (3, 7) should be non-transparent (alpha = 255)
        assert_eq!(
            canvas.pixels[offset + 3],
            255,
            "alpha should be 255 for valid cluster"
        );

        // Verify a neighboring empty cell is transparent
        let empty_offset = (3 * 20 + 8) * 4;
        assert_eq!(
            &canvas.pixels[empty_offset..empty_offset + 4],
            &EMPTY_BG_RGBA
        );
    }

    #[test]
    fn test_pixel_value_matches_blend_sub_cell() {
        let cluster_id = 2;
        let sim = 0.85;
        let gamma = 1.5;
        let threshold = 0.75;
        let hidden = HashSet::new();

        let grid = grid_with_cluster_at(1, 10, 15, cluster_id, sim);
        let canvas = rasterize_page(&grid, gamma, threshold, &hidden);

        // Compute expected color directly
        let clusters = vec![SubCellCluster {
            cluster_id,
            sim_to_centroid: sim,
            window_id: "win-2".to_string(),
        }];
        let expected = blend_sub_cell(&clusters, gamma, threshold, &hidden);

        let offset = (10 * 20 + 15) * 4;
        assert_eq!(&canvas.pixels[offset..offset + 4], &expected);
    }

    #[test]
    fn test_hidden_clusters_produce_transparent() {
        let mut hidden = HashSet::new();
        hidden.insert(5);

        let grid = grid_with_cluster_at(1, 0, 0, 5, 0.95);
        let canvas = rasterize_page(&grid, 1.5, 0.75, &hidden);

        // Cluster 5 is hidden, so pixel should be background
        assert_eq!(&canvas.pixels[0..4], &EMPTY_BG_RGBA);
    }

    #[test]
    fn test_below_threshold_produces_transparent() {
        // Cluster with sim=0.70, threshold=0.75 → below threshold
        let grid = grid_with_cluster_at(1, 5, 5, 3, 0.70);
        let canvas = rasterize_page(&grid, 1.5, 0.75, &HashSet::new());

        let offset = (5 * 20 + 5) * 4;
        assert_eq!(&canvas.pixels[offset..offset + 4], &EMPTY_BG_RGBA);
    }

    // --- rasterize_pages tests ---

    #[test]
    fn test_multiple_pages_rasterized_correctly() {
        let grids = vec![
            grid_with_cluster_at(1, 0, 0, 1, 0.9),
            grid_with_cluster_at(2, 19, 19, 2, 0.85),
            empty_grid(3),
        ];

        let canvases = rasterize_pages(&grids, 1.5, 0.75, &HashSet::new());

        assert_eq!(canvases.len(), 3);
        assert_eq!(canvases[0].page, 1);
        assert_eq!(canvases[1].page, 2);
        assert_eq!(canvases[2].page, 3);

        // Page 1: pixel at (0,0) should be non-transparent
        assert_eq!(canvases[0].pixels[3], 255);

        // Page 2: pixel at (19,19) should be non-transparent
        let offset = (19 * 20 + 19) * 4;
        assert_eq!(canvases[1].pixels[offset + 3], 255);

        // Page 3: all transparent
        for chunk in canvases[2].pixels.chunks(4) {
            assert_eq!(chunk, &EMPTY_BG_RGBA);
        }
    }

    #[test]
    fn test_rasterize_pages_empty_input() {
        let canvases = rasterize_pages(&[], 1.5, 0.75, &HashSet::new());
        assert!(canvases.is_empty());
    }

    // --- encode_canvas_base64 tests ---

    #[test]
    fn test_encode_canvas_base64_produces_valid_base64() {
        let canvas = PageCanvas::new(1);
        let encoded = encode_canvas_base64(&canvas);

        // Should be decodable
        let decoded = STANDARD.decode(&encoded).expect("should be valid base64");
        assert_eq!(decoded.len(), 1600);
        assert_eq!(decoded, canvas.pixels);
    }

    #[test]
    fn test_encode_canvas_base64_non_empty() {
        let mut canvas = PageCanvas::new(1);
        // Set some non-zero pixels
        canvas.pixels[0] = 255;
        canvas.pixels[1] = 128;
        canvas.pixels[2] = 64;
        canvas.pixels[3] = 255;

        let encoded = encode_canvas_base64(&canvas);
        let decoded = STANDARD.decode(&encoded).expect("should be valid base64");
        assert_eq!(decoded[0], 255);
        assert_eq!(decoded[1], 128);
        assert_eq!(decoded[2], 64);
        assert_eq!(decoded[3], 255);
    }

    #[test]
    fn test_encode_canvas_base64_length() {
        let canvas = PageCanvas::new(1);
        let encoded = encode_canvas_base64(&canvas);
        // base64 of 1600 bytes: ceil(1600/3)*4 = 534*4 = 2136 chars (no padding needed since 1600 % 3 == 2, so 2136 + padding)
        // Actually: 1600 / 3 = 533.33, so ceil = 534 groups → 534 * 4 = 2136 chars with padding
        assert!(!encoded.is_empty());
        // Verify round-trip length
        let decoded = STANDARD.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), 1600);
    }

    // --- rasterize_selected_pages tests ---

    #[test]
    fn test_selected_pages_filters_correctly() {
        let grids = vec![
            grid_with_cluster_at(1, 0, 0, 1, 0.9),
            grid_with_cluster_at(2, 5, 5, 2, 0.85),
            grid_with_cluster_at(3, 10, 10, 3, 0.8),
            grid_with_cluster_at(4, 15, 15, 4, 0.75),
            empty_grid(5),
        ];

        // Request only pages 2 and 4
        let canvases = rasterize_selected_pages(&grids, &[2, 4], 1.5, 0.75, &HashSet::new());

        assert_eq!(canvases.len(), 2);
        assert_eq!(canvases[0].page, 2);
        assert_eq!(canvases[1].page, 4);
    }

    #[test]
    fn test_selected_pages_empty_page_list() {
        let grids = vec![
            grid_with_cluster_at(1, 0, 0, 1, 0.9),
            grid_with_cluster_at(2, 5, 5, 2, 0.85),
        ];

        let canvases = rasterize_selected_pages(&grids, &[], 1.5, 0.75, &HashSet::new());
        assert!(canvases.is_empty());
    }

    #[test]
    fn test_selected_pages_empty_grids() {
        let canvases = rasterize_selected_pages(&[], &[1, 2, 3], 1.5, 0.75, &HashSet::new());
        assert!(canvases.is_empty());
    }

    #[test]
    fn test_selected_pages_nonexistent_pages_ignored() {
        let grids = vec![
            grid_with_cluster_at(1, 0, 0, 1, 0.9),
            grid_with_cluster_at(3, 10, 10, 3, 0.8),
        ];

        // Request pages 2 and 4 which don't exist in grids
        let canvases = rasterize_selected_pages(&grids, &[2, 4], 1.5, 0.75, &HashSet::new());
        assert!(canvases.is_empty());
    }

    #[test]
    fn test_selected_pages_applies_hidden_clusters() {
        let grids = vec![
            grid_with_cluster_at(1, 0, 0, 5, 0.95),
            grid_with_cluster_at(2, 0, 0, 6, 0.90),
        ];

        let mut hidden = HashSet::new();
        hidden.insert(5);

        let canvases = rasterize_selected_pages(&grids, &[1, 2], 1.5, 0.75, &hidden);

        assert_eq!(canvases.len(), 2);
        // Page 1 has cluster 5 which is hidden → transparent
        assert_eq!(&canvases[0].pixels[0..4], &EMPTY_BG_RGBA);
        // Page 2 has cluster 6 which is not hidden → non-transparent
        assert_eq!(canvases[1].pixels[3], 255);
    }

    #[test]
    fn test_selected_pages_applies_threshold() {
        let grids = vec![
            grid_with_cluster_at(1, 0, 0, 1, 0.70), // below threshold
            grid_with_cluster_at(2, 0, 0, 2, 0.90), // above threshold
        ];

        let canvases = rasterize_selected_pages(&grids, &[1, 2], 1.5, 0.75, &HashSet::new());

        assert_eq!(canvases.len(), 2);
        // Page 1: sim 0.70 < threshold 0.75 → transparent
        assert_eq!(&canvases[0].pixels[0..4], &EMPTY_BG_RGBA);
        // Page 2: sim 0.90 >= threshold 0.75 → non-transparent
        assert_eq!(canvases[1].pixels[3], 255);
    }

    #[test]
    fn test_selected_pages_applies_gamma() {
        let grids = vec![grid_with_cluster_at(1, 0, 0, 1, 0.9)];

        let canvas_low_gamma = rasterize_selected_pages(&grids, &[1], 0.5, 0.75, &HashSet::new());
        let canvas_high_gamma = rasterize_selected_pages(&grids, &[1], 3.0, 0.75, &HashSet::new());

        // Both should produce non-transparent pixels but with different color values
        assert_eq!(canvas_low_gamma[0].pixels[3], 255);
        assert_eq!(canvas_high_gamma[0].pixels[3], 255);

        // The RGB values should differ due to different gamma
        // Low gamma → brighter (higher value), high gamma → darker (lower value)
        // Compare the first RGB channel (any will do)
        let low_r = canvas_low_gamma[0].pixels[0];
        let high_r = canvas_high_gamma[0].pixels[0];
        // With gamma 0.5, 0.9^0.5 ≈ 0.949 (brighter)
        // With gamma 3.0, 0.9^3.0 ≈ 0.729 (darker)
        // So low_gamma should produce brighter (higher) pixel values
        assert!(
            low_r >= high_r,
            "low gamma should produce brighter pixels: {} vs {}",
            low_r,
            high_r
        );
    }

    #[test]
    fn test_selected_pages_preserves_grid_order() {
        // Grids are in page order 1, 2, 3, 4, 5
        let grids = vec![
            grid_with_cluster_at(1, 0, 0, 1, 0.9),
            grid_with_cluster_at(2, 0, 0, 2, 0.9),
            grid_with_cluster_at(3, 0, 0, 3, 0.9),
            grid_with_cluster_at(4, 0, 0, 4, 0.9),
            grid_with_cluster_at(5, 0, 0, 5, 0.9),
        ];

        // Request pages in non-sequential order
        let canvases = rasterize_selected_pages(&grids, &[5, 2, 4], 1.5, 0.75, &HashSet::new());

        // Results should be in grid order (2, 4, 5), not request order
        assert_eq!(canvases.len(), 3);
        assert_eq!(canvases[0].page, 2);
        assert_eq!(canvases[1].page, 4);
        assert_eq!(canvases[2].page, 5);
    }

    #[test]
    fn test_selected_pages_duplicate_page_numbers() {
        let grids = vec![
            grid_with_cluster_at(1, 0, 0, 1, 0.9),
            grid_with_cluster_at(2, 0, 0, 2, 0.9),
        ];

        // Duplicate page numbers in request should not produce duplicate results
        let canvases = rasterize_selected_pages(&grids, &[1, 1, 2, 2], 1.5, 0.75, &HashSet::new());

        assert_eq!(canvases.len(), 2);
        assert_eq!(canvases[0].page, 1);
        assert_eq!(canvases[1].page, 2);
    }

    #[test]
    fn test_selected_pages_single_page() {
        let grids = vec![
            grid_with_cluster_at(1, 3, 7, 10, 0.88),
            grid_with_cluster_at(2, 0, 0, 2, 0.9),
            grid_with_cluster_at(3, 19, 19, 3, 0.8),
        ];

        let canvases = rasterize_selected_pages(&grids, &[2], 1.5, 0.75, &HashSet::new());

        assert_eq!(canvases.len(), 1);
        assert_eq!(canvases[0].page, 2);
        // Pixel at (0,0) should be non-transparent for page 2
        assert_eq!(canvases[0].pixels[3], 255);
    }
}
