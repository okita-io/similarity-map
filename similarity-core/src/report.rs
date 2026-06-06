//! Editorial repetition report for downstream tools (e.g. Romance Factory).

use serde::{Deserialize, Serialize};

use crate::centroid::{build_cluster_registry, WindowData};
use crate::importer::{import_document, ImportDocumentParams};
use crate::job_data::parse_window_data_from_batches;
use crate::spans::{expand_to_sentence_boundaries, merge_overlapping_spans};
use crate::storage::{Storage, StorageError};
use crate::types::{AppError, ClusterRegistry, Page, SessionError};

/// Current repetition report schema version string.
pub const SCHEMA_VERSION: &str = "1";

/// Editorial operation suggested for a repetition cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedOp {
    /// Keep the canonical instance; no edit required on duplicates beyond optional polish.
    Keep,
    /// Rewrite duplicate instances in place.
    Rewrite,
    /// Remove duplicate instances (same-act near-exact echo).
    Remove,
    /// Insert transitional bridge prose between acts before rewriting.
    Bridge,
}

fn default_suggested_op() -> SuggestedOp {
    SuggestedOp::Rewrite
}

/// What manuscript unit this analysis covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisScope {
    /// 1-based chapter number within the story.
    pub chapter: u32,
    /// 1-based act when the pass targets a single act; omitted for whole-chapter scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_hash: Option<String>,
    /// Character range of this scope within the chapter text (inclusive start, exclusive end).
    pub scope_char_start: u32,
    pub scope_char_end: u32,
    /// Absolute document offsets for the same range.
    pub doc_char_start: u32,
    pub doc_char_end: u32,
}

/// Analysis tuning parameters recorded on an enriched repetition report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportAnalysisParams {
    pub window_size: u32,
    pub stride: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chapter_break_regex: Option<String>,
    pub min_repetitions: u32,
    pub min_samples: u32,
    pub enable_hdbscan: bool,
    pub link_subphrases: bool,
}

impl From<crate::analysis::AnalysisParams> for ReportAnalysisParams {
    fn from(params: crate::analysis::AnalysisParams) -> Self {
        Self {
            window_size: params.window_size,
            stride: params.stride,
            tokens_per_page: params.tokens_per_page,
            chapter_break_regex: params.chapter_break_regex,
            min_repetitions: params.min_repetitions,
            min_samples: params.min_samples,
            enable_hdbscan: params.enable_hdbscan,
            link_subphrases: params.link_subphrases,
        }
    }
}

/// One paragraph entry in the nested act index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParagraphSpan {
    /// 1-based paragraph index within the act.
    pub paragraph_index: u32,
    /// Stable segment id: `ch{NN}_a{MM}_p{PP}` (zero-padded).
    pub segment_id: String,
    pub scope_char_start: u32,
    pub scope_char_end: u32,
    pub doc_char_start: u32,
    pub doc_char_end: u32,
}

/// One act segment with nested paragraph index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeSegment {
    /// 1-based act number within the chapter.
    pub act: u32,
    pub scope_char_start: u32,
    pub scope_char_end: u32,
    pub doc_char_start: u32,
    pub doc_char_end: u32,
    pub paragraphs: Vec<ParagraphSpan>,
}

/// Structural index mapping scope-local and document offsets to act/paragraph segments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeManifest {
    pub chapter: u32,
    pub acts: Vec<ScopeSegment>,
}

/// Resolved location for an edit span within chapter structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanLocation {
    pub chapter: u32,
    pub act: u32,
    pub paragraph_index: u32,
    pub segment_id: String,
    /// 1-based sentence index within the paragraph segment.
    pub sentence_index: u32,
    pub scope_char_start: u32,
    pub scope_char_end: u32,
    pub doc_char_start: u32,
    pub doc_char_end: u32,
}

/// Format a segment id: `ch01_a02_p03`.
pub fn format_segment_id(chapter: u32, act: u32, paragraph_index: u32) -> String {
    format!("ch{chapter:02}_a{act:02}_p{paragraph_index:02}")
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SpanLocation>,
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
    #[serde(default)]
    pub cross_act: bool,
    #[serde(default = "default_suggested_op")]
    pub suggested_op: SuggestedOp,
    #[serde(default)]
    pub needs_bridge: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<AnalysisScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_params: Option<ReportAnalysisParams>,
}

/// Reconstruct full document text from paginated import output.
pub fn pages_to_document_text(pages: &[Page]) -> String {
    pages.iter().map(|p| p.text.as_str()).collect()
}

/// Derive cluster editorial enrichments from span locations and similarities.
pub fn derive_cluster_enrichments(spans: &[EditSpan]) -> (bool, bool, SuggestedOp) {
    let acts: std::collections::HashSet<u32> = spans
        .iter()
        .filter_map(|span| span.location.as_ref().map(|loc| loc.act))
        .collect();
    let cross_act = acts.len() > 1;
    let needs_bridge = cross_act
        && spans
            .iter()
            .any(|span| span.similarity_to_centroid >= 0.85);
    let duplicates: Vec<&EditSpan> = spans.iter().skip(1).collect();
    let suggested_op = if needs_bridge {
        SuggestedOp::Bridge
    } else if cross_act {
        SuggestedOp::Rewrite
    } else if duplicates
        .iter()
        .all(|span| span.similarity_to_centroid >= 0.95)
    {
        SuggestedOp::Remove
    } else {
        SuggestedOp::Rewrite
    };
    (cross_act, needs_bridge, suggested_op)
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
                        location: None,
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
            let (cross_act, needs_bridge, suggested_op) = derive_cluster_enrichments(&edit_spans);

            Some(ClusterSummary {
                cluster_id: info.cluster_id,
                representative_text: info.most_central_window_text.clone(),
                instance_count: edit_spans.len() as u32,
                total_word_estimate,
                canonical,
                duplicates,
                spans: edit_spans,
                cross_act,
                suggested_op,
                needs_bridge,
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
        schema_version: None,
        scope: None,
        analysis_params: None,
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

    fn sample_span_location() -> SpanLocation {
        SpanLocation {
            chapter: 1,
            act: 1,
            paragraph_index: 1,
            segment_id: "ch01_a01_p01".to_string(),
            sentence_index: 1,
            scope_char_start: 12,
            scope_char_end: 38,
            doc_char_start: 12,
            doc_char_end: 38,
        }
    }

    fn sample_scope_manifest() -> ScopeManifest {
        ScopeManifest {
            chapter: 1,
            acts: vec![ScopeSegment {
                act: 1,
                scope_char_start: 0,
                scope_char_end: 198,
                doc_char_start: 0,
                doc_char_end: 198,
                paragraphs: vec![ParagraphSpan {
                    paragraph_index: 1,
                    segment_id: "ch01_a01_p01".to_string(),
                    scope_char_start: 0,
                    scope_char_end: 95,
                    doc_char_start: 0,
                    doc_char_end: 95,
                }],
            }],
        }
    }

    #[test]
    fn format_segment_id_zero_pads() {
        assert_eq!(format_segment_id(3, 2, 7), "ch03_a02_p07");
    }

    #[test]
    fn scope_manifest_roundtrip_json() {
        let manifest = sample_scope_manifest();
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: ScopeManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, manifest);
        assert_eq!(parsed.acts[0].paragraphs[0].segment_id, "ch01_a01_p01");
    }

    #[test]
    fn span_location_roundtrip_json() {
        let location = sample_span_location();
        let json = serde_json::to_string(&location).unwrap();
        let parsed: SpanLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, location);
    }

    #[test]
    fn edit_span_roundtrip_json_with_optional_location() {
        let span = EditSpan {
            cluster_id: 1,
            instance_id: 1,
            doc_char_start: 12,
            doc_char_end: 38,
            text: "the velvet darkness pooled".to_string(),
            similarity_to_centroid: 1.0,
            member_window_count: 3,
            location: Some(sample_span_location()),
        };
        let json = serde_json::to_string(&span).unwrap();
        let parsed: EditSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, span);
        assert!(parsed.location.is_some());
    }

    #[test]
    fn edit_span_roundtrip_json_without_location() {
        let span = EditSpan {
            cluster_id: 1,
            instance_id: 1,
            doc_char_start: 0,
            doc_char_end: 17,
            text: "Alpha block here.".to_string(),
            similarity_to_centroid: 1.0,
            member_window_count: 1,
            location: None,
        };
        let json = serde_json::to_string(&span).unwrap();
        assert!(!json.contains("location"));
        let parsed: EditSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, span);
    }

    #[test]
    fn cluster_summary_roundtrip_json() {
        let canonical = EditSpan {
            cluster_id: 1,
            instance_id: 1,
            doc_char_start: 12,
            doc_char_end: 38,
            text: "the velvet darkness pooled".to_string(),
            similarity_to_centroid: 1.0,
            member_window_count: 3,
            location: Some(sample_span_location()),
        };
        let duplicate = EditSpan {
            cluster_id: 1,
            instance_id: 2,
            doc_char_start: 110,
            doc_char_end: 136,
            text: "the velvet darkness pooled".to_string(),
            similarity_to_centroid: 0.97,
            member_window_count: 2,
            location: Some(SpanLocation {
                paragraph_index: 2,
                segment_id: "ch01_a01_p02".to_string(),
                ..sample_span_location()
            }),
        };
        let cluster = ClusterSummary {
            cluster_id: 1,
            representative_text: "the velvet darkness pooled".to_string(),
            instance_count: 2,
            total_word_estimate: 8,
            canonical: canonical.clone(),
            duplicates: vec![duplicate.clone()],
            spans: vec![canonical, duplicate],
            cross_act: false,
            suggested_op: SuggestedOp::Remove,
            needs_bridge: false,
        };
        let json = serde_json::to_string(&cluster).unwrap();
        let parsed: ClusterSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cluster);
        assert_eq!(parsed.suggested_op, SuggestedOp::Remove);
    }

    #[test]
    fn repetition_report_roundtrip_json() {
        let report = RepetitionReport {
            job_id: "job-1".to_string(),
            clusters: vec![],
            stats: AnalysisStats {
                cluster_count: 0,
                total_duplicate_instances: 0,
                total_duplicate_words_estimate: 0,
            },
            schema_version: Some(SCHEMA_VERSION.to_string()),
            scope: Some(AnalysisScope {
                chapter: 1,
                act: None,
                document_path: Some("stories/x/drafts/chapter_01.md".to_string()),
                document_hash: None,
                scope_char_start: 0,
                scope_char_end: 512,
                doc_char_start: 0,
                doc_char_end: 512,
            }),
            analysis_params: Some(ReportAnalysisParams {
                window_size: 50,
                stride: 10,
                tokens_per_page: Some(400),
                chapter_break_regex: None,
                min_repetitions: 3,
                min_samples: 3,
                enable_hdbscan: true,
                link_subphrases: false,
            }),
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: RepetitionReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, report);
        assert_eq!(parsed.schema_version.as_deref(), Some(SCHEMA_VERSION));
    }

    #[test]
    fn derive_cluster_enrichments_cross_act_bridge() {
        let spans = vec![
            EditSpan {
                cluster_id: 1,
                instance_id: 1,
                doc_char_start: 0,
                doc_char_end: 10,
                text: "first".to_string(),
                similarity_to_centroid: 1.0,
                member_window_count: 1,
                location: Some(SpanLocation {
                    act: 1,
                    ..sample_span_location()
                }),
            },
            EditSpan {
                cluster_id: 1,
                instance_id: 2,
                doc_char_start: 100,
                doc_char_end: 110,
                text: "second".to_string(),
                similarity_to_centroid: 0.9,
                member_window_count: 1,
                location: Some(SpanLocation {
                    act: 2,
                    paragraph_index: 1,
                    segment_id: "ch01_a02_p01".to_string(),
                    ..sample_span_location()
                }),
            },
        ];
        let (cross_act, needs_bridge, suggested_op) = derive_cluster_enrichments(&spans);
        assert!(cross_act);
        assert!(needs_bridge);
        assert_eq!(suggested_op, SuggestedOp::Bridge);
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
        assert!(!cluster.cross_act);
        assert_eq!(cluster.suggested_op, SuggestedOp::Remove);
        assert!(cluster.canonical.location.is_none());
        assert!(report.schema_version.is_none());
    }
}
