use std::collections::HashMap;

use crate::spans::merge_overlapping_spans;
use crate::types::{ClusterInfo, ClusterRegistry};

/// Input data for centroid computation — one entry per window.
#[derive(Debug, Clone)]
pub struct WindowData {
    pub window_id: String,
    pub window_index: u32,
    pub page: u32,
    pub cluster_id: i32,
    pub embedding: Vec<f32>,
    pub text: String,
    /// Inclusive document character start.
    pub doc_char_start: u32,
    /// Exclusive document character end.
    pub doc_char_end: u32,
}

/// Group overlapping document spans into distinct repetition instances.
///
/// Sliding windows from stride < phrase length often overlap; this merges
/// any spans that share document text into one instance.
pub fn count_overlapping_instances(spans: &[(u32, u32)]) -> u32 {
    merge_overlapping_spans(spans).len() as u32
}

/// Compute cosine similarity between two vectors.
/// Returns 0.0 if either vector has zero magnitude.
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

/// Compute the element-wise mean of a set of embeddings.
fn compute_centroid(embeddings: &[&Vec<f32>]) -> Vec<f32> {
    if embeddings.is_empty() {
        return Vec::new();
    }
    let dim = embeddings[0].len();
    let n = embeddings.len() as f32;
    let mut centroid = vec![0.0f32; dim];
    for emb in embeddings {
        for (i, val) in emb.iter().enumerate() {
            centroid[i] += val;
        }
    }
    for val in centroid.iter_mut() {
        *val /= n;
    }
    centroid
}

/// Check if a vector has zero magnitude.
fn is_zero_magnitude(v: &[f32]) -> bool {
    let magnitude_sq: f32 = v.iter().map(|x| x * x).sum();
    magnitude_sq == 0.0
}

/// Build the cluster registry from window data.
///
/// Groups windows by cluster_id (skipping noise: cluster_id == -1), computes
/// centroids, identifies most central windows, and builds page indices.
///
/// Clusters whose centroid has zero magnitude are excluded from the registry
/// (per Requirement 9.5).
pub fn build_cluster_registry(windows: &[WindowData]) -> ClusterRegistry {
    // Group windows by cluster_id, skipping noise (cluster_id == -1)
    let mut groups: HashMap<i32, Vec<&WindowData>> = HashMap::new();
    for w in windows {
        if w.cluster_id == -1 {
            continue;
        }
        groups.entry(w.cluster_id).or_default().push(w);
    }

    let mut clusters: HashMap<i32, ClusterInfo> = HashMap::new();

    for (cluster_id, members) in &groups {
        // Compute centroid = element-wise mean of all member embeddings
        let embeddings: Vec<&Vec<f32>> = members.iter().map(|w| &w.embedding).collect();
        let centroid = compute_centroid(&embeddings);

        // Handle zero-magnitude centroid: exclude cluster from registry (Req 9.5)
        if is_zero_magnitude(&centroid) {
            continue;
        }

        // Compute cosine similarity of each member to the centroid
        // Find most_central_window_id: highest cosine sim, ties broken by lowest window_index
        let mut best_window: Option<&WindowData> = None;
        let mut best_sim: f32 = f32::NEG_INFINITY;

        for member in members {
            let sim = cosine_similarity(&member.embedding, &centroid);
            if sim > best_sim || (sim == best_sim && best_window.map_or(true, |bw| member.window_index < bw.window_index)) {
                best_sim = sim;
                best_window = Some(member);
            }
        }

        let best = best_window.expect("cluster must have at least one member");

        // Build pages list: sorted unique page numbers
        let mut pages: Vec<u32> = members.iter().map(|w| w.page).collect();
        pages.sort_unstable();
        pages.dedup();

        let spans: Vec<(u32, u32)> = members
            .iter()
            .map(|w| (w.doc_char_start, w.doc_char_end))
            .collect();
        let instance_count = count_overlapping_instances(&spans);

        // Compute hue using golden-ratio formula
        let hue = ((*cluster_id as f64 * 0.6180339887) % 1.0) as f32;

        clusters.insert(
            *cluster_id,
            ClusterInfo {
                cluster_id: *cluster_id,
                hue,
                centroid,
                most_central_window_id: best.window_id.clone(),
                most_central_window_text: best.text.clone(),
                member_count: members.len() as u32,
                instance_count,
                pages,
            },
        );
    }

    ClusterRegistry { clusters }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a WindowData with the given parameters.
    fn make_window(
        window_id: &str,
        window_index: u32,
        page: u32,
        cluster_id: i32,
        embedding: Vec<f32>,
        text: &str,
    ) -> WindowData {
        let doc_char_start = window_index * 100;
        WindowData {
            window_id: window_id.to_string(),
            window_index,
            page,
            cluster_id,
            embedding,
            text: text.to_string(),
            doc_char_start,
            doc_char_end: doc_char_start + 50,
        }
    }

    #[test]
    fn test_centroid_is_element_wise_mean() {
        let windows = vec![
            make_window("w1", 0, 1, 1, vec![1.0, 2.0, 3.0], "hello"),
            make_window("w2", 1, 1, 1, vec![3.0, 4.0, 5.0], "world"),
            make_window("w3", 2, 2, 1, vec![2.0, 6.0, 1.0], "foo"),
        ];

        let registry = build_cluster_registry(&windows);
        let info = registry.clusters.get(&1).unwrap();

        // Mean of [1,2,3], [3,4,5], [2,6,1] = [2.0, 4.0, 3.0]
        let expected = vec![2.0, 4.0, 3.0];
        assert_eq!(info.centroid.len(), expected.len());
        for (a, b) in info.centroid.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-6, "centroid mismatch: {} vs {}", a, b);
        }
    }

    #[test]
    fn test_most_central_window_highest_cosine_sim() {
        // Centroid will be mean of embeddings. We construct windows where one
        // is clearly closest to the centroid.
        // Embeddings: [1,0,0], [0,1,0], [1,1,0]
        // Centroid = [2/3, 2/3, 0]
        // cos([1,0,0], [2/3,2/3,0]) = 2/3 / (1 * sqrt(8/9)) = (2/3) / (2*sqrt(2)/3) = 1/sqrt(2) ≈ 0.707
        // cos([0,1,0], [2/3,2/3,0]) = 2/3 / (1 * sqrt(8/9)) = same ≈ 0.707
        // cos([1,1,0], [2/3,2/3,0]) = (2/3+2/3) / (sqrt(2) * sqrt(8/9)) = (4/3) / (sqrt(2)*2sqrt(2)/3) = (4/3)/(4/3) = 1.0
        let windows = vec![
            make_window("w1", 0, 1, 1, vec![1.0, 0.0, 0.0], "first"),
            make_window("w2", 1, 1, 1, vec![0.0, 1.0, 0.0], "second"),
            make_window("w3", 2, 2, 1, vec![1.0, 1.0, 0.0], "third"),
        ];

        let registry = build_cluster_registry(&windows);
        let info = registry.clusters.get(&1).unwrap();

        // w3 has the highest cosine similarity to the centroid
        assert_eq!(info.most_central_window_id, "w3");
        assert_eq!(info.most_central_window_text, "third");
    }

    #[test]
    fn test_tie_breaking_by_lowest_window_index() {
        // Two windows with identical embeddings → same cosine sim to centroid.
        // Tie should be broken by lowest window_index.
        let windows = vec![
            make_window("w1", 5, 1, 2, vec![1.0, 1.0, 1.0], "alpha"),
            make_window("w2", 3, 1, 2, vec![1.0, 1.0, 1.0], "beta"),
            make_window("w3", 7, 2, 2, vec![1.0, 1.0, 1.0], "gamma"),
        ];

        let registry = build_cluster_registry(&windows);
        let info = registry.clusters.get(&2).unwrap();

        // All have the same embedding, so same cosine sim to centroid.
        // Tie broken by lowest window_index → w2 (index 3)
        assert_eq!(info.most_central_window_id, "w2");
    }

    #[test]
    fn test_pages_list_sorted_and_unique() {
        let windows = vec![
            make_window("w1", 0, 3, 1, vec![1.0, 0.0], "a"),
            make_window("w2", 1, 1, 1, vec![0.0, 1.0], "b"),
            make_window("w3", 2, 3, 1, vec![1.0, 1.0], "c"),
            make_window("w4", 3, 2, 1, vec![0.5, 0.5], "d"),
            make_window("w5", 4, 1, 1, vec![0.3, 0.7], "e"),
        ];

        let registry = build_cluster_registry(&windows);
        let info = registry.clusters.get(&1).unwrap();

        // Pages should be sorted and unique: [1, 2, 3]
        assert_eq!(info.pages, vec![1, 2, 3]);
    }

    #[test]
    fn test_zero_magnitude_centroid_excluded() {
        // All embeddings are zero vectors → centroid is zero → excluded
        let windows = vec![
            make_window("w1", 0, 1, 1, vec![0.0, 0.0, 0.0], "a"),
            make_window("w2", 1, 1, 1, vec![0.0, 0.0, 0.0], "b"),
            make_window("w3", 2, 2, 1, vec![0.0, 0.0, 0.0], "c"),
        ];

        let registry = build_cluster_registry(&windows);

        // Cluster 1 should be excluded due to zero-magnitude centroid
        assert!(!registry.clusters.contains_key(&1));
    }

    #[test]
    fn test_hue_formula_correct() {
        let windows = vec![
            make_window("w1", 0, 1, 0, vec![1.0, 0.0], "a"),
            make_window("w2", 1, 1, 0, vec![0.0, 1.0], "b"),
            make_window("w3", 2, 1, 0, vec![1.0, 1.0], "c"),
            make_window("w4", 3, 1, 5, vec![1.0, 0.0], "d"),
            make_window("w5", 4, 1, 5, vec![0.0, 1.0], "e"),
            make_window("w6", 5, 1, 5, vec![1.0, 1.0], "f"),
        ];

        let registry = build_cluster_registry(&windows);

        // Hue for cluster 0: (0 × 0.6180339887) mod 1.0 = 0.0
        let info0 = registry.clusters.get(&0).unwrap();
        let expected_hue_0 = (0.0_f64 * 0.6180339887) % 1.0;
        assert!((info0.hue - expected_hue_0 as f32).abs() < 1e-6);

        // Hue for cluster 5: (5 × 0.6180339887) mod 1.0 = 3.0901699435 mod 1.0 = 0.0901699435
        let info5 = registry.clusters.get(&5).unwrap();
        let expected_hue_5 = (5.0_f64 * 0.6180339887) % 1.0;
        assert!(
            (info5.hue - expected_hue_5 as f32).abs() < 1e-6,
            "hue mismatch for cluster 5: {} vs {}",
            info5.hue,
            expected_hue_5
        );
    }

    #[test]
    fn test_noise_windows_skipped() {
        let windows = vec![
            make_window("w1", 0, 1, -1, vec![1.0, 0.0], "noise1"),
            make_window("w2", 1, 1, -1, vec![0.0, 1.0], "noise2"),
            make_window("w3", 2, 1, 1, vec![1.0, 1.0], "signal1"),
            make_window("w4", 3, 2, 1, vec![0.5, 0.5], "signal2"),
            make_window("w5", 4, 2, 1, vec![0.7, 0.3], "signal3"),
        ];

        let registry = build_cluster_registry(&windows);

        // Noise cluster (-1) should not appear
        assert!(!registry.clusters.contains_key(&-1));
        // Cluster 1 should be present
        assert!(registry.clusters.contains_key(&1));
        assert_eq!(registry.clusters.get(&1).unwrap().member_count, 3);
    }

    #[test]
    fn test_member_count_correct() {
        let windows = vec![
            make_window("w1", 0, 1, 1, vec![1.0, 0.0], "a"),
            make_window("w2", 1, 1, 1, vec![0.0, 1.0], "b"),
            make_window("w3", 2, 2, 1, vec![1.0, 1.0], "c"),
            make_window("w4", 3, 2, 2, vec![0.5, 0.5], "d"),
            make_window("w5", 4, 3, 2, vec![0.3, 0.7], "e"),
        ];

        let registry = build_cluster_registry(&windows);

        assert_eq!(registry.clusters.get(&1).unwrap().member_count, 3);
        assert_eq!(registry.clusters.get(&2).unwrap().member_count, 2);
    }

    #[test]
    fn test_count_overlapping_instances_merges_runs() {
        let spans = vec![(0, 100), (50, 150), (80, 180), (300, 400)];
        assert_eq!(count_overlapping_instances(&spans), 2);
    }

    #[test]
    fn test_instance_count_merges_overlapping_windows() {
        let windows = vec![
            WindowData {
                window_id: "w1".to_string(),
                window_index: 0,
                page: 1,
                cluster_id: 1,
                embedding: vec![1.0, 0.0],
                text: "a".to_string(),
                doc_char_start: 0,
                doc_char_end: 100,
            },
            WindowData {
                window_id: "w2".to_string(),
                window_index: 1,
                page: 1,
                cluster_id: 1,
                embedding: vec![0.9, 0.1],
                text: "b".to_string(),
                doc_char_start: 40,
                doc_char_end: 140,
            },
            WindowData {
                window_id: "w3".to_string(),
                window_index: 2,
                page: 1,
                cluster_id: 1,
                embedding: vec![0.8, 0.2],
                text: "c".to_string(),
                doc_char_start: 500,
                doc_char_end: 600,
            },
        ];

        let registry = build_cluster_registry(&windows);
        let info = registry.clusters.get(&1).unwrap();
        assert_eq!(info.member_count, 3);
        assert_eq!(info.instance_count, 2);
    }

    #[test]
    fn test_empty_input_produces_empty_registry() {
        let windows: Vec<WindowData> = vec![];
        let registry = build_cluster_registry(&windows);
        assert!(registry.clusters.is_empty());
    }

    #[test]
    fn test_multiple_clusters() {
        let windows = vec![
            make_window("w1", 0, 1, 1, vec![1.0, 0.0, 0.0], "a"),
            make_window("w2", 1, 1, 1, vec![0.9, 0.1, 0.0], "b"),
            make_window("w3", 2, 2, 1, vec![0.8, 0.2, 0.0], "c"),
            make_window("w4", 3, 3, 2, vec![0.0, 1.0, 0.0], "d"),
            make_window("w5", 4, 3, 2, vec![0.0, 0.9, 0.1], "e"),
            make_window("w6", 5, 4, 2, vec![0.0, 0.8, 0.2], "f"),
        ];

        let registry = build_cluster_registry(&windows);

        assert_eq!(registry.clusters.len(), 2);
        assert!(registry.clusters.contains_key(&1));
        assert!(registry.clusters.contains_key(&2));

        // Cluster 1 pages: [1, 2]
        assert_eq!(registry.clusters.get(&1).unwrap().pages, vec![1, 2]);
        // Cluster 2 pages: [3, 4]
        assert_eq!(registry.clusters.get(&2).unwrap().pages, vec![3, 4]);
    }
}
