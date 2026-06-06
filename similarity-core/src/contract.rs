//! RepetitionReport v1 JSON contract — pipeline-consumable output for Romance Factory.
//!
//! See `schemas/analysis_output_v1.schema.json` and
//! `.kiro/specs/similarity-map/integration-contract.md` for the canonical spec.

use serde::{Deserialize, Serialize};

use crate::report::{
    derive_cluster_enrichments, format_segment_id, AnalysisScope, AnalysisStats, EditSpan,
    ParagraphSpan, RepetitionReport, ScopeManifest, ScopeSegment, SpanLocation, SuggestedOp,
    SCHEMA_VERSION,
};

pub use crate::report::resolve_span_location;

/// JSON contract alias for [`ScopeSegment`].
pub type ActSegment = ScopeSegment;

/// JSON contract alias for [`ParagraphSpan`].
pub type ParagraphIndexEntry = ParagraphSpan;

/// Document span with structural location — v1 [`EditSpan`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditSpanV1 {
    pub location: SpanLocation,
    pub cluster_id: i32,
    /// 1-based instance index within the cluster (document order).
    pub instance_id: u32,
    pub text: String,
    pub similarity_to_centroid: f32,
    pub member_window_count: u32,
}

/// Cluster summary with editorial enrichments for downstream surgical editing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterSummaryV1 {
    pub cluster_id: i32,
    pub representative_text: String,
    pub instance_count: u32,
    pub total_word_estimate: u32,
    pub canonical: EditSpanV1,
    pub duplicates: Vec<EditSpanV1>,
    pub spans: Vec<EditSpanV1>,
    pub suggested_op: SuggestedOp,
    /// True when duplicate instances span more than one act.
    pub cross_act: bool,
    /// True when cross-act repetition likely needs a transitional bridge before rewrite.
    pub needs_bridge: bool,
}

/// Editorial repetition report — v1 contract shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepetitionReportV1 {
    pub job_id: String,
    pub clusters: Vec<ClusterSummaryV1>,
    pub stats: AnalysisStats,
}

/// One analysis pass (act window, chapter window/stride bundle, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisPassRecord {
    pub pass_id: String,
    pub pass_label: String,
    pub scope: AnalysisScope,
    pub window_size: u32,
    pub stride: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_page: Option<u32>,
    pub repetition_report: RepetitionReportV1,
}

/// Top-level pipeline envelope shared by UI, CLI, PyO3, and Romance Factory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisOutput {
    pub schema_version: String,
    pub scope: AnalysisScope,
    pub scope_manifest: ScopeManifest,
    pub passes: Vec<AnalysisPassRecord>,
    pub merged_repetition_report: RepetitionReportV1,
}

/// Build a scope manifest from chapter text split into acts (blocks separated by `\n\n`).
///
/// Each act is a slice of the chapter; paragraphs are non-empty lines within an act.
pub fn build_scope_manifest(
    chapter: u32,
    chapter_text: &str,
    doc_char_offset: u32,
) -> ScopeManifest {
    let mut acts = Vec::new();
    let mut act_num = 0u32;
    let mut search_start = 0usize;

    while search_start < chapter_text.len() {
        while search_start < chapter_text.len() {
            if chapter_text[search_start..].starts_with("\n\n") {
                search_start += 2;
            } else if let Some(ch) = chapter_text[search_start..].chars().next() {
                if ch.is_whitespace() && ch != '\n' {
                    search_start += ch.len_utf8();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if search_start >= chapter_text.len() {
            break;
        }

        let act_end = chapter_text[search_start..]
            .find("\n\n")
            .map(|idx| search_start + idx)
            .unwrap_or(chapter_text.len());
        let act_slice = &chapter_text[search_start..act_end];
        let act_trimmed = act_slice.trim();
        if act_trimmed.is_empty() {
            search_start = act_end;
            continue;
        }

        act_num += 1;
        let act_scope_start = search_start as u32;
        let act_scope_end = (search_start + act_slice.len()) as u32;
        let act_doc_start = doc_char_offset + act_scope_start;
        let act_doc_end = doc_char_offset + act_scope_end;

        let mut paragraphs = Vec::new();
        let mut para_num = 0u32;
        let mut offset_in_act = 0usize;
        for line in act_slice.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                offset_in_act += line.len() + 1;
                continue;
            }
            let trimmed_start = line.find(trimmed).unwrap_or(0);
            let para_offset = offset_in_act + trimmed_start;
            para_num += 1;
            let para_len = trimmed.len() as u32;
            let para_scope_start = act_scope_start + para_offset as u32;
            paragraphs.push(ParagraphSpan {
                paragraph_index: para_num,
                segment_id: format_segment_id(chapter, act_num, para_num),
                scope_char_start: para_scope_start,
                scope_char_end: para_scope_start + para_len,
                doc_char_start: doc_char_offset + para_scope_start,
                doc_char_end: doc_char_offset + para_scope_start + para_len,
            });
            offset_in_act += line.len() + 1;
        }

        acts.push(ScopeSegment {
            act: act_num,
            scope_char_start: act_scope_start,
            scope_char_end: act_scope_end,
            doc_char_start: act_doc_start,
            doc_char_end: act_doc_end,
            paragraphs,
        });

        search_start = act_end;
    }

    ScopeManifest { chapter, acts }
}

/// Convert a legacy [`RepetitionReport`] into v1 with locations and cluster enrichments.
pub fn repetition_report_to_v1(
    report: &RepetitionReport,
    manifest: &ScopeManifest,
    _scope_base_doc: u32,
    chapter_text: &str,
) -> RepetitionReportV1 {
    let clusters = report
        .clusters
        .iter()
        .map(|cluster| cluster_summary_to_v1(cluster, manifest, chapter_text))
        .collect();

    RepetitionReportV1 {
        job_id: report.job_id.clone(),
        clusters,
        stats: report.stats.clone(),
    }
}

fn cluster_summary_to_v1(
    cluster: &crate::report::ClusterSummary,
    manifest: &ScopeManifest,
    chapter_text: &str,
) -> ClusterSummaryV1 {
    let spans: Vec<EditSpanV1> = cluster
        .spans
        .iter()
        .map(|span| edit_span_to_v1(span, manifest, chapter_text))
        .collect();

    let canonical = spans.first().cloned().unwrap_or_else(|| {
        edit_span_to_v1(&cluster.canonical, manifest, chapter_text)
    });
    let duplicates: Vec<EditSpanV1> = spans.iter().skip(1).cloned().collect();

    let (cross_act, needs_bridge, suggested_op) = if cluster.spans.iter().any(|s| s.location.is_some())
    {
        derive_cluster_enrichments(&cluster.spans)
    } else {
        (
            cluster.cross_act,
            cluster.needs_bridge,
            cluster.suggested_op,
        )
    };

    ClusterSummaryV1 {
        cluster_id: cluster.cluster_id,
        representative_text: cluster.representative_text.clone(),
        instance_count: cluster.instance_count,
        total_word_estimate: cluster.total_word_estimate,
        canonical,
        duplicates,
        spans,
        suggested_op,
        cross_act,
        needs_bridge,
    }
}

fn edit_span_to_v1(
    span: &EditSpan,
    manifest: &ScopeManifest,
    chapter_text: &str,
) -> EditSpanV1 {
    let location = span.location.clone().unwrap_or_else(|| {
        resolve_span_location(
            chapter_text,
            manifest,
            span.doc_char_start,
            span.doc_char_end,
        )
    });
    EditSpanV1 {
        location,
        cluster_id: span.cluster_id,
        instance_id: span.instance_id,
        text: span.text.clone(),
        similarity_to_centroid: span.similarity_to_centroid,
        member_window_count: span.member_window_count,
    }
}

/// Merge multiple pass reports into one editorial report.
///
/// See `.kiro/specs/similarity-map/integration-contract.md` for merge rules.
pub fn merge_pass_reports(passes: &[AnalysisPassRecord]) -> RepetitionReportV1 {
    if passes.is_empty() {
        return RepetitionReportV1 {
            job_id: String::new(),
            clusters: Vec::new(),
            stats: AnalysisStats {
                cluster_count: 0,
                total_duplicate_instances: 0,
                total_duplicate_words_estimate: 0,
            },
        };
    }

    if passes.len() == 1 {
        return passes[0].repetition_report.clone();
    }

    let job_id = passes[0].repetition_report.job_id.clone();
    let mut cluster_map: std::collections::BTreeMap<i32, ClusterSummaryV1> =
        std::collections::BTreeMap::new();
    let mut next_cluster_id = 1i32;

    for pass in passes {
        for cluster in &pass.repetition_report.clusters {
            let merged_id = find_merge_target(&cluster_map, cluster)
                .unwrap_or_else(|| {
                    let id = next_cluster_id;
                    next_cluster_id += 1;
                    id
                });

            cluster_map
                .entry(merged_id)
                .and_modify(|existing| merge_cluster_summaries(existing, cluster))
                .or_insert_with(|| rekey_cluster(cluster, merged_id));
        }
    }

    let mut clusters: Vec<ClusterSummaryV1> = cluster_map.into_values().collect();
    for cluster in &mut clusters {
        recompute_cluster_enrichments(cluster);
        cluster.spans.sort_by_key(|s| s.location.doc_char_start);
        for (idx, span) in cluster.spans.iter_mut().enumerate() {
            span.instance_id = (idx + 1) as u32;
        }
        if let Some(first) = cluster.spans.first() {
            cluster.canonical = first.clone();
        }
        cluster.duplicates = cluster.spans.iter().skip(1).cloned().collect();
        cluster.instance_count = cluster.spans.len() as u32;
    }
    clusters.sort_by_key(|c| c.canonical.location.doc_char_start);

    let cluster_count = clusters.len() as u32;
    let total_duplicate_instances: u32 = clusters.iter().map(|c| c.duplicates.len() as u32).sum();
    let total_duplicate_words_estimate: u32 = clusters
        .iter()
        .flat_map(|c| c.duplicates.iter())
        .map(|s| s.text.split_whitespace().count() as u32)
        .sum();

    RepetitionReportV1 {
        job_id,
        clusters,
        stats: AnalysisStats {
            cluster_count,
            total_duplicate_instances,
            total_duplicate_words_estimate,
        },
    }
}

fn find_merge_target(
    existing: &std::collections::BTreeMap<i32, ClusterSummaryV1>,
    incoming: &ClusterSummaryV1,
) -> Option<i32> {
    for (id, cluster) in existing {
        if clusters_overlap(cluster, incoming) {
            return Some(*id);
        }
    }
    None
}

fn clusters_overlap(a: &ClusterSummaryV1, b: &ClusterSummaryV1) -> bool {
    a.spans.iter().any(|span_a| {
        b.spans.iter().any(|span_b| {
            spans_overlap(
                span_a.location.doc_char_start,
                span_a.location.doc_char_end,
                span_b.location.doc_char_start,
                span_b.location.doc_char_end,
            )
        })
    })
}

fn spans_overlap(a_start: u32, a_end: u32, b_start: u32, b_end: u32) -> bool {
    let overlap_start = a_start.max(b_start);
    let overlap_end = a_end.min(b_end);
    if overlap_end <= overlap_start {
        return false;
    }
    let overlap = overlap_end - overlap_start;
    let a_len = a_end.saturating_sub(a_start).max(1);
    let b_len = b_end.saturating_sub(b_start).max(1);
    let min_len = a_len.min(b_len);
    overlap * 100 / min_len >= 50
}

fn rekey_cluster(cluster: &ClusterSummaryV1, new_id: i32) -> ClusterSummaryV1 {
    let mut out = cluster.clone();
    out.cluster_id = new_id;
    for span in &mut out.spans {
        span.cluster_id = new_id;
    }
    out.canonical.cluster_id = new_id;
    for dup in &mut out.duplicates {
        dup.cluster_id = new_id;
    }
    out
}

fn merge_cluster_summaries(existing: &mut ClusterSummaryV1, incoming: &ClusterSummaryV1) {
    for span in &incoming.spans {
        if !existing
            .spans
            .iter()
            .any(|s| spans_overlap(
                s.location.doc_char_start,
                s.location.doc_char_end,
                span.location.doc_char_start,
                span.location.doc_char_end,
            ))
        {
            existing.spans.push(span.clone());
        }
    }
    if incoming.representative_text.len() > existing.representative_text.len() {
        existing.representative_text = incoming.representative_text.clone();
    }
}

fn recompute_cluster_enrichments(cluster: &mut ClusterSummaryV1) {
    let acts: std::collections::HashSet<u32> =
        cluster.spans.iter().map(|s| s.location.act).collect();
    cluster.cross_act = acts.len() > 1;
    cluster.needs_bridge = cluster.cross_act
        && cluster
            .spans
            .iter()
            .any(|s| s.similarity_to_centroid >= 0.85);
    cluster.suggested_op = if cluster.needs_bridge {
        SuggestedOp::Bridge
    } else if cluster.cross_act {
        SuggestedOp::Rewrite
    } else if cluster
        .duplicates
        .iter()
        .all(|d| d.similarity_to_centroid >= 0.95)
    {
        SuggestedOp::Remove
    } else {
        SuggestedOp::Rewrite
    };
}

/// Assemble a full [`AnalysisOutput`] from pass records and chapter text.
pub fn build_analysis_output(
    scope: AnalysisScope,
    chapter_text: &str,
    passes: Vec<AnalysisPassRecord>,
) -> AnalysisOutput {
    let scope_manifest = build_scope_manifest(scope.chapter, chapter_text, scope.doc_char_start);
    let merged_repetition_report = merge_pass_reports(&passes);

    AnalysisOutput {
        schema_version: SCHEMA_VERSION.to_string(),
        scope,
        scope_manifest,
        passes,
        merged_repetition_report,
    }
}

/// Serialize to pretty JSON for export fixtures and RF consumption.
pub fn to_export_json(output: &AnalysisOutput) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(output)
}

/// Parse and validate schema version on load.
pub fn from_export_json(json: &str) -> Result<AnalysisOutput, ContractError> {
    let output: AnalysisOutput = serde_json::from_str(json)?;
    validate_analysis_output(&output)?;
    Ok(output)
}

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Unsupported schema_version: {0}")]
    UnsupportedSchema(String),
    #[error("Validation error: {0}")]
    Validation(String),
}

/// Validate an analysis output envelope (schema version + structural invariants).
pub fn validate_analysis_output(output: &AnalysisOutput) -> Result<(), ContractError> {
    if output.schema_version != SCHEMA_VERSION {
        return Err(ContractError::UnsupportedSchema(output.schema_version.clone()));
    }
    if output.passes.is_empty() {
        return Err(ContractError::Validation(
            "passes must contain at least one pass record".into(),
        ));
    }
    if output.scope_manifest.chapter != output.scope.chapter {
        return Err(ContractError::Validation(
            "scope_manifest.chapter must match scope.chapter".into(),
        ));
    }
    for act in &output.scope_manifest.acts {
        for para in &act.paragraphs {
            if !para.segment_id.starts_with(&format!("ch{:02}", output.scope.chapter)) {
                return Err(ContractError::Validation(format!(
                    "segment_id {} does not match chapter {}",
                    para.segment_id, output.scope.chapter
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::build_repetition_report;

    #[test]
    fn scope_manifest_builds_act_paragraph_index() {
        let text = "Act one para one.\n\nAct two para one.\nAct two para two.";
        let manifest = build_scope_manifest(1, text, 0);
        assert_eq!(manifest.chapter, 1);
        assert_eq!(manifest.acts.len(), 2);
        assert_eq!(manifest.acts[0].paragraphs[0].segment_id, "ch01_a01_p01");
        assert_eq!(manifest.acts[1].paragraphs.len(), 2);
        assert_eq!(manifest.acts[1].paragraphs[1].segment_id, "ch01_a02_p02");
    }

    #[test]
    fn repetition_report_v1_roundtrip_json() {
        let doc = "Alpha block here. Beta block here.";
        let windows = vec![
            crate::centroid::WindowData {
                window_id: "w1".into(),
                window_index: 0,
                page: 1,
                cluster_id: 1,
                embedding: vec![1.0, 0.0],
                text: "Alpha block here.".into(),
                doc_char_start: 0,
                doc_char_end: 17,
            },
            crate::centroid::WindowData {
                window_id: "w2".into(),
                window_index: 1,
                page: 1,
                cluster_id: 1,
                embedding: vec![0.9, 0.1],
                text: "Beta block here.".into(),
                doc_char_start: 18,
                doc_char_end: 35,
            },
        ];
        let report = build_repetition_report("job-1", doc, &windows, false);
        let manifest = build_scope_manifest(1, doc, 0);
        let v1 = repetition_report_to_v1(&report, &manifest, 0, doc);
        let json = serde_json::to_string(&v1).unwrap();
        let parsed: RepetitionReportV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.clusters.len(), 1);
        assert!(!parsed.clusters[0].spans.is_empty());
        assert_eq!(parsed.clusters[0].spans[0].location.segment_id, "ch01_a01_p01");
    }

    #[test]
    fn merge_pass_reports_unions_non_overlapping_clusters() {
        let make_cluster = |id: i32, start: u32, end: u32| ClusterSummaryV1 {
            cluster_id: id,
            representative_text: "x".into(),
            instance_count: 2,
            total_word_estimate: 2,
            canonical: EditSpanV1 {
                location: SpanLocation {
                    chapter: 1,
                    act: 1,
                    paragraph_index: 1,
                    segment_id: "ch01_a01_p01".into(),
                    sentence_index: 1,
                    scope_char_start: start,
                    scope_char_end: end,
                    doc_char_start: start,
                    doc_char_end: end,
                },
                cluster_id: id,
                instance_id: 1,
                text: "first".into(),
                similarity_to_centroid: 1.0,
                member_window_count: 1,
            },
            duplicates: vec![EditSpanV1 {
                location: SpanLocation {
                    chapter: 1,
                    act: 1,
                    paragraph_index: 1,
                    segment_id: "ch01_a01_p01".into(),
                    sentence_index: 2,
                    scope_char_start: start + 100,
                    scope_char_end: end + 100,
                    doc_char_start: start + 100,
                    doc_char_end: end + 100,
                },
                cluster_id: id,
                instance_id: 2,
                text: "second".into(),
                similarity_to_centroid: 0.9,
                member_window_count: 1,
            }],
            spans: vec![],
            suggested_op: SuggestedOp::Rewrite,
            cross_act: false,
            needs_bridge: false,
        };

        let pass_a = AnalysisPassRecord {
            pass_id: "p1".into(),
            pass_label: "act".into(),
            scope: AnalysisScope {
                chapter: 1,
                act: Some(1),
                document_path: None,
                document_hash: None,
                scope_char_start: 0,
                scope_char_end: 500,
                doc_char_start: 0,
                doc_char_end: 500,
            },
            window_size: 50,
            stride: 10,
            tokens_per_page: None,
            repetition_report: RepetitionReportV1 {
                job_id: "job".into(),
                clusters: vec![make_cluster(1, 0, 50)],
                stats: AnalysisStats {
                    cluster_count: 1,
                    total_duplicate_instances: 1,
                    total_duplicate_words_estimate: 1,
                },
            },
        };

        let pass_b = AnalysisPassRecord {
            pass_id: "p2".into(),
            pass_label: "chapter".into(),
            scope: pass_a.scope.clone(),
            window_size: 200,
            stride: 50,
            tokens_per_page: Some(400),
            repetition_report: RepetitionReportV1 {
                job_id: "job".into(),
                clusters: vec![make_cluster(2, 300, 400)],
                stats: AnalysisStats {
                    cluster_count: 1,
                    total_duplicate_instances: 1,
                    total_duplicate_words_estimate: 1,
                },
            },
        };

        let merged = merge_pass_reports(&[pass_a, pass_b]);
        assert_eq!(merged.clusters.len(), 2);
    }

    #[test]
    fn fixture_example_validates() {
        let fixture = include_str!("../fixtures/analysis_output_v1.example.json");
        let output = from_export_json(fixture).expect("fixture must parse and validate");
        assert_eq!(output.schema_version, SCHEMA_VERSION);
        assert!(!output.passes.is_empty());
    }
}
