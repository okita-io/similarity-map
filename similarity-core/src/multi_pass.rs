//! Multi-pass analysis orchestrator — act + chapter window bundles merged into one report.
//!
//! Act-scoped passes analyze each act independently with fine strides.
//! Chapter-scoped passes analyze the full concatenated chapter with coarse strides
//! and token-based pagination (windows may span act boundaries).

use serde::{Deserialize, Serialize};

use crate::analysis::{paginate_text, AnalysisParams};
use crate::analyze_prose::{run_analysis_stages, run_analysis_stages_from_pages, TextEmbedder};
use crate::contract::{
    build_analysis_output_with_manifest, repetition_report_to_v1, AnalysisOutput,
    AnalysisPassRecord,
};
use crate::report::{AnalysisScope, AnalysisStats, ScopeManifest, ScopeSegment};
use crate::types::AppError;

/// Which manuscript unit a pass targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PassScope {
    Act,
    Chapter,
}

/// One window/stride bundle within a multi-pass run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassSpec {
    pub name: String,
    pub scope: PassScope,
    pub window_size: u32,
    pub stride: u32,
}

/// Shared tuning + ordered pass list (mirrors RF `settings.yaml` `generate:similarity_map:`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiPassConfig {
    #[serde(default = "default_min_repetitions")]
    pub min_repetitions: u32,
    #[serde(default = "default_min_samples")]
    pub min_samples: u32,
    #[serde(default = "default_true")]
    pub enable_hdbscan: bool,
    #[serde(default)]
    pub link_subphrases: bool,
    #[serde(default = "default_true")]
    pub expand_to_sentences: bool,
    #[serde(default = "default_tokens_per_page")]
    pub tokens_per_page: u32,
    pub passes: Vec<PassSpec>,
}

/// Input for [`analyze_prose_multi_pass`].
#[derive(Debug, Clone)]
pub struct MultiPassInput {
    pub text: String,
    pub scope_manifest: ScopeManifest,
    pub config: MultiPassConfig,
    pub chapter_scope: AnalysisScope,
    pub job_id: String,
}

/// Result of a multi-pass run (contract envelope only; no visualization).
#[derive(Debug, Clone)]
pub struct MultiPassResult {
    pub output: AnalysisOutput,
}

fn default_min_repetitions() -> u32 {
    3
}

fn default_min_samples() -> u32 {
    3
}

fn default_true() -> bool {
    true
}

fn default_tokens_per_page() -> u32 {
    400
}

/// Default 4-pass bundle from Romance Factory `settings.yaml` / integration contract.
pub fn default_rf_multi_pass_config() -> MultiPassConfig {
    MultiPassConfig {
        min_repetitions: 3,
        min_samples: 3,
        enable_hdbscan: true,
        link_subphrases: false,
        expand_to_sentences: true,
        tokens_per_page: 400,
        passes: vec![
            PassSpec {
                name: "act_50_10".into(),
                scope: PassScope::Act,
                window_size: 50,
                stride: 10,
            },
            PassSpec {
                name: "act_100_25".into(),
                scope: PassScope::Act,
                window_size: 100,
                stride: 25,
            },
            PassSpec {
                name: "chapter_200_50".into(),
                scope: PassScope::Chapter,
                window_size: 200,
                stride: 50,
            },
            PassSpec {
                name: "chapter_400_100".into(),
                scope: PassScope::Chapter,
                window_size: 400,
                stride: 100,
            },
        ],
    }
}

impl MultiPassConfig {
    pub fn validate(&self) -> Result<(), AppError> {
        crate::validate_analysis_params(
            &self.to_analysis_params(&PassSpec {
                name: "validate".into(),
                scope: PassScope::Chapter,
                window_size: 50,
                stride: 10,
            }),
            None,
        )?;
        if self.passes.is_empty() {
            return Err(AppError::Validation(crate::types::ValidationError {
                field: "passes".into(),
                message: "multi-pass config must contain at least one pass".into(),
            }));
        }
        let mut seen = std::collections::HashSet::new();
        for (i, pass) in self.passes.iter().enumerate() {
            if pass.name.trim().is_empty() {
                return Err(AppError::Validation(crate::types::ValidationError {
                    field: format!("passes[{i}].name"),
                    message: "pass name must be non-empty".into(),
                }));
            }
            if !seen.insert(pass.name.clone()) {
                return Err(AppError::Validation(crate::types::ValidationError {
                    field: format!("passes[{i}].name"),
                    message: format!("duplicate pass name {:?}", pass.name),
                }));
            }
            if pass.window_size < 5 || pass.window_size > 1500 {
                return Err(AppError::Validation(crate::types::ValidationError {
                    field: format!("passes[{i}].window_size"),
                    message: format!("window_size must be in [5, 1500], got {}", pass.window_size),
                }));
            }
            if pass.stride < 1 || pass.stride > 200 || pass.stride > pass.window_size {
                return Err(AppError::Validation(crate::types::ValidationError {
                    field: format!("passes[{i}].stride"),
                    message: format!(
                        "stride must be in [1, 200] and <= window_size, got {}",
                        pass.stride
                    ),
                }));
            }
        }
        Ok(())
    }

    pub fn to_analysis_params(&self, pass: &PassSpec) -> AnalysisParams {
        AnalysisParams {
            window_size: pass.window_size,
            stride: pass.stride,
            tokens_per_page: Some(self.tokens_per_page),
            chapter_break_regex: None,
            min_repetitions: self.min_repetitions,
            min_samples: self.min_samples,
            enable_hdbscan: self.enable_hdbscan,
            link_subphrases: self.link_subphrases,
        }
    }
}

/// Run all configured passes and merge into one [`AnalysisOutput`].
pub fn analyze_prose_multi_pass(
    input: &MultiPassInput,
    embedder: &mut impl TextEmbedder,
) -> Result<MultiPassResult, AppError> {
    input.config.validate()?;

    let mut pass_records = Vec::new();
    for spec in &input.config.passes {
        match spec.scope {
            PassScope::Act => {
                for act in &input.scope_manifest.acts {
                    if let Some(record) = run_act_pass(
                        &input.text,
                        &input.scope_manifest,
                        act,
                        spec,
                        &input.config,
                        &input.job_id,
                        embedder,
                    )? {
                        pass_records.push(record);
                    }
                }
            }
            PassScope::Chapter => {
                pass_records.push(run_chapter_pass(
                    &input.text,
                    &input.scope_manifest,
                    spec,
                    &input.config,
                    &input.chapter_scope,
                    &input.job_id,
                    embedder,
                )?);
            }
        }
    }

    if pass_records.is_empty() {
        return Err(AppError::Import(crate::types::ImportError {
            message: "multi-pass analysis produced no pass records".into(),
            path: None,
        }));
    }

    let mut chapter_scope = input.chapter_scope.clone();
    if chapter_scope.scope_char_end == 0 {
        chapter_scope.scope_char_end = input.text.len() as u32;
    }
    if chapter_scope.doc_char_end == 0 {
        chapter_scope.doc_char_end =
            chapter_scope.doc_char_start + chapter_scope.scope_char_end;
    }

    let output = build_analysis_output_with_manifest(
        chapter_scope,
        input.scope_manifest.clone(),
        pass_records,
    );

    Ok(MultiPassResult { output })
}

fn run_act_pass(
    chapter_text: &str,
    chapter_manifest: &ScopeManifest,
    act: &ScopeSegment,
    spec: &PassSpec,
    config: &MultiPassConfig,
    job_id: &str,
    embedder: &mut impl TextEmbedder,
) -> Result<Option<AnalysisPassRecord>, AppError> {
    let start = act.scope_char_start as usize;
    let end = act.scope_char_end as usize;
    let Some(act_slice) = chapter_text.get(start..end) else {
        return Ok(None);
    };
    let act_text = act_slice.trim();
    if act_text.is_empty() {
        return Ok(None);
    }

    let act_manifest =
        crate::contract::build_scope_manifest(chapter_manifest.chapter, act_text, act.doc_char_start);
    let params = config.to_analysis_params(spec);

    let artifacts = match run_analysis_stages(
        act_text,
        &params,
        &act_manifest,
        job_id,
        embedder,
        config.expand_to_sentences,
    ) {
        Ok(artifacts) => artifacts,
        Err(err) if is_benign_no_repetition_error(&err) => {
            let pass_scope = AnalysisScope {
                chapter: chapter_manifest.chapter,
                act: Some(act.act),
                document_path: None,
                document_hash: None,
                scope_char_start: act.scope_char_start,
                scope_char_end: act.scope_char_end,
                doc_char_start: act.doc_char_start,
                doc_char_end: act.doc_char_end,
            };
            return Ok(Some(AnalysisPassRecord {
                pass_id: format!("{}_a{:02}", spec.name, act.act),
                pass_label: format!(
                    "Act {} phrase pass ({}/{})",
                    act.act, spec.window_size, spec.stride
                ),
                scope: pass_scope,
                window_size: spec.window_size,
                stride: spec.stride,
                tokens_per_page: None,
                repetition_report: empty_pass_report(job_id),
            }));
        }
        Err(err) => return Err(err),
    };

    let v1 = repetition_report_to_v1(
        &artifacts.repetition_report,
        chapter_manifest,
        act.doc_char_start,
        chapter_text,
    );

    let pass_scope = AnalysisScope {
        chapter: chapter_manifest.chapter,
        act: Some(act.act),
        document_path: None,
        document_hash: None,
        scope_char_start: act.scope_char_start,
        scope_char_end: act.scope_char_end,
        doc_char_start: act.doc_char_start,
        doc_char_end: act.doc_char_end,
    };

    Ok(Some(AnalysisPassRecord {
        pass_id: format!("{}_a{:02}", spec.name, act.act),
        pass_label: format!(
            "Act {} phrase pass ({}/{})",
            act.act, spec.window_size, spec.stride
        ),
        scope: pass_scope,
        window_size: spec.window_size,
        stride: spec.stride,
        tokens_per_page: None,
        repetition_report: v1,
    }))
}

fn run_chapter_pass(
    chapter_text: &str,
    chapter_manifest: &ScopeManifest,
    spec: &PassSpec,
    config: &MultiPassConfig,
    chapter_scope: &AnalysisScope,
    job_id: &str,
    embedder: &mut impl TextEmbedder,
) -> Result<AnalysisPassRecord, AppError> {
    let mut params = config.to_analysis_params(spec);
    params.tokens_per_page = Some(config.tokens_per_page);

    // Token pagination — coarse windows may span act boundaries.
    let pages = paginate_text(chapter_text, &params, None)?;
    let artifacts = match run_analysis_stages_from_pages(
        chapter_text,
        &pages,
        &params,
        chapter_manifest,
        job_id,
        embedder,
        config.expand_to_sentences,
    ) {
        Ok(artifacts) => artifacts,
        Err(err) if is_benign_no_repetition_error(&err) => {
            return Ok(AnalysisPassRecord {
                pass_id: spec.name.clone(),
                pass_label: format!(
                    "Chapter-scoped phrase pass ({}/{})",
                    spec.window_size, spec.stride
                ),
                scope: chapter_scope.clone(),
                window_size: spec.window_size,
                stride: spec.stride,
                tokens_per_page: Some(config.tokens_per_page),
                repetition_report: empty_pass_report(job_id),
            });
        }
        Err(err) => return Err(err),
    };

    let v1 = repetition_report_to_v1(
        &artifacts.repetition_report,
        chapter_manifest,
        chapter_scope.doc_char_start,
        chapter_text,
    );

    Ok(AnalysisPassRecord {
        pass_id: spec.name.clone(),
        pass_label: format!(
            "Chapter-scoped phrase pass ({}/{})",
            spec.window_size, spec.stride
        ),
        scope: chapter_scope.clone(),
        window_size: spec.window_size,
        stride: spec.stride,
        tokens_per_page: Some(config.tokens_per_page),
        repetition_report: v1,
    })
}

fn empty_pass_report(job_id: &str) -> crate::contract::RepetitionReportV1 {
    crate::contract::RepetitionReportV1 {
        job_id: job_id.to_string(),
        clusters: vec![],
        stats: AnalysisStats {
            cluster_count: 0,
            total_duplicate_instances: 0,
            total_duplicate_words_estimate: 0,
        },
        boundary_version: crate::report::BOUNDARY_VERSION,
    }
}

fn is_benign_no_repetition_error(err: &AppError) -> bool {
    match err {
        AppError::Clustering(e) => e.message.contains("No clusters found"),
        AppError::Import(e) => e.message.contains("no analyzable windows"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{validate_analysis_output, EditSpanV1, ClusterSummaryV1};
    use crate::analyze_prose::DeterministicTestEmbedder;
    use crate::contract::build_scope_manifest;
    use crate::report::{AnalysisScope, SpanLocation, SuggestedOp, derive_cluster_enrichments_v1};

    fn repeated_chapter_text() -> String {
        let phrase = "alpha beta gamma delta epsilon alpha beta gamma delta epsilon";
        [phrase, phrase, phrase].join("\n\n")
    }

    #[test]
    fn multi_pass_merged_report_dedupes_overlapping_passes() {
        let text = repeated_chapter_text();
        let manifest = build_scope_manifest(1, &text, 0);
        let mut config = default_rf_multi_pass_config();
        config.min_repetitions = 2;
        config.min_samples = 2;
        config.enable_hdbscan = false;
        // Two identical chapter passes should detect the same clusters once merged.
        config.passes = vec![
            PassSpec {
                name: "chapter_a".into(),
                scope: PassScope::Chapter,
                window_size: 5,
                stride: 5,
            },
            PassSpec {
                name: "chapter_b".into(),
                scope: PassScope::Chapter,
                window_size: 5,
                stride: 5,
            },
        ];
        config.tokens_per_page = 400;

        let text_len = text.len() as u32;
        let input = MultiPassInput {
            text: text.clone(),
            scope_manifest: manifest,
            config,
            chapter_scope: AnalysisScope {
                chapter: 1,
                act: None,
                document_path: None,
                document_hash: None,
                scope_char_start: 0,
                scope_char_end: text_len,
                doc_char_start: 0,
                doc_char_end: text_len,
            },
            job_id: "multi-test".into(),
        };

        let mut embedder = DeterministicTestEmbedder::new(64);
        let result = analyze_prose_multi_pass(&input, &mut embedder).expect("multi-pass ok");
        assert_eq!(result.output.passes.len(), 2);
        validate_analysis_output(&result.output).expect("valid envelope");

        let pass_clusters: u32 = result
            .output
            .passes
            .iter()
            .map(|p| p.repetition_report.stats.cluster_count)
            .sum();
        let merged = result.output.merged_repetition_report.stats.cluster_count;
        assert!(
            merged <= pass_clusters,
            "merged ({merged}) should dedupe pass clusters ({pass_clusters})"
        );
        assert!(merged >= 1, "expected at least one merged cluster");
        assert!(
            merged < pass_clusters,
            "identical passes should merge overlapping clusters ({merged} vs {pass_clusters})"
        );
    }

    #[test]
    fn default_rf_config_has_four_passes() {
        let cfg = default_rf_multi_pass_config();
        assert_eq!(cfg.passes.len(), 4);
        assert_eq!(cfg.passes[0].name, "act_50_10");
        assert_eq!(cfg.passes[0].scope, PassScope::Act);
        assert_eq!(cfg.passes[2].name, "chapter_200_50");
        assert_eq!(cfg.passes[2].scope, PassScope::Chapter);
        assert_eq!(cfg.passes[2].window_size, 200);
        assert_eq!(cfg.passes[3].stride, 100);
    }

    #[test]
    fn large_duplicate_span_suggests_replace_paragraph() {
        let cluster = ClusterSummaryV1 {
            cluster_id: 1,
            representative_text: "x".into(),
            instance_count: 2,
            total_word_estimate: 120,
            canonical: EditSpanV1 {
                location: SpanLocation {
                    chapter: 1,
                    act: 1,
                    paragraph_index: 1,
                    segment_id: "ch01_a01_p01".into(),
                    sentence_index: 1,
                    scope_char_start: 0,
                    scope_char_end: 600,
                    doc_char_start: 0,
                    doc_char_end: 600,
                },
                cluster_id: 1,
                instance_id: 1,
                text: "word ".repeat(60),
                similarity_to_centroid: 0.99,
                member_window_count: 1,
            },
            duplicates: vec![EditSpanV1 {
                location: SpanLocation {
                    chapter: 1,
                    act: 1,
                    paragraph_index: 1,
                    segment_id: "ch01_a01_p01".into(),
                    sentence_index: 2,
                    scope_char_start: 700,
                    scope_char_end: 800,
                    doc_char_start: 700,
                    doc_char_end: 800,
                },
                cluster_id: 1,
                instance_id: 2,
                text: "word ".repeat(55),
                similarity_to_centroid: 0.99,
                member_window_count: 1,
            }],
            spans: vec![],
            suggested_op: SuggestedOp::DeleteSpan,
            cross_act: false,
            needs_bridge: false,
        };
        let spans = vec![cluster.canonical.clone(), cluster.duplicates[0].clone()];
        let (cross_act, needs_bridge, suggested_op) = derive_cluster_enrichments_v1(&spans);
        assert!(!cross_act);
        assert!(needs_bridge);
        assert_eq!(suggested_op, SuggestedOp::ReplaceParagraph);
    }

    #[test]
    fn small_blast_radius_suggests_delete_span() {
        let make_span = |start: u32, end: u32, words: usize| EditSpanV1 {
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
            cluster_id: 1,
            instance_id: 1,
            text: "word ".repeat(words),
            similarity_to_centroid: 0.98,
            member_window_count: 1,
        };

        let cluster = ClusterSummaryV1 {
            cluster_id: 1,
            representative_text: "x".into(),
            instance_count: 2,
            total_word_estimate: 10,
            canonical: make_span(0, 50, 5),
            duplicates: vec![make_span(60, 110, 5)],
            spans: vec![make_span(0, 50, 5), make_span(60, 110, 5)],
            suggested_op: SuggestedOp::RewriteSpan,
            cross_act: false,
            needs_bridge: false,
        };

        let (_, _, suggested_op) = derive_cluster_enrichments_v1(&cluster.spans);
        assert_eq!(suggested_op, SuggestedOp::DeleteSpan);
    }
}
