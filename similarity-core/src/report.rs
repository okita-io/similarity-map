//! Editorial repetition report for downstream tools (e.g. Romance Factory).

use serde::{Deserialize, Serialize};

use crate::centroid::{build_cluster_registry, WindowData};
use crate::importer::{import_document, ImportDocumentParams};
use crate::job_data::parse_window_data_from_batches;
use crate::spans::{expand_to_sentence_boundaries, merge_overlapping_spans};
use crate::storage::{Storage, StorageError};
use crate::types::{AppError, ClusterRegistry, Page, SessionError};

/// A document span suitable for surgical text editing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditSpan {
    pub cluster_id: i32,
    /// 1-based instance index within the cluster (document order).
    pub instance_id: u32,
    pub doc_char_start: u32,
    pub doc_char_end: u32,
    pub text: String,
    pub similarity_to_centroid: f32,
    pub member_window_count: u32,
}

/// Summary of one repetition cluster with canonical + duplicate instances.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClusterSummary {
    pub cluster_id: i32,
    pub representative_text: String,
    pub instance_count: u32,
    pub total_word_estimate: u32,
    /// First occurrence in document order — keep for the reader.
    pub canonical: EditSpan,
    /// All later occurrences — editorial targets (rewrite or remove).
    pub duplicates: Vec<EditSpan>,
    /// All merged instances in document order (canonical + duplicates).
    pub spans: Vec<EditSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisStats {
    pub cluster_count: u32,
    pub total_duplicate_instances: u32,
    pub total_duplicate_words_estimate: u32,
}

/// Full editorial report for a completed analysis job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepetitionReport {
    pub job_id: String,
    pub clusters: Vec<ClusterSummary>,
    pub stats: AnalysisStats,
}

/// Reconstruct full document text from paginated import output.
pub fn pages_to_document_text(pages: &[Page]) -> String {
    pages.iter().map(|p| p.text.as_str()).collect()
}

/// Build an editorial repetition report from clustered window data.
///
/// When `expand_to_sentences` is true, each merged instance span is expanded
/// to sentence (or paragraph) boundaries before extracting text.
pub fn build_repetition_report(
    job_id: &str,
    document_text: &str,
    windows: &[WindowData],
    expand_to_sentences: bool,
) -> RepetitionReport {
    let registry = build_cluster_registry(windows);
    build_repetition_report_from_registry(
        job_id,
        document_text,
        windows,
        &registry,
        expand_to_sentences,
    )
}

pub fn build_repetition_report_from_registry(
    job_id: &str,
    document_text: &str,
    windows: &[WindowData],
    registry: &ClusterRegistry,
    expand_to_sentences: bool,
) -> RepetitionReport {
    let mut clusters: Vec<ClusterSummary> = registry
        .clusters
        .values()
        .filter_map(|info| {
            let cluster_windows: Vec<&WindowData> = windows
                .iter()
                .filter(|w| w.cluster_id == info.cluster_id)
                .collect();

            if cluster_windows.is_empty() {
                return None;
            }

            let raw_spans: Vec<(u32, u32)> = cluster_windows
                .iter()
                .map(|w| (w.doc_char_start, w.doc_char_end))
                .collect();
            let merged = merge_overlapping_spans(&raw_spans);

            let mut edit_spans: Vec<EditSpan> = merged
                .into_iter()
                .enumerate()
                .map(|(idx, span)| {
                    let (start, end) = if expand_to_sentences {
                        expand_to_sentence_boundaries(
                            document_text,
                            span.doc_char_start,
                            span.doc_char_end,
                        )
                    } else {
                        (span.doc_char_start, span.doc_char_end)
                    };

                    let text = slice_document_text(document_text, start, end);
                    let similarity = best_similarity_in_span(&cluster_windows, start, end);

                    EditSpan {
                        cluster_id: info.cluster_id,
                        instance_id: (idx + 1) as u32,
                        doc_char_start: start,
                        doc_char_end: end,
                        text,
                        similarity_to_centroid: similarity,
                        member_window_count: span.member_window_count,
                    }
                })
                .collect();

            edit_spans.sort_by_key(|s| s.doc_char_start);

            for (idx, span) in edit_spans.iter_mut().enumerate() {
                span.instance_id = (idx + 1) as u32;
            }

            if edit_spans.is_empty() {
                return None;
            }

            let canonical = edit_spans[0].clone();
            let duplicates = edit_spans.iter().skip(1).cloned().collect::<Vec<_>>();
            let total_word_estimate = edit_spans
                .iter()
                .map(|s| s.text.split_whitespace().count() as u32)
                .sum();

            Some(ClusterSummary {
                cluster_id: info.cluster_id,
                representative_text: info.most_central_window_text.clone(),
                instance_count: edit_spans.len() as u32,
                total_word_estimate,
                canonical,
                duplicates,
                spans: edit_spans,
            })
        })
        .collect();

    clusters.sort_by_key(|c| c.canonical.doc_char_start);

    let total_duplicate_instances: u32 = clusters.iter().map(|c| c.duplicates.len() as u32).sum();
    let total_duplicate_words_estimate: u32 = clusters
        .iter()
        .flat_map(|c| c.duplicates.iter())
        .map(|s| s.text.split_whitespace().count() as u32)
        .sum();

    RepetitionReport {
        job_id: job_id.to_string(),
        clusters,
        stats: AnalysisStats {
            cluster_count: registry.clusters.len() as u32,
            total_duplicate_instances,
            total_duplicate_words_estimate,
        },
    }
}

fn map_storage_error(err: StorageError) -> AppError {
    AppError::Storage(crate::types::StorageError {
        message: err.to_string(),
    })
}

/// Load windows from storage and build an editorial repetition report.
///
/// Re-imports the source document using the job's saved pagination settings so
/// sentence-boundary expansion uses the full manuscript text.
pub async fn load_repetition_report_from_storage(
    store: &Storage,
    job_id: &str,
    expand_to_sentences: bool,
) -> Result<RepetitionReport, AppError> {
    let job = store
        .get_job_by_id(job_id)
        .await
        .map_err(map_storage_error)?
        .ok_or_else(|| {
            AppError::Session(SessionError {
                message: format!("Job not found: {}", job_id),
            })
        })?;

    let pages = import_document(
        std::path::Path::new(&job.document_path),
        &ImportDocumentParams {
            path: job.document_path.clone(),
            tokens_per_page: job.tokens_per_page,
            chapter_break_regex: job.chapter_break_re.clone(),
        },
    )?;

    let document_text = pages_to_document_text(&pages);
    let window_batches = store
        .get_windows_for_job(job_id)
        .await
        .map_err(map_storage_error)?;
    let windows = parse_window_data_from_batches(&window_batches);

    Ok(build_repetition_report(
        job_id,
        &document_text,
        &windows,
        expand_to_sentences,
    ))
}

fn slice_document_text(document_text: &str, start: u32, end: u32) -> String {
    let len = document_text.len();
    let start = (start as usize).min(len);
    let end = (end as usize).min(len).max(start);
    document_text[start..end].to_string()
}

fn best_similarity_in_span(cluster_windows: &[&WindowData], start: u32, end: u32) -> f32 {
    cluster_windows
        .iter()
        .filter(|w| w.doc_char_start < end && w.doc_char_end > start)
        .map(|_| 1.0_f32)
        .fold(0.0_f32, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::centroid::WindowData;

    fn make_window(
        id: &str,
        idx: u32,
        cluster_id: i32,
        start: u32,
        end: u32,
        text: &str,
    ) -> WindowData {
        WindowData {
            window_id: id.to_string(),
            window_index: idx,
            page: 1,
            cluster_id,
            embedding: vec![1.0, 0.0],
            text: text.to_string(),
            doc_char_start: start,
            doc_char_end: end,
        }
    }

    #[test]
    fn report_splits_canonical_and_duplicates() {
        let doc = "Alpha block here. Beta block here. Gamma block here.";
        let alpha_end = "Alpha block here.".len() as u32;
        let beta_start = alpha_end + 1;
        let beta_end = beta_start + "Beta block here.".len() as u32;
        let gamma_start = beta_end + 1;
        let gamma_end = gamma_start + "Gamma block here.".len() as u32;

        let windows = vec![
            make_window("w1", 0, 1, 0, alpha_end, "Alpha block here."),
            make_window(
                "w2",
                1,
                1,
                beta_start,
                beta_end,
                "Beta block here.",
            ),
            make_window(
                "w3",
                2,
                1,
                gamma_start,
                gamma_end,
                "Gamma block here.",
            ),
        ];

        let report = build_repetition_report("job-1", doc, &windows, true);
        assert_eq!(report.clusters.len(), 1);
        let cluster = &report.clusters[0];
        assert_eq!(cluster.instance_count, 3);
        assert_eq!(cluster.duplicates.len(), 2);
        assert_eq!(cluster.canonical.text, "Alpha block here.");
        assert_eq!(cluster.duplicates[0].text, "Beta block here.");
        assert_eq!(cluster.duplicates[1].text, "Gamma block here.");
        assert_eq!(report.stats.total_duplicate_instances, 2);
    }
}
