//! Shared analysis parameters and clustering stages for file and text input.

use std::collections::HashMap;

use crate::centroid::{build_cluster_registry, WindowData};
use crate::clustering::{
    cluster_by_kmeans_similarity, derive_min_cluster_size, merge_subsumed_clusters, run_hdbscan,
    stabilize_clusters, validate_clustering_params,
};
use crate::subcell::{build_page_sub_grids, WindowSubCellData};
use crate::types::{
    AppError, ClusterRegistry, ImportError, Page, ValidationError, Window,
};

/// Analysis tuning parameters shared by the visual app and Romance Factory integration.
#[derive(Debug, Clone)]
pub struct AnalysisParams {
    pub window_size: u32,
    pub stride: u32,
    pub tokens_per_page: Option<u32>,
    pub chapter_break_regex: Option<String>,
    pub min_repetitions: u32,
    pub min_samples: u32,
    pub enable_hdbscan: bool,
    pub link_subphrases: bool,
}

/// Output of clustering + sub-cell mapping, ready for visualization payload assembly.
#[derive(Debug, Clone)]
pub struct ClusteringArtifacts {
    pub window_data: Vec<WindowData>,
    pub cluster_registry: ClusterRegistry,
    pub page_sub_grids: Vec<crate::types::PageSubGrid>,
}

/// Paginate plain text using the same settings as file import.
pub fn paginate_text(
    text: &str,
    params: &AnalysisParams,
) -> Result<Vec<Page>, AppError> {
    if text.is_empty() || text.chars().all(|c| c.is_whitespace()) {
        return Err(AppError::Import(ImportError {
            message: "Text contains no analyzable content".to_string(),
            path: None,
        }));
    }

    if let Some(ref regex_str) = params.chapter_break_regex {
        if !regex_str.is_empty() {
            let tpp = params.tokens_per_page.unwrap_or(400);
            return crate::importer::paginate_by_chapter_break(text, regex_str, tpp);
        }
    }

    let tpp = params.tokens_per_page.unwrap_or(400);
    crate::importer::paginate_by_token_count(text, tpp)
}

/// Validate analysis parameters. Set `require_path` when analyzing from a file path.
pub fn validate_analysis_params(
    params: &AnalysisParams,
    path: Option<&str>,
) -> Result<(), AppError> {
    if let Some(path) = path {
        if path.is_empty() {
            return Err(AppError::Validation(ValidationError {
                field: "path".to_string(),
                message: "Document path cannot be empty".to_string(),
            }));
        }
    }

    if params.window_size < 5 || params.window_size > 1500 {
        return Err(AppError::Validation(ValidationError {
            field: "window_size".to_string(),
            message: format!(
                "window_size must be between 5 and 1500, got {}",
                params.window_size
            ),
        }));
    }

    if params.stride < 1 || params.stride > 200 {
        return Err(AppError::Validation(ValidationError {
            field: "stride".to_string(),
            message: format!("stride must be between 1 and 200, got {}", params.stride),
        }));
    }

    if let Some(tpp) = params.tokens_per_page {
        if tpp < 200 || tpp > 2000 {
            return Err(AppError::Validation(ValidationError {
                field: "tokens_per_page".to_string(),
                message: format!("tokens_per_page must be between 200 and 2000, got {}", tpp),
            }));
        }
    }

    if let Some(ref regex_str) = params.chapter_break_regex {
        if !regex_str.is_empty() {
            regex::Regex::new(regex_str).map_err(|e| {
                AppError::Validation(ValidationError {
                    field: "chapter_break_regex".to_string(),
                    message: format!("Invalid regex pattern: {}", e),
                })
            })?;
        }
    }

    validate_clustering_params(params.min_repetitions, params.min_samples)
}

/// Run HDBSCAN/KMeans clustering and optional subphrase merging.
pub fn run_clustering(
    params: &AnalysisParams,
    all_embeddings: &[Vec<f32>],
    windows: &[Window],
) -> Result<(Vec<i32>, Vec<i32>), AppError> {
    let window_indices: Vec<u32> = windows.iter().map(|w| w.window_index).collect();
    let min_cluster_size = derive_min_cluster_size(
        params.min_repetitions,
        params.window_size,
        params.stride,
    );

    let (hdbscan_labels, mut stable_labels) = if params.enable_hdbscan {
        let labels = run_hdbscan(all_embeddings, min_cluster_size, params.min_samples)?;
        let stable = stabilize_clusters(all_embeddings, &labels, &window_indices);
        (labels, stable)
    } else {
        let stable =
            cluster_by_kmeans_similarity(all_embeddings, &window_indices, min_cluster_size)?;
        let labels = vec![-1i32; all_embeddings.len()];
        (labels, stable)
    };

    if params.link_subphrases {
        let texts: Vec<String> = windows.iter().map(|w| w.text.clone()).collect();
        let pages: Vec<u32> = windows.iter().map(|w| w.page).collect();
        merge_subsumed_clusters(&mut stable_labels, all_embeddings, &texts, &pages);
    }

    Ok((hdbscan_labels, stable_labels))
}

/// Build cluster registry and page sub-grids from clustered windows.
pub fn build_clustering_artifacts(
    pages: &[Page],
    windows: &[Window],
    embeddings: &[Vec<f32>],
    stable_labels: &[i32],
) -> ClusteringArtifacts {
    let cluster_registry = {
        let window_data: Vec<WindowData> = windows
            .iter()
            .enumerate()
            .map(|(i, w)| WindowData {
                window_id: w.window_id.clone(),
                window_index: w.window_index,
                page: w.page,
                cluster_id: stable_labels[i],
                embedding: embeddings[i].clone(),
                text: w.text.clone(),
                doc_char_start: w.doc_char_start,
                doc_char_end: w.doc_char_start + w.char_end.saturating_sub(w.char_start),
            })
            .collect();
        build_cluster_registry(&window_data)
    };

    let mut sim_to_centroids: Vec<f32> = vec![0.0; windows.len()];
    for (i, label) in stable_labels.iter().enumerate() {
        if *label >= 0 {
            if let Some(info) = cluster_registry.clusters.get(label) {
                sim_to_centroids[i] = cosine_similarity(&embeddings[i], &info.centroid);
            }
        }
    }

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

    let window_data: Vec<WindowData> = windows
        .iter()
        .enumerate()
        .map(|(i, w)| WindowData {
            window_id: w.window_id.clone(),
            window_index: w.window_index,
            page: w.page,
            cluster_id: stable_labels[i],
            embedding: embeddings[i].clone(),
            text: w.text.clone(),
            doc_char_start: w.doc_char_start,
            doc_char_end: w.doc_char_start + w.char_end.saturating_sub(w.char_start),
        })
        .collect();

    ClusteringArtifacts {
        window_data,
        cluster_registry,
        page_sub_grids,
    }
}

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
