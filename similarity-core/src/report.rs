//! Editorial repetition report for downstream tools (e.g. Romance Factory).

use serde::{Deserialize, Serialize};

use crate::centroid::{build_cluster_registry, WindowData};
use crate::importer::{import_document, ImportDocumentParams};
use crate::job_data::parse_window_data_from_batches;
use crate::spans::{
    expand_to_sentence_boundaries, merge_overlapping_spans, sentence_index_at_char_offset,
};
use crate::storage::{Storage, StorageError};
use crate::types::{AppError, ClusterRegistry, Page, SessionError};

/// Current repetition report schema version string.
pub const SCHEMA_VERSION: &str = "1";

/// Sentence-boundary expansion version — must match Romance Factory span expansion.
pub const BOUNDARY_VERSION: u32 = 1;

/// Word-count threshold for whole-paragraph surgical ops and mid-act bridge hints.
pub const PARAGRAPH_WORD_THRESHOLD: u32 = 40;

/// Max duplicate span word count eligible for `delete_span`.
pub const DELETE_SPAN_MAX_WORDS: u32 = 15;

/// Minimum similarity for near-exact `delete_span` routing.
pub const HIGH_SIMILARITY: f32 = 0.95;

/// Editorial operation suggested for a repetition cluster — maps 1:1 to RF PatchPlanner ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedOp {
    /// Remove a near-exact duplicate phrase (`delete_span`).
    DeleteSpan,
    /// Rewrite duplicate in place with LLM (`rewrite_span`).
    RewriteSpan,
    /// Replace an entire paragraph-sized echo (`replace_paragraph`).
    ReplaceParagraph,
}

fn default_suggested_op() -> SuggestedOp {
    SuggestedOp::RewriteSpan
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

/// Derive cluster editorial enrichments from span locations, similarities, and word counts.
pub fn derive_cluster_enrichments(spans: &[EditSpan]) -> (bool, bool, SuggestedOp) {
    let instances: Vec<(u32, f32, &str)> = spans
        .iter()
        .filter_map(|span| {
            span.location
                .as_ref()
                .map(|loc| (loc.act, span.similarity_to_centroid, span.text.as_str()))
        })
        .collect();
    if instances.len() == spans.len() && !instances.is_empty() {
        return derive_enrichments_from_instances(&instances);
    }

    // Fallback when locations are unresolved — same-act fuzzy rewrite.
    let duplicates: Vec<_> = spans.iter().skip(1).collect();
    let blast = duplicate_blast_radius_words(duplicates.iter().map(|s| s.text.as_str()));
    let all_high = duplicates
        .iter()
        .all(|span| span.similarity_to_centroid >= HIGH_SIMILARITY);
    let suggested_op = if all_high && blast <= DELETE_SPAN_MAX_WORDS {
        SuggestedOp::DeleteSpan
    } else if blast > PARAGRAPH_WORD_THRESHOLD {
        SuggestedOp::ReplaceParagraph
    } else {
        SuggestedOp::RewriteSpan
    };
    (false, blast > PARAGRAPH_WORD_THRESHOLD, suggested_op)
}

/// Like [`derive_cluster_enrichments`] for v1 contract spans (always location-resolved).
pub fn derive_cluster_enrichments_v1(
    spans: &[crate::contract::EditSpanV1],
) -> (bool, bool, SuggestedOp) {
    let instances: Vec<(u32, f32, &str)> = spans
        .iter()
        .map(|span| {
            (
                span.location.act,
                span.similarity_to_centroid,
                span.text.as_str(),
            )
        })
        .collect();
    derive_enrichments_from_instances(&instances)
}

fn derive_enrichments_from_instances(instances: &[(u32, f32, &str)]) -> (bool, bool, SuggestedOp) {
    if instances.is_empty() {
        return (false, false, SuggestedOp::RewriteSpan);
    }

    let acts: std::collections::HashSet<u32> = instances.iter().map(|(act, _, _)| *act).collect();
    let cross_act = acts.len() > 1;
    let duplicates = &instances[1..];
    let blast = duplicate_blast_radius_words(duplicates.iter().map(|(_, _, text)| *text));
    let all_high = duplicates.iter().all(|(_, sim, _)| *sim >= HIGH_SIMILARITY);

    let suggested_op = if cross_act {
        if blast > PARAGRAPH_WORD_THRESHOLD {
            SuggestedOp::ReplaceParagraph
        } else {
            SuggestedOp::RewriteSpan
        }
    } else if all_high && blast <= DELETE_SPAN_MAX_WORDS {
        SuggestedOp::DeleteSpan
    } else if blast > PARAGRAPH_WORD_THRESHOLD {
        SuggestedOp::ReplaceParagraph
    } else {
        SuggestedOp::RewriteSpan
    };

    // Mid-act paragraph-sized duplicate — insert bridge prose before destructive edit.
    let needs_bridge = !cross_act && blast > PARAGRAPH_WORD_THRESHOLD;

    (cross_act, needs_bridge, suggested_op)
}

/// Max whitespace-delimited word count across duplicate instance texts (surgical blast radius).
pub fn duplicate_blast_radius_words<'a>(duplicate_texts: impl Iterator<Item = &'a str>) -> u32 {
    duplicate_texts
        .map(|text| text.split_whitespace().count() as u32)
        .max()
        .unwrap_or(0)
}

/// Resolve structural location for a document span against a scope manifest.
///
/// `scope_char_start` / `scope_char_end` are act-relative offsets suitable for surgical
/// [`PatchTarget`] resolution within the containing act; `doc_char_*` remain absolute.
pub fn resolve_span_location(
    document_text: &str,
    manifest: &ScopeManifest,
    doc_char_start: u32,
    doc_char_end: u32,
) -> SpanLocation {
    for act in &manifest.acts {
        if doc_char_start >= act.doc_char_start && doc_char_start < act.doc_char_end {
            for para in &act.paragraphs {
                if doc_char_start >= para.doc_char_start && doc_char_start < para.doc_char_end {
                    let para_text =
                        slice_document_text(document_text, para.doc_char_start, para.doc_char_end);
                    let local_start = doc_char_start.saturating_sub(para.doc_char_start) as usize;
                    let sentence_index = sentence_index_at_char_offset(&para_text, local_start);
                    return SpanLocation {
                        chapter: manifest.chapter,
                        act: act.act,
                        paragraph_index: para.paragraph_index,
                        segment_id: para.segment_id.clone(),
                        sentence_index,
                        scope_char_start: doc_char_start.saturating_sub(act.doc_char_start),
                        scope_char_end: doc_char_end.saturating_sub(act.doc_char_start),
                        doc_char_start,
                        doc_char_end,
                    };
                }
            }

            return SpanLocation {
                chapter: manifest.chapter,
                act: act.act,
                paragraph_index: 1,
                segment_id: format_segment_id(manifest.chapter, act.act, 1),
                sentence_index: 1,
                scope_char_start: doc_char_start.saturating_sub(act.doc_char_start),
                scope_char_end: doc_char_end.saturating_sub(act.doc_char_start),
                doc_char_start,
                doc_char_end,
            };
        }
    }

    let fallback_act = manifest.acts.first();
    let act_num = fallback_act.map(|a| a.act).unwrap_or(1);
    let act_doc = fallback_act.map(|a| a.doc_char_start).unwrap_or(0);
    SpanLocation {
        chapter: manifest.chapter,
        act: act_num,
        paragraph_index: 1,
        segment_id: format_segment_id(manifest.chapter, act_num, 1),
        sentence_index: 1,
        scope_char_start: doc_char_start.saturating_sub(act_doc),
        scope_char_end: doc_char_end.saturating_sub(act_doc),
        doc_char_start,
        doc_char_end,
    }
}

fn find_act_for_doc_char<'a>(
    manifest: &'a ScopeManifest,
    doc_char: u32,
) -> Option<&'a ScopeSegment> {
    manifest
        .acts
        .iter()
        .find(|act| doc_char >= act.doc_char_start && doc_char < act.doc_char_end)
}

/// Clip and optionally expand a span within its containing act.
fn clip_and_expand_span_in_act(
    document_text: &str,
    manifest: &ScopeManifest,
    doc_char_start: u32,
    doc_char_end: u32,
    expand_to_sentences: bool,
) -> (u32, u32) {
    let Some(act) = find_act_for_doc_char(manifest, doc_char_start) else {
        return if expand_to_sentences {
            expand_to_sentence_boundaries(document_text, doc_char_start, doc_char_end)
        } else {
            (doc_char_start, doc_char_end)
        };
    };

    let clipped_start = doc_char_start.max(act.doc_char_start);
    let clipped_end = doc_char_end.min(act.doc_char_end);

    if !expand_to_sentences {
        return (clipped_start, clipped_end);
    }

    let act_text = slice_document_text(document_text, act.doc_char_start, act.doc_char_end);
    let local_start = clipped_start - act.doc_char_start;
    let local_end = clipped_end - act.doc_char_start;
    let (exp_local_start, exp_local_end) =
        expand_to_sentence_boundaries(&act_text, local_start, local_end);

    (
        act.doc_char_start + exp_local_start,
        act.doc_char_start + exp_local_end,
    )
}

/// Build an editorial repetition report from clustered window data.
///
/// When `expand_to_sentences` is true, each merged instance span is expanded
/// to sentence (or paragraph) boundaries before extracting text. When `manifest`
/// is provided, spans are clipped to their act first and expansion runs on act
/// text only; each [`EditSpan`] receives a resolved [`SpanLocation`].
pub fn build_repetition_report(
    job_id: &str,
    document_text: &str,
    windows: &[WindowData],
    expand_to_sentences: bool,
) -> RepetitionReport {
    build_repetition_report_with_manifest(job_id, document_text, windows, expand_to_sentences, None)
}

/// Like [`build_repetition_report`] but resolves [`SpanLocation`] when `manifest` is set.
pub fn build_repetition_report_with_manifest(
    job_id: &str,
    document_text: &str,
    windows: &[WindowData],
    expand_to_sentences: bool,
    manifest: Option<&ScopeManifest>,
) -> RepetitionReport {
    let registry = build_cluster_registry(windows);
    build_repetition_report_from_registry(
        job_id,
        document_text,
        windows,
        &registry,
        expand_to_sentences,
        manifest,
    )
}

pub fn build_repetition_report_from_registry(
    job_id: &str,
    document_text: &str,
    windows: &[WindowData],
    registry: &ClusterRegistry,
    expand_to_sentences: bool,
    manifest: Option<&ScopeManifest>,
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
                    let (start, end) = if let Some(manifest) = manifest {
                        clip_and_expand_span_in_act(
                            document_text,
                            manifest,
                            span.doc_char_start,
                            span.doc_char_end,
                            expand_to_sentences,
                        )
                    } else if expand_to_sentences {
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
                    let location =
                        manifest.map(|m| resolve_span_location(document_text, m, start, end));

                    EditSpan {
                        cluster_id: info.cluster_id,
                        instance_id: (idx + 1) as u32,
                        doc_char_start: start,
                        doc_char_end: end,
                        text,
                        similarity_to_centroid: similarity,
                        member_window_count: span.member_window_count,
                        location,
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
    let overlapping: Vec<&&WindowData> = cluster_windows
        .iter()
        .filter(|w| w.doc_char_start < end && w.doc_char_end > start)
        .collect();
    if overlapping.is_empty() {
        return 0.0;
    }

    // Centroid over the full cluster, then score overlapping members for real values.
    let dim = cluster_windows
        .iter()
        .map(|w| w.embedding.len())
        .max()
        .unwrap_or(0);
    if dim == 0 {
        return 0.0;
    }
    let mut centroid = vec![0.0f32; dim];
    let mut counted = 0f32;
    for w in cluster_windows {
        if w.embedding.len() != dim {
            continue;
        }
        for (i, v) in w.embedding.iter().enumerate() {
            centroid[i] += *v;
        }
        counted += 1.0;
    }
    if counted == 0.0 {
        return 0.0;
    }
    for v in centroid.iter_mut() {
        *v /= counted;
    }

    overlapping
        .iter()
        .filter_map(|w| {
            if w.embedding.len() != dim {
                return None;
            }
            Some(cosine_similarity_f32(&w.embedding, &centroid))
        })
        .fold(0.0_f32, f32::max)
}

fn cosine_similarity_f32(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denom = norm_a * norm_b;
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::centroid::WindowData;
    use crate::contract::build_scope_manifest;

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
            suggested_op: SuggestedOp::DeleteSpan,
            needs_bridge: false,
        };
        let json = serde_json::to_string(&cluster).unwrap();
        let parsed: ClusterSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cluster);
        assert_eq!(parsed.suggested_op, SuggestedOp::DeleteSpan);
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
    fn derive_cluster_enrichments_cross_act_rewrite() {
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
        assert!(!needs_bridge);
        assert_eq!(suggested_op, SuggestedOp::RewriteSpan);
    }

    #[test]
    fn derive_cluster_enrichments_mid_act_paragraph_needs_bridge() {
        let long_text = "word ".repeat(45);
        let spans = vec![
            EditSpan {
                cluster_id: 1,
                instance_id: 1,
                doc_char_start: 0,
                doc_char_end: 100,
                text: long_text.clone(),
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
                doc_char_start: 200,
                doc_char_end: 300,
                text: long_text,
                similarity_to_centroid: 0.99,
                member_window_count: 1,
                location: Some(SpanLocation {
                    act: 1,
                    paragraph_index: 2,
                    segment_id: "ch01_a01_p02".to_string(),
                    ..sample_span_location()
                }),
            },
        ];
        let (cross_act, needs_bridge, suggested_op) = derive_cluster_enrichments(&spans);
        assert!(!cross_act);
        assert!(needs_bridge);
        assert_eq!(suggested_op, SuggestedOp::ReplaceParagraph);
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
            make_window("w2", 1, 1, beta_start, beta_end, "Beta block here."),
            make_window("w3", 2, 1, gamma_start, gamma_end, "Gamma block here."),
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
        assert_eq!(cluster.suggested_op, SuggestedOp::DeleteSpan);
        assert!(cluster.canonical.location.is_none());
        assert!(report.schema_version.is_none());
    }

    #[test]
    fn resolve_span_location_maps_act_and_paragraph() {
        let doc = "Act one para one.\n\nAct two para one.";
        let manifest = build_scope_manifest(1, doc, 0);
        let act2_start = doc.find("Act two").unwrap() as u32;
        let loc = resolve_span_location(
            doc,
            &manifest,
            act2_start,
            act2_start + "Act two".len() as u32,
        );
        assert_eq!(loc.act, 2);
        assert_eq!(loc.segment_id, "ch01_a02_p01");
        assert_eq!(loc.scope_char_start, 0);
    }

    #[test]
    fn cross_act_duplicate_sets_cross_act_with_manifest() {
        let doc = "Echo phrase here.\n\nEcho phrase here.";
        let act2_start = doc.find("\n\n").unwrap() + 2;
        let phrase_len = "Echo phrase here.".len() as u32;

        let windows = vec![
            make_window("w1", 0, 1, 0, phrase_len, "Echo phrase here."),
            make_window(
                "w2",
                1,
                1,
                act2_start as u32,
                act2_start as u32 + phrase_len,
                "Echo phrase here.",
            ),
        ];
        let manifest = build_scope_manifest(1, doc, 0);
        let report =
            build_repetition_report_with_manifest("job-x", doc, &windows, false, Some(&manifest));

        assert_eq!(report.clusters.len(), 1);
        let cluster = &report.clusters[0];
        assert!(cluster.cross_act);
        assert_eq!(cluster.duplicates.len(), 1);
        assert_eq!(cluster.canonical.location.as_ref().unwrap().act, 1);
        assert_eq!(cluster.duplicates[0].location.as_ref().unwrap().act, 2);
    }

    #[test]
    fn sentence_expansion_runs_per_act_after_clip() {
        let doc = "First act ends here.\n\nSecond act begins now.";
        let act1_phrase = "First act ends here.";
        let act1_end = act1_phrase.len() as u32;
        let inner_start = "First act ends".len() as u32;
        let inner_end = inner_start + 4;

        let windows = vec![make_window("w1", 0, 1, inner_start, inner_end, "ends")];
        let manifest = build_scope_manifest(1, doc, 0);

        let report =
            build_repetition_report_with_manifest("job-y", doc, &windows, true, Some(&manifest));

        let cluster = &report.clusters[0];
        assert_eq!(cluster.canonical.text, act1_phrase);
        assert!(cluster.canonical.doc_char_end <= act1_end);
        assert!(!cluster.canonical.text.contains("Second act"));
    }
}
