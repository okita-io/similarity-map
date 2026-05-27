use std::collections::HashMap;

use hdbscan::{Hdbscan, HdbscanHyperParams};
use linfa::traits::Fit;
use linfa::prelude::Predict;
use linfa::DatasetBase;
use linfa_clustering::KMeans;
use ndarray::Array2;
use rand_chacha::ChaCha8Rng;
use rand::SeedableRng;

use crate::types::{AppError, ClusteringError, ValidationError};

/// Derives the HDBSCAN `min_cluster_size` from user-facing parameters.
///
/// Formula: `min_repetitions × max(1, floor(phrase_length / stride))`
///
/// This converts the intuitive "minimum repetitions" concept into the
/// density parameter HDBSCAN needs, accounting for the fact that a single
/// repeated phrase generates multiple overlapping windows.
pub fn derive_min_cluster_size(min_repetitions: u32, phrase_length: u32, stride: u32) -> u32 {
    let windows_per_phrase = (phrase_length / stride).max(1);
    min_repetitions * windows_per_phrase
}

/// Validates clustering parameters are within acceptable ranges.
///
/// - `min_repetitions`: must be 2–20
/// - `min_samples`: must be 1–10
pub fn validate_clustering_params(min_repetitions: u32, min_samples: u32) -> Result<(), AppError> {
    if min_repetitions < 2 || min_repetitions > 20 {
        return Err(AppError::Validation(ValidationError {
            field: "min_repetitions".to_string(),
            message: format!(
                "min_repetitions must be between 2 and 20, got {}",
                min_repetitions
            ),
        }));
    }
    if min_samples < 1 || min_samples > 10 {
        return Err(AppError::Validation(ValidationError {
            field: "min_samples".to_string(),
            message: format!("min_samples must be between 1 and 10, got {}", min_samples),
        }));
    }
    Ok(())
}

/// Runs HDBSCAN clustering on the provided embeddings.
///
/// Each embedding is a 384-dimensional f32 vector. The algorithm uses
/// Euclidean distance (appropriate for L2-normalized embeddings where
/// Euclidean distance is monotonically related to cosine distance).
///
/// Returns a vector of cluster labels:
/// - `>= 0`: cluster membership ID
/// - `-1`: noise (not assigned to any cluster)
///
/// Returns `AppError::Clustering` if no clusters are found (all points are noise).
pub fn run_hdbscan(
    embeddings: &[Vec<f32>],
    min_cluster_size: u32,
    min_samples: u32,
) -> Result<Vec<i32>, AppError> {
    if embeddings.is_empty() {
        return Err(AppError::Clustering(ClusteringError {
            message: "No embeddings provided for clustering".to_string(),
        }));
    }

    // Build hyper parameters
    let config = HdbscanHyperParams::builder()
        .min_cluster_size(min_cluster_size as usize)
        .min_samples(min_samples as usize)
        .build();

    // Run HDBSCAN
    let clusterer = Hdbscan::new(embeddings, config);
    let labels = clusterer.cluster().map_err(|e| {
        AppError::Clustering(ClusteringError {
            message: format!("HDBSCAN clustering failed: {}", e),
        })
    })?;

    // Check if all points are noise
    let has_clusters = labels.iter().any(|&label| label >= 0);
    if !has_clusters {
        return Err(AppError::Clustering(ClusteringError {
            message: "No clusters found. All windows were assigned as noise. \
                      Try lowering Min Repetitions or Min Samples to detect \
                      less dense repetition patterns."
                .to_string(),
        }));
    }

    Ok(labels)
}

/// Fixed random seed for KMeans reproducibility.
const KMEANS_SEED: u64 = 42;

/// Maximum iterations for KMeans convergence.
const KMEANS_MAX_ITER: u64 = 300;

/// Tolerance for KMeans convergence.
const KMEANS_TOLERANCE: f64 = 1e-4;

/// Stabilizes HDBSCAN cluster assignments using KMeans.
///
/// Takes the HDBSCAN output and produces stable integer cluster IDs by:
/// 1. Filtering to non-noise windows only (hdbscan_label ≥ 0)
/// 2. Counting distinct HDBSCAN clusters with ≥3 members → k
/// 3. If k == 0, returning the original labels unchanged
/// 4. Sorting non-noise windows by window_index for determinism
/// 5. Running KMeans with k clusters using a fixed random seed
/// 6. Assigning stable integer IDs (0, 1, 2, ..., k-1)
/// 7. Never merging clusters — output has exactly k distinct IDs
/// 8. Noise windows keep -1, non-noise get new stable IDs
pub fn stabilize_clusters(
    embeddings: &[Vec<f32>],
    hdbscan_labels: &[i32],
    window_indices: &[u32],
) -> Vec<i32> {
    assert_eq!(embeddings.len(), hdbscan_labels.len());
    assert_eq!(embeddings.len(), window_indices.len());

    // Count distinct HDBSCAN clusters with ≥3 members
    let mut cluster_counts: HashMap<i32, usize> = HashMap::new();
    for &label in hdbscan_labels {
        if label >= 0 {
            *cluster_counts.entry(label).or_insert(0) += 1;
        }
    }

    // Only keep clusters with ≥3 members
    let valid_clusters: Vec<i32> = cluster_counts
        .iter()
        .filter(|(_, &count)| count >= 3)
        .map(|(&label, _)| label)
        .collect();

    let k = valid_clusters.len();

    // If k == 0, return original labels unchanged
    if k == 0 {
        return hdbscan_labels.to_vec();
    }

    // Collect indices of non-noise windows that belong to valid clusters (≥3 members)
    let mut non_noise_indices: Vec<usize> = Vec::new();
    for (i, &label) in hdbscan_labels.iter().enumerate() {
        if label >= 0 && valid_clusters.contains(&label) {
            non_noise_indices.push(i);
        }
    }

    // Sort by window_index for determinism
    non_noise_indices.sort_by_key(|&i| window_indices[i]);

    if non_noise_indices.is_empty() {
        return hdbscan_labels.to_vec();
    }

    // Build the embedding matrix for non-noise windows
    let dim = embeddings[0].len();
    let n = non_noise_indices.len();
    let mut data = Array2::<f64>::zeros((n, dim));
    for (row, &idx) in non_noise_indices.iter().enumerate() {
        for (col, &val) in embeddings[idx].iter().enumerate() {
            data[[row, col]] = val as f64;
        }
    }

    // Create dataset for linfa fitting
    let dataset = DatasetBase::from(data.clone());

    // Run KMeans with fixed seed
    let rng = ChaCha8Rng::seed_from_u64(KMEANS_SEED);
    let model = KMeans::params_with_rng(k, rng)
        .max_n_iterations(KMEANS_MAX_ITER)
        .tolerance(KMEANS_TOLERANCE)
        .fit(&dataset)
        .expect("KMeans fitting should not fail on valid data");

    // Get cluster assignments using predict on the raw array
    let predictions = model.predict(data.view());
    let assignments = predictions.targets();

    // Build result: noise windows keep -1, non-noise get KMeans cluster ID
    let mut result = vec![-1i32; embeddings.len()];
    for (i, &idx) in non_noise_indices.iter().enumerate() {
        result[idx] = assignments[i] as i32;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── derive_min_cluster_size tests ───────────────────────────────────────

    #[test]
    fn test_derive_min_cluster_size_basic() {
        // phrase_length=20, stride=5 → windows_per_phrase = 4
        // min_repetitions=3 → 3 * 4 = 12
        assert_eq!(derive_min_cluster_size(3, 20, 5), 12);
    }

    #[test]
    fn test_derive_min_cluster_size_stride_equals_phrase() {
        // phrase_length=10, stride=10 → windows_per_phrase = 1
        // min_repetitions=2 → 2 * 1 = 2
        assert_eq!(derive_min_cluster_size(2, 10, 10), 2);
    }

    #[test]
    fn test_derive_min_cluster_size_stride_larger_than_phrase() {
        // phrase_length=5, stride=10 → floor(5/10) = 0, max(1, 0) = 1
        // min_repetitions=4 → 4 * 1 = 4
        assert_eq!(derive_min_cluster_size(4, 5, 10), 4);
    }

    #[test]
    fn test_derive_min_cluster_size_large_phrase() {
        // phrase_length=1500, stride=200 → windows_per_phrase = 7
        // min_repetitions=2 → 2 * 7 = 14
        assert_eq!(derive_min_cluster_size(2, 1500, 200), 14);
    }

    #[test]
    fn test_derive_min_cluster_size_min_values() {
        // phrase_length=5, stride=1 → windows_per_phrase = 5
        // min_repetitions=2 → 2 * 5 = 10
        assert_eq!(derive_min_cluster_size(2, 5, 1), 10);
    }

    #[test]
    fn test_derive_min_cluster_size_max_repetitions() {
        // phrase_length=20, stride=5 → windows_per_phrase = 4
        // min_repetitions=20 → 20 * 4 = 80
        assert_eq!(derive_min_cluster_size(20, 20, 5), 80);
    }

    // ─── validate_clustering_params tests ────────────────────────────────────

    #[test]
    fn test_validate_params_valid() {
        assert!(validate_clustering_params(2, 1).is_ok());
        assert!(validate_clustering_params(3, 3).is_ok());
        assert!(validate_clustering_params(20, 10).is_ok());
        assert!(validate_clustering_params(10, 5).is_ok());
    }

    #[test]
    fn test_validate_params_min_repetitions_too_low() {
        let result = validate_clustering_params(1, 3);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Validation(e) => {
                assert_eq!(e.field, "min_repetitions");
                assert!(e.message.contains("2 and 20"));
            }
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_validate_params_min_repetitions_too_high() {
        let result = validate_clustering_params(21, 3);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Validation(e) => {
                assert_eq!(e.field, "min_repetitions");
            }
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_validate_params_min_samples_too_low() {
        let result = validate_clustering_params(3, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Validation(e) => {
                assert_eq!(e.field, "min_samples");
                assert!(e.message.contains("1 and 10"));
            }
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_validate_params_min_samples_too_high() {
        let result = validate_clustering_params(3, 11);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Validation(e) => {
                assert_eq!(e.field, "min_samples");
            }
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_validate_params_both_invalid() {
        // First invalid param encountered wins (min_repetitions checked first)
        let result = validate_clustering_params(0, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Validation(e) => {
                assert_eq!(e.field, "min_repetitions");
            }
            _ => panic!("Expected Validation error"),
        }
    }

    // ─── run_hdbscan tests ───────────────────────────────────────────────────

    #[test]
    fn test_run_hdbscan_empty_embeddings() {
        let result = run_hdbscan(&[], 5, 3);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Clustering(e) => {
                assert!(e.message.contains("No embeddings"));
            }
            _ => panic!("Expected Clustering error"),
        }
    }

    #[test]
    fn test_run_hdbscan_finds_clusters_in_synthetic_data() {
        // Create two tight clusters of 384-dim vectors
        let dim = 384;
        let mut embeddings: Vec<Vec<f32>> = Vec::new();

        // Cluster A: 10 points near [1, 0, 0, ..., 0] (normalized)
        for i in 0..10 {
            let mut v = vec![0.0f32; dim];
            v[0] = 1.0;
            // Add small perturbation
            v[1] = 0.01 * (i as f32);
            // Normalize
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v.iter_mut() {
                *x /= norm;
            }
            embeddings.push(v);
        }

        // Cluster B: 10 points near [0, 1, 0, ..., 0] (normalized)
        for i in 0..10 {
            let mut v = vec![0.0f32; dim];
            v[1] = 1.0;
            v[2] = 0.01 * (i as f32);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v.iter_mut() {
                *x /= norm;
            }
            embeddings.push(v);
        }

        let result = run_hdbscan(&embeddings, 3, 2);
        assert!(result.is_ok(), "Expected clusters to be found");

        let labels = result.unwrap();
        assert_eq!(labels.len(), 20);

        // At least some points should be assigned to clusters
        let clustered_count = labels.iter().filter(|&&l| l >= 0).count();
        assert!(
            clustered_count > 0,
            "Expected some points to be clustered, got all noise"
        );
    }

    #[test]
    fn test_run_hdbscan_all_noise_returns_error() {
        // Only 3 points with high min_cluster_size = 10 should result in all noise
        let dim = 384;
        let mut embeddings: Vec<Vec<f32>> = Vec::new();

        for i in 0..3 {
            let mut v = vec![0.0f32; dim];
            v[i % dim] = 1.0;
            embeddings.push(v);
        }

        let result = run_hdbscan(&embeddings, 10, 3);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Clustering(e) => {
                assert!(e.message.contains("No clusters found"));
                assert!(e.message.contains("Min Repetitions"));
            }
            _ => panic!("Expected Clustering error"),
        }
    }

    #[test]
    fn test_run_hdbscan_labels_are_valid() {
        // All labels should be >= -1
        let dim = 384;
        let mut embeddings: Vec<Vec<f32>> = Vec::new();

        // Create a tight cluster
        for i in 0..20 {
            let mut v = vec![0.0f32; dim];
            v[0] = 1.0;
            v[1] = 0.001 * (i as f32);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v.iter_mut() {
                *x /= norm;
            }
            embeddings.push(v);
        }

        let result = run_hdbscan(&embeddings, 3, 2);
        if let Ok(labels) = result {
            for &label in &labels {
                assert!(label >= -1, "Label should be >= -1, got {}", label);
            }
        }
    }

    // ─── stabilize_clusters tests ────────────────────────────────────────────

    #[test]
    fn test_stabilize_clusters_zero_non_noise_returns_original() {
        // All labels are -1 (noise) → should return original labels unchanged
        let embeddings = vec![
            vec![1.0f32; 384],
            vec![0.5f32; 384],
            vec![0.3f32; 384],
        ];
        let hdbscan_labels = vec![-1, -1, -1];
        let window_indices = vec![0, 1, 2];

        let result = stabilize_clusters(&embeddings, &hdbscan_labels, &window_indices);
        assert_eq!(result, vec![-1, -1, -1]);
    }

    #[test]
    fn test_stabilize_clusters_small_clusters_skipped() {
        // Clusters with < 3 members should be skipped
        // Cluster 0 has 2 members, cluster 1 has 2 members → k=0 → return original
        let dim = 384;
        let embeddings = vec![
            vec![1.0f32; dim],
            vec![1.0f32; dim],
            vec![0.0f32; dim],
            vec![0.0f32; dim],
        ];
        let hdbscan_labels = vec![0, 0, 1, 1]; // each cluster has only 2 members
        let window_indices = vec![0, 1, 2, 3];

        let result = stabilize_clusters(&embeddings, &hdbscan_labels, &window_indices);
        // k=0 since no cluster has ≥3 members, return original
        assert_eq!(result, vec![0, 0, 1, 1]);
    }

    #[test]
    fn test_stabilize_clusters_determinism() {
        // Same input should produce same output on repeated runs
        let dim = 384;
        let mut embeddings: Vec<Vec<f32>> = Vec::new();

        // Cluster A: 5 points near [1, 0, 0, ...]
        for i in 0..5 {
            let mut v = vec![0.0f32; dim];
            v[0] = 1.0;
            v[1] = 0.01 * (i as f32);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v.iter_mut() {
                *x /= norm;
            }
            embeddings.push(v);
        }

        // Cluster B: 5 points near [0, 1, 0, ...]
        for i in 0..5 {
            let mut v = vec![0.0f32; dim];
            v[1] = 1.0;
            v[2] = 0.01 * (i as f32);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v.iter_mut() {
                *x /= norm;
            }
            embeddings.push(v);
        }

        let hdbscan_labels = vec![0, 0, 0, 0, 0, 1, 1, 1, 1, 1];
        let window_indices: Vec<u32> = (0..10).collect();

        let result1 = stabilize_clusters(&embeddings, &hdbscan_labels, &window_indices);
        let result2 = stabilize_clusters(&embeddings, &hdbscan_labels, &window_indices);

        assert_eq!(result1, result2, "KMeans stabilization must be deterministic");
    }

    #[test]
    fn test_stabilize_clusters_correct_number_of_ids() {
        // Two valid clusters (≥3 members each) → output should have exactly 2 distinct IDs
        let dim = 384;
        let mut embeddings: Vec<Vec<f32>> = Vec::new();

        // Cluster A: 4 points near [1, 0, 0, ...]
        for i in 0..4 {
            let mut v = vec![0.0f32; dim];
            v[0] = 1.0;
            v[1] = 0.01 * (i as f32);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v.iter_mut() {
                *x /= norm;
            }
            embeddings.push(v);
        }

        // Cluster B: 4 points near [0, 1, 0, ...]
        for i in 0..4 {
            let mut v = vec![0.0f32; dim];
            v[1] = 1.0;
            v[2] = 0.01 * (i as f32);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v.iter_mut() {
                *x /= norm;
            }
            embeddings.push(v);
        }

        // Add 2 noise points
        embeddings.push(vec![0.5f32; dim]);
        embeddings.push(vec![0.3f32; dim]);

        let hdbscan_labels = vec![0, 0, 0, 0, 1, 1, 1, 1, -1, -1];
        let window_indices: Vec<u32> = (0..10).collect();

        let result = stabilize_clusters(&embeddings, &hdbscan_labels, &window_indices);

        // Noise windows should remain -1
        assert_eq!(result[8], -1);
        assert_eq!(result[9], -1);

        // Non-noise windows should have valid cluster IDs
        let non_noise_ids: Vec<i32> = result.iter()
            .filter(|&&id| id >= 0)
            .copied()
            .collect();

        // Should have exactly 2 distinct cluster IDs (k=2)
        let mut distinct_ids: Vec<i32> = non_noise_ids.clone();
        distinct_ids.sort();
        distinct_ids.dedup();
        assert_eq!(distinct_ids.len(), 2, "Expected exactly 2 distinct cluster IDs");

        // All non-noise IDs should be in range [0, k-1]
        for &id in &non_noise_ids {
            assert!(id >= 0 && id < 2, "Cluster ID {} out of range [0, 1]", id);
        }
    }

    #[test]
    fn test_stabilize_clusters_noise_windows_remain_negative_one() {
        let dim = 384;
        let mut embeddings: Vec<Vec<f32>> = Vec::new();

        // 3 clustered points
        for i in 0..3 {
            let mut v = vec![0.0f32; dim];
            v[0] = 1.0;
            v[1] = 0.01 * (i as f32);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in v.iter_mut() {
                *x /= norm;
            }
            embeddings.push(v);
        }

        // 2 noise points
        embeddings.push(vec![0.7f32; dim]);
        embeddings.push(vec![0.2f32; dim]);

        let hdbscan_labels = vec![0, 0, 0, -1, -1];
        let window_indices = vec![0, 1, 2, 3, 4];

        let result = stabilize_clusters(&embeddings, &hdbscan_labels, &window_indices);

        // Noise windows must remain -1
        assert_eq!(result[3], -1);
        assert_eq!(result[4], -1);

        // Clustered windows should have non-negative IDs
        assert!(result[0] >= 0);
        assert!(result[1] >= 0);
        assert!(result[2] >= 0);
    }
}
