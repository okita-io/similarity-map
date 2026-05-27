/// HSV Color Mapper module.
///
/// Assigns hue (cluster identity via golden-ratio distribution), saturation (fixed 1.0),
/// and value (proximity to centroid raised to gamma) to each cluster, then converts
/// through HSV → linear RGB → sRGB for final pixel output.

use std::collections::HashSet;

use crate::types::SubCellCluster;

/// Golden-ratio hue assignment for a cluster.
/// Returns a value in [0, 1).
pub fn cluster_hue(cluster_id: i32) -> f32 {
    ((cluster_id as f64 * 0.6180339887) % 1.0) as f32
}

/// Compute the V (value) channel from similarity to centroid.
/// Clamps negative similarities to zero, then raises to gamma.
pub fn compute_value(sim_to_centroid: f32, gamma: f32) -> f32 {
    sim_to_centroid.max(0.0).powf(gamma)
}

/// Full cluster color computation in linear RGB.
/// Returns (R, G, B) in linear space, each in [0, 1].
pub fn cluster_color(cluster_id: i32, sim_to_centroid: f32, gamma: f32) -> (f32, f32, f32) {
    let h = cluster_hue(cluster_id);
    let s = 1.0;
    let v = compute_value(sim_to_centroid, gamma);
    hsv_to_linear_rgb(h, s, v)
}

/// Convert HSV to linear RGB.
///
/// H is in [0, 1), S and V in [0, 1].
/// Uses the standard sector-based algorithm.
pub fn hsv_to_linear_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let h_prime = h * 6.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = if h_prime < 1.0 {
        (c, x, 0.0)
    } else if h_prime < 2.0 {
        (x, c, 0.0)
    } else if h_prime < 3.0 {
        (0.0, c, x)
    } else if h_prime < 4.0 {
        (0.0, x, c)
    } else if h_prime < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (r1 + m, g1 + m, b1 + m)
}

/// Convert a single linear RGB channel value to sRGB.
///
/// Applies the standard sRGB gamma curve:
/// - If c <= 0.0031308: 12.92 * c
/// - Else: 1.055 * c^(1/2.4) - 0.055
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Convert linear RGB + alpha to sRGB RGBA bytes [0..255].
///
/// Each linear channel is gamma-corrected via `linear_to_srgb`, then scaled to [0, 255].
/// Alpha is passed through directly (already in [0, 1] range).
pub fn linear_to_srgb_rgba(r: f32, g: f32, b: f32, a: f32) -> [u8; 4] {
    let to_byte = |c: f32| -> u8 {
        (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8
    };
    [
        to_byte(r),
        to_byte(g),
        to_byte(b),
        (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
    ]
}

/// Blend colors for a sub-cell containing one or more cluster entries.
///
/// Filters out clusters below the similarity threshold and hidden clusters,
/// caps at 8 visible clusters (already sorted by sim_to_centroid desc in SubCell),
/// computes similarity-weighted linear RGB blend, and converts to sRGB RGBA.
///
/// Returns `[0, 0, 0, 0]` (transparent) for empty, all-below-threshold,
/// or all-hidden sub-cells.
pub fn blend_sub_cell(
    clusters: &[SubCellCluster],
    gamma: f32,
    threshold: f32,
    hidden: &HashSet<i32>,
) -> [u8; 4] {
    let visible: Vec<_> = clusters
        .iter()
        .filter(|c| c.sim_to_centroid >= threshold && !hidden.contains(&c.cluster_id))
        .take(8)
        .collect();

    if visible.is_empty() {
        return [0, 0, 0, 0];
    }

    let mut r = 0.0f32;
    let mut g = 0.0f32;
    let mut b = 0.0f32;
    let mut total_weight = 0.0f32;

    for c in &visible {
        let weight = c.sim_to_centroid.powf(gamma);
        let (cr, cg, cb) = cluster_color(c.cluster_id, c.sim_to_centroid, gamma);
        r += weight * cr;
        g += weight * cg;
        b += weight * cb;
        total_weight += weight;
    }

    if total_weight == 0.0 {
        return [0, 0, 0, 0];
    }

    linear_to_srgb_rgba(r / total_weight, g / total_weight, b / total_weight, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- cluster_hue tests ---

    #[test]
    fn test_cluster_hue_zero() {
        let h = cluster_hue(0);
        assert!((h - 0.0).abs() < 1e-6, "cluster 0 hue should be 0.0, got {h}");
    }

    #[test]
    fn test_cluster_hue_one() {
        let h = cluster_hue(1);
        let expected = 0.6180339887_f32;
        assert!(
            (h - expected).abs() < 1e-5,
            "cluster 1 hue should be ~0.618, got {h}"
        );
    }

    #[test]
    fn test_cluster_hue_two() {
        let h = cluster_hue(2);
        // (2 * 0.6180339887) mod 1.0 = 1.2360679774 mod 1.0 = 0.2360679774
        let expected = 0.2360679774_f32;
        assert!(
            (h - expected).abs() < 1e-5,
            "cluster 2 hue should be ~0.236, got {h}"
        );
    }

    #[test]
    fn test_cluster_hue_large_id() {
        let h = cluster_hue(100);
        // (100 * 0.6180339887) mod 1.0 = 61.80339887 mod 1.0 = 0.80339887
        let expected = ((100.0_f64 * 0.6180339887) % 1.0) as f32;
        assert!(
            (h - expected).abs() < 1e-5,
            "cluster 100 hue should be ~{expected}, got {h}"
        );
    }

    #[test]
    fn test_cluster_hue_always_in_range() {
        for id in 0..1000 {
            let h = cluster_hue(id);
            assert!(h >= 0.0 && h < 1.0, "hue out of range for id {id}: {h}");
        }
    }

    // --- compute_value tests ---

    #[test]
    fn test_compute_value_positive_sim() {
        let v = compute_value(0.8, 1.5);
        let expected = 0.8_f32.powf(1.5);
        assert!(
            (v - expected).abs() < 1e-6,
            "expected {expected}, got {v}"
        );
    }

    #[test]
    fn test_compute_value_zero_sim() {
        let v = compute_value(0.0, 1.5);
        assert!((v - 0.0).abs() < 1e-6, "expected 0.0, got {v}");
    }

    #[test]
    fn test_compute_value_negative_sim_clamped() {
        let v = compute_value(-0.5, 1.5);
        assert!((v - 0.0).abs() < 1e-6, "negative sim should clamp to 0, got {v}");
    }

    #[test]
    fn test_compute_value_sim_one_any_gamma() {
        let v = compute_value(1.0, 2.0);
        assert!((v - 1.0).abs() < 1e-6, "sim=1.0 should give value=1.0, got {v}");
    }

    #[test]
    fn test_compute_value_gamma_one() {
        let v = compute_value(0.6, 1.0);
        assert!((v - 0.6).abs() < 1e-6, "gamma=1.0 should pass through, got {v}");
    }

    // --- hsv_to_linear_rgb tests ---

    #[test]
    fn test_hsv_pure_red() {
        // H=0, S=1, V=1 → (1, 0, 0)
        let (r, g, b) = hsv_to_linear_rgb(0.0, 1.0, 1.0);
        assert!((r - 1.0).abs() < 1e-5, "red channel: {r}");
        assert!((g - 0.0).abs() < 1e-5, "green channel: {g}");
        assert!((b - 0.0).abs() < 1e-5, "blue channel: {b}");
    }

    #[test]
    fn test_hsv_pure_green() {
        // H=1/3, S=1, V=1 → (0, 1, 0)
        let (r, g, b) = hsv_to_linear_rgb(1.0 / 3.0, 1.0, 1.0);
        assert!((r - 0.0).abs() < 1e-5, "red channel: {r}");
        assert!((g - 1.0).abs() < 1e-5, "green channel: {g}");
        assert!((b - 0.0).abs() < 1e-5, "blue channel: {b}");
    }

    #[test]
    fn test_hsv_pure_blue() {
        // H=2/3, S=1, V=1 → (0, 0, 1)
        let (r, g, b) = hsv_to_linear_rgb(2.0 / 3.0, 1.0, 1.0);
        assert!((r - 0.0).abs() < 1e-5, "red channel: {r}");
        assert!((g - 0.0).abs() < 1e-5, "green channel: {g}");
        assert!((b - 1.0).abs() < 1e-5, "blue channel: {b}");
    }

    #[test]
    fn test_hsv_white() {
        // H=0, S=0, V=1 → (1, 1, 1)
        let (r, g, b) = hsv_to_linear_rgb(0.0, 0.0, 1.0);
        assert!((r - 1.0).abs() < 1e-5, "red: {r}");
        assert!((g - 1.0).abs() < 1e-5, "green: {g}");
        assert!((b - 1.0).abs() < 1e-5, "blue: {b}");
    }

    #[test]
    fn test_hsv_black() {
        // H=0, S=1, V=0 → (0, 0, 0)
        let (r, g, b) = hsv_to_linear_rgb(0.0, 1.0, 0.0);
        assert!((r - 0.0).abs() < 1e-5, "red: {r}");
        assert!((g - 0.0).abs() < 1e-5, "green: {g}");
        assert!((b - 0.0).abs() < 1e-5, "blue: {b}");
    }

    #[test]
    fn test_hsv_yellow() {
        // H=1/6, S=1, V=1 → (1, 1, 0)
        let (r, g, b) = hsv_to_linear_rgb(1.0 / 6.0, 1.0, 1.0);
        assert!((r - 1.0).abs() < 1e-5, "red: {r}");
        assert!((g - 1.0).abs() < 1e-5, "green: {g}");
        assert!((b - 0.0).abs() < 1e-5, "blue: {b}");
    }

    #[test]
    fn test_hsv_cyan() {
        // H=0.5, S=1, V=1 → (0, 1, 1)
        let (r, g, b) = hsv_to_linear_rgb(0.5, 1.0, 1.0);
        assert!((r - 0.0).abs() < 1e-5, "red: {r}");
        assert!((g - 1.0).abs() < 1e-5, "green: {g}");
        assert!((b - 1.0).abs() < 1e-5, "blue: {b}");
    }

    #[test]
    fn test_hsv_magenta() {
        // H=5/6, S=1, V=1 → (1, 0, 1)
        let (r, g, b) = hsv_to_linear_rgb(5.0 / 6.0, 1.0, 1.0);
        assert!((r - 1.0).abs() < 1e-5, "red: {r}");
        assert!((g - 0.0).abs() < 1e-5, "green: {g}");
        assert!((b - 1.0).abs() < 1e-5, "blue: {b}");
    }

    // --- linear_to_srgb tests ---

    #[test]
    fn test_linear_to_srgb_zero() {
        assert!((linear_to_srgb(0.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_linear_to_srgb_one() {
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_linear_to_srgb_low_value() {
        // Below threshold: 12.92 * c
        let c = 0.001;
        let expected = 12.92 * c;
        assert!(
            (linear_to_srgb(c) - expected).abs() < 1e-6,
            "low value sRGB conversion"
        );
    }

    #[test]
    fn test_linear_to_srgb_mid_value() {
        // Above threshold: 1.055 * c^(1/2.4) - 0.055
        let c = 0.5;
        let expected = 1.055 * (c as f32).powf(1.0 / 2.4) - 0.055;
        assert!(
            (linear_to_srgb(c) - expected).abs() < 1e-5,
            "mid value sRGB conversion"
        );
    }

    #[test]
    fn test_linear_to_srgb_threshold_boundary() {
        // At the threshold, both formulas should give approximately the same result
        let c = 0.0031308_f32;
        let linear_result = 12.92 * c;
        let gamma_result = 1.055 * c.powf(1.0 / 2.4) - 0.055;
        assert!(
            (linear_result - gamma_result).abs() < 1e-4,
            "threshold boundary: linear={linear_result}, gamma={gamma_result}"
        );
    }

    // --- linear_to_srgb_rgba tests ---

    #[test]
    fn test_linear_to_srgb_rgba_black_opaque() {
        let rgba = linear_to_srgb_rgba(0.0, 0.0, 0.0, 1.0);
        assert_eq!(rgba, [0, 0, 0, 255]);
    }

    #[test]
    fn test_linear_to_srgb_rgba_white_opaque() {
        let rgba = linear_to_srgb_rgba(1.0, 1.0, 1.0, 1.0);
        assert_eq!(rgba, [255, 255, 255, 255]);
    }

    #[test]
    fn test_linear_to_srgb_rgba_transparent() {
        let rgba = linear_to_srgb_rgba(0.0, 0.0, 0.0, 0.0);
        assert_eq!(rgba, [0, 0, 0, 0]);
    }

    #[test]
    fn test_alpha_255_for_valid_cluster() {
        // A valid cluster (cluster_id >= 0) with positive similarity should have alpha = 255
        let (r, g, b) = cluster_color(5, 0.9, 1.5);
        let rgba = linear_to_srgb_rgba(r, g, b, 1.0);
        assert_eq!(rgba[3], 255, "valid cluster should have alpha=255");
    }

    #[test]
    fn test_alpha_0_for_noise() {
        // Noise/empty sub-cells get alpha = 0
        let rgba = linear_to_srgb_rgba(0.0, 0.0, 0.0, 0.0);
        assert_eq!(rgba[3], 0, "noise/empty should have alpha=0");
    }

    // --- cluster_color integration tests ---

    #[test]
    fn test_cluster_color_produces_valid_rgb() {
        for id in 0..50 {
            let (r, g, b) = cluster_color(id, 0.85, 1.5);
            assert!(r >= 0.0 && r <= 1.0, "r out of range for cluster {id}: {r}");
            assert!(g >= 0.0 && g <= 1.0, "g out of range for cluster {id}: {g}");
            assert!(b >= 0.0 && b <= 1.0, "b out of range for cluster {id}: {b}");
        }
    }

    #[test]
    fn test_cluster_color_zero_sim_is_black() {
        let (r, g, b) = cluster_color(3, 0.0, 1.5);
        assert!((r - 0.0).abs() < 1e-6);
        assert!((g - 0.0).abs() < 1e-6);
        assert!((b - 0.0).abs() < 1e-6);
    }

    // --- blend_sub_cell tests ---

    fn make_cluster(cluster_id: i32, sim: f32) -> SubCellCluster {
        SubCellCluster {
            cluster_id,
            sim_to_centroid: sim,
            window_id: format!("win-{cluster_id}"),
        }
    }

    #[test]
    fn test_blend_single_cluster_matches_direct_conversion() {
        // Single cluster should produce the same result as direct HSV→sRGB
        let clusters = vec![make_cluster(5, 0.9)];
        let gamma = 1.5;
        let threshold = 0.75;
        let hidden = HashSet::new();

        let result = blend_sub_cell(&clusters, gamma, threshold, &hidden);

        // Direct conversion for comparison
        let (r, g, b) = cluster_color(5, 0.9, gamma);
        let expected = linear_to_srgb_rgba(r, g, b, 1.0);

        assert_eq!(result, expected);
        assert_eq!(result[3], 255, "alpha should be fully opaque");
    }

    #[test]
    fn test_blend_multiple_clusters_weighted() {
        // Two clusters with different similarities → weighted blend
        let clusters = vec![
            make_cluster(1, 0.95),
            make_cluster(2, 0.80),
        ];
        let gamma = 1.5;
        let threshold = 0.75;
        let hidden = HashSet::new();

        let result = blend_sub_cell(&clusters, gamma, threshold, &hidden);

        // Manually compute expected blend
        let w1 = 0.95_f32.powf(gamma);
        let w2 = 0.80_f32.powf(gamma);
        let total = w1 + w2;

        let (r1, g1, b1) = cluster_color(1, 0.95, gamma);
        let (r2, g2, b2) = cluster_color(2, 0.80, gamma);

        let blended_r = (w1 * r1 + w2 * r2) / total;
        let blended_g = (w1 * g1 + w2 * g2) / total;
        let blended_b = (w1 * b1 + w2 * b2) / total;

        let expected = linear_to_srgb_rgba(blended_r, blended_g, blended_b, 1.0);
        assert_eq!(result, expected);
        assert_eq!(result[3], 255);
    }

    #[test]
    fn test_blend_empty_clusters_transparent() {
        let clusters: Vec<SubCellCluster> = vec![];
        let result = blend_sub_cell(&clusters, 1.5, 0.75, &HashSet::new());
        assert_eq!(result, [0, 0, 0, 0]);
    }

    #[test]
    fn test_blend_below_threshold_transparent() {
        // All clusters below threshold → transparent
        let clusters = vec![
            make_cluster(1, 0.70),
            make_cluster(2, 0.60),
        ];
        let result = blend_sub_cell(&clusters, 1.5, 0.75, &HashSet::new());
        assert_eq!(result, [0, 0, 0, 0]);
    }

    #[test]
    fn test_blend_hidden_clusters_excluded() {
        // Two clusters, one hidden → only the visible one contributes
        let clusters = vec![
            make_cluster(1, 0.95),
            make_cluster(2, 0.90),
        ];
        let gamma = 1.5;
        let threshold = 0.75;
        let mut hidden = HashSet::new();
        hidden.insert(1);

        let result = blend_sub_cell(&clusters, gamma, threshold, &hidden);

        // Only cluster 2 should contribute (same as single-cluster case)
        let (r, g, b) = cluster_color(2, 0.90, gamma);
        let expected = linear_to_srgb_rgba(r, g, b, 1.0);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_blend_all_hidden_transparent() {
        let clusters = vec![
            make_cluster(1, 0.95),
            make_cluster(2, 0.90),
        ];
        let mut hidden = HashSet::new();
        hidden.insert(1);
        hidden.insert(2);

        let result = blend_sub_cell(&clusters, 1.5, 0.75, &hidden);
        assert_eq!(result, [0, 0, 0, 0]);
    }

    #[test]
    fn test_blend_zero_total_weight_transparent() {
        // sim_to_centroid = 0.0 passes threshold of 0.0 but weight = 0^gamma = 0
        let clusters = vec![make_cluster(1, 0.0)];
        let result = blend_sub_cell(&clusters, 1.5, 0.0, &HashSet::new());
        assert_eq!(result, [0, 0, 0, 0]);
    }

    #[test]
    fn test_blend_caps_at_8_clusters() {
        // Provide 10 clusters, only first 8 should be used
        let clusters: Vec<_> = (0..10)
            .map(|i| make_cluster(i, 0.95 - i as f32 * 0.01))
            .collect();
        let gamma = 1.5;
        let threshold = 0.75;
        let hidden = HashSet::new();

        let result = blend_sub_cell(&clusters, gamma, threshold, &hidden);

        // Compute expected using only first 8
        let mut r = 0.0f32;
        let mut g = 0.0f32;
        let mut b = 0.0f32;
        let mut total = 0.0f32;
        for c in clusters.iter().take(8) {
            let w = c.sim_to_centroid.powf(gamma);
            let (cr, cg, cb) = cluster_color(c.cluster_id, c.sim_to_centroid, gamma);
            r += w * cr;
            g += w * cg;
            b += w * cb;
            total += w;
        }
        let expected = linear_to_srgb_rgba(r / total, g / total, b / total, 1.0);
        assert_eq!(result, expected);
    }
}
