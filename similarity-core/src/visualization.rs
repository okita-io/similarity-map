//! JSON payload consumed by the visual app and Romance Factory tooling.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::analysis::AnalysisParams;
use crate::centroid::WindowData;
use crate::contract::AnalysisOutput;
use crate::importer::{import_document, ImportDocumentParams};
use crate::job_data::{load_job_render_data, parse_window_data_from_batches};
use crate::rasterizer::{encode_canvas_base64, rasterize_page};
use crate::report::ScopeManifest;
use crate::report::{
    build_repetition_report_with_manifest, pages_to_document_text, RepetitionReport, SpanLocation,
};
use crate::storage::Storage;
use crate::types::{AppError, ClusterRegistry, Page, PageSubGrid, SessionError};

pub const DEFAULT_TOLERANCE: f32 = 0.75;
pub const DEFAULT_GAMMA: f32 = 1.5;

/// How a highlighted span should be treated in the editorial workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HighlightRole {
    Canonical,
    Duplicate,
}

/// A document span for inline text highlighting in the visual app.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextHighlight {
    pub cluster_id: i32,
    pub instance_id: u32,
    pub role: HighlightRole,
    pub doc_char_start: u32,
    pub doc_char_end: u32,
    pub page: u32,
    pub hue: f32,
    pub similarity_to_centroid: f32,
    pub text: String,
    /// Resolved chapter/act/paragraph location when a scope manifest was used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SpanLocation>,
}

/// Rasterized page preview at a specific tolerance/gamma.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PageRaster {
    pub page: u32,
    pub canvas_rgba_b64: String,
}

/// Echo of the analysis settings used to produce this payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisSummary {
    pub window_size: u32,
    pub stride: u32,
    pub tokens_per_page: Option<u32>,
    pub min_repetitions: u32,
    pub min_samples: u32,
    pub enable_hdbscan: bool,
    pub link_subphrases: bool,
    pub page_count: u32,
    pub window_count: u32,
    pub tolerance: f32,
    pub gamma: f32,
}

/// Complete analysis output for grid rendering, text highlighting, and RF export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualizationPayload {
    pub job_id: String,
    pub document_text: String,
    pub pages: Vec<Page>,
    pub cluster_registry: ClusterRegistry,
    pub page_sub_grids: Vec<PageSubGrid>,
    pub repetition_report: RepetitionReport,
    pub highlights: Vec<TextHighlight>,
    pub page_rasters: Vec<PageRaster>,
    pub analysis: AnalysisSummary,
    /// Act/paragraph index when analysis used an RF scope manifest (one grid page per act).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_manifest: Option<ScopeManifest>,
    /// Pipeline-consumable v1 output (multi-pass merge) when loaded from RF chapter analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_output: Option<AnalysisOutput>,
}

impl AnalysisSummary {
    pub fn from_params(params: &AnalysisParams, page_count: u32, window_count: u32) -> Self {
        Self {
            window_size: params.window_size,
            stride: params.stride,
            tokens_per_page: params.tokens_per_page,
            min_repetitions: params.min_repetitions,
            min_samples: params.min_samples,
            enable_hdbscan: params.enable_hdbscan,
            link_subphrases: params.link_subphrases,
            page_count,
            window_count,
            tolerance: DEFAULT_TOLERANCE,
            gamma: DEFAULT_GAMMA,
        }
    }
}

/// Map a document character offset to a 1-based page number.
pub fn doc_char_to_page(pages: &[Page], doc_char: u32) -> u32 {
    for page in pages.iter().rev() {
        if doc_char >= page.char_offset_in_doc {
            return page.page_num;
        }
    }
    pages.first().map(|p| p.page_num).unwrap_or(1)
}

/// Build text highlights from an editorial repetition report.
pub fn build_text_highlights(
    pages: &[Page],
    report: &RepetitionReport,
    registry: &ClusterRegistry,
) -> Vec<TextHighlight> {
    let mut highlights = Vec::new();

    for cluster in &report.clusters {
        let hue = registry
            .clusters
            .get(&cluster.cluster_id)
            .map(|info| info.hue)
            .unwrap_or(0.0);

        highlights.push(TextHighlight {
            cluster_id: cluster.canonical.cluster_id,
            instance_id: cluster.canonical.instance_id,
            role: HighlightRole::Canonical,
            doc_char_start: cluster.canonical.doc_char_start,
            doc_char_end: cluster.canonical.doc_char_end,
            page: doc_char_to_page(pages, cluster.canonical.doc_char_start),
            hue,
            similarity_to_centroid: cluster.canonical.similarity_to_centroid,
            text: cluster.canonical.text.clone(),
            location: cluster.canonical.location.clone(),
        });

        for duplicate in &cluster.duplicates {
            highlights.push(TextHighlight {
                cluster_id: duplicate.cluster_id,
                instance_id: duplicate.instance_id,
                role: HighlightRole::Duplicate,
                doc_char_start: duplicate.doc_char_start,
                doc_char_end: duplicate.doc_char_end,
                page: doc_char_to_page(pages, duplicate.doc_char_start),
                hue,
                similarity_to_centroid: duplicate.similarity_to_centroid,
                text: duplicate.text.clone(),
                location: duplicate.location.clone(),
            });
        }
    }

    highlights.sort_by_key(|h| (h.doc_char_start, h.cluster_id, h.instance_id));
    highlights
}

/// Assemble a visualization payload from in-memory analysis artifacts.
pub fn build_visualization_payload(
    job_id: &str,
    pages: &[Page],
    window_data: &[WindowData],
    cluster_registry: &ClusterRegistry,
    page_sub_grids: &[PageSubGrid],
    params: &AnalysisParams,
    tolerance: f32,
    gamma: f32,
    expand_to_sentences: bool,
    scope_manifest: Option<ScopeManifest>,
    analysis_output: Option<AnalysisOutput>,
) -> VisualizationPayload {
    let document_text = pages_to_document_text(pages);
    let repetition_report = build_repetition_report_with_manifest(
        job_id,
        &document_text,
        window_data,
        expand_to_sentences,
        scope_manifest.as_ref(),
    );
    let highlights = build_text_highlights(pages, &repetition_report, cluster_registry);

    let hidden = HashSet::new();
    let page_rasters = page_sub_grids
        .iter()
        .map(|grid| {
            let canvas = rasterize_page(grid, gamma, tolerance, &hidden);
            PageRaster {
                page: grid.page,
                canvas_rgba_b64: encode_canvas_base64(&canvas),
            }
        })
        .collect();

    let mut analysis =
        AnalysisSummary::from_params(params, pages.len() as u32, window_data.len() as u32);
    analysis.tolerance = tolerance;
    analysis.gamma = gamma;

    VisualizationPayload {
        job_id: job_id.to_string(),
        document_text,
        pages: pages.to_vec(),
        cluster_registry: cluster_registry.clone(),
        page_sub_grids: page_sub_grids.to_vec(),
        repetition_report,
        highlights,
        page_rasters,
        analysis,
        scope_manifest,
        analysis_output,
    }
}

fn map_storage_error(err: crate::storage::StorageError) -> AppError {
    AppError::Storage(crate::types::StorageError {
        message: err.to_string(),
    })
}

/// Load a completed job from storage and build the full visualization payload.
pub async fn load_visualization_payload(
    store: &Storage,
    job_id: &str,
    tolerance: f32,
    gamma: f32,
    expand_to_sentences: bool,
) -> Result<VisualizationPayload, AppError> {
    load_visualization_payload_with_analysis_output(
        store,
        job_id,
        tolerance,
        gamma,
        expand_to_sentences,
        None,
    )
    .await
}

/// Like [`load_visualization_payload`], but attaches a preloaded `AnalysisOutput`
/// (e.g. lexical sidecar from the desktop sessions directory).
pub async fn load_visualization_payload_with_analysis_output(
    store: &Storage,
    job_id: &str,
    tolerance: f32,
    gamma: f32,
    expand_to_sentences: bool,
    analysis_output: Option<AnalysisOutput>,
) -> Result<VisualizationPayload, AppError> {
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

    let render_data = load_job_render_data(store, job_id)
        .await
        .map_err(map_storage_error)?;

    let window_batches = store
        .get_windows_for_job(job_id)
        .await
        .map_err(map_storage_error)?;
    let window_data = parse_window_data_from_batches(&window_batches);
    let cluster_registry = crate::centroid::build_cluster_registry(&window_data);

    let params = AnalysisParams {
        window_size: job.window_size,
        stride: job.stride,
        tokens_per_page: job.tokens_per_page,
        chapter_break_regex: job.chapter_break_re.clone(),
        min_repetitions: job.min_repetitions,
        min_samples: job.min_samples,
        enable_hdbscan: true,
        link_subphrases: false,
    };

    let mut payload = build_visualization_payload(
        job_id,
        &pages,
        &window_data,
        &cluster_registry,
        &render_data.page_sub_grids,
        &params,
        tolerance,
        gamma,
        expand_to_sentences,
        analysis_output.as_ref().map(|o| o.scope_manifest.clone()),
        analysis_output,
    );

    // Prefer lexical/merged contract report for highlights when sidecar is present.
    if let Some(ref output) = payload.analysis_output {
        let lexical_highlights =
            highlights_from_analysis_output(&payload.pages, output, &payload.cluster_registry);
        if !lexical_highlights.is_empty() {
            payload.highlights.extend(lexical_highlights);
            payload
                .highlights
                .sort_by_key(|h| (h.doc_char_start, h.cluster_id, h.instance_id));
        }
    }

    Ok(payload)
}

fn highlights_from_analysis_output(
    pages: &[Page],
    output: &AnalysisOutput,
    cluster_registry: &ClusterRegistry,
) -> Vec<TextHighlight> {
    let mut highlights = Vec::new();
    // Use a synthetic hue offset so lexical clusters remain visible even when
    // embedding registry IDs don't overlap.
    let hue_for = |cluster_id: i32| -> f32 {
        if let Some(info) = cluster_registry.clusters.get(&cluster_id) {
            info.hue
        } else {
            let bucket = (cluster_id.rem_euclid(12)) as f32;
            bucket * 30.0
        }
    };

    for cluster in &output.merged_repetition_report.clusters {
        let hue = hue_for(cluster.cluster_id);
        for (idx, span) in cluster.spans.iter().enumerate() {
            let role = if idx == 0 {
                HighlightRole::Canonical
            } else {
                HighlightRole::Duplicate
            };
            highlights.push(TextHighlight {
                cluster_id: cluster.cluster_id,
                instance_id: span.instance_id,
                role,
                doc_char_start: span.location.doc_char_start,
                doc_char_end: span.location.doc_char_end,
                page: doc_char_to_page(pages, span.location.doc_char_start),
                hue,
                similarity_to_centroid: span.similarity_to_centroid,
                text: span.text.clone(),
                location: Some(span.location.clone()),
            });
        }
    }
    highlights
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::centroid::WindowData;
    use crate::report::build_repetition_report;

    #[test]
    fn doc_char_to_page_maps_offsets() {
        let pages = vec![
            Page {
                page_num: 1,
                text: "abc".to_string(),
                char_offset_in_doc: 0,
                char_count: 3,
                token_count: 1,
                pagination_mode: crate::types::PaginationMode::Token,
            },
            Page {
                page_num: 2,
                text: "def".to_string(),
                char_offset_in_doc: 3,
                char_count: 3,
                token_count: 1,
                pagination_mode: crate::types::PaginationMode::Token,
            },
        ];
        assert_eq!(doc_char_to_page(&pages, 0), 1);
        assert_eq!(doc_char_to_page(&pages, 4), 2);
    }

    #[test]
    fn highlights_mark_canonical_and_duplicates() {
        let pages = vec![Page {
            page_num: 1,
            text: "Alpha. Beta.".to_string(),
            char_offset_in_doc: 0,
            char_count: 12,
            token_count: 2,
            pagination_mode: crate::types::PaginationMode::Token,
        }];
        let doc = "Alpha. Beta.";
        let windows = vec![
            WindowData {
                window_id: "w1".into(),
                window_index: 0,
                page: 1,
                cluster_id: 1,
                embedding: vec![1.0, 0.0],
                text: "Alpha.".into(),
                doc_char_start: 0,
                doc_char_end: 6,
            },
            WindowData {
                window_id: "w2".into(),
                window_index: 1,
                page: 1,
                cluster_id: 1,
                embedding: vec![0.9, 0.1],
                text: "Beta.".into(),
                doc_char_start: 7,
                doc_char_end: 12,
            },
        ];
        let registry = crate::centroid::build_cluster_registry(&windows);
        let report = build_repetition_report("job", doc, &windows, false);
        let highlights = build_text_highlights(&pages, &report, &registry);
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].role, HighlightRole::Canonical);
        assert_eq!(highlights[1].role, HighlightRole::Duplicate);
    }

    #[test]
    fn highlights_include_span_location_when_manifest_present() {
        use crate::contract::build_scope_manifest;
        use crate::report::build_repetition_report_with_manifest;

        let text = "Alpha. Beta.\n\nGamma.";
        let pages = vec![Page {
            page_num: 1,
            text: text.to_string(),
            char_offset_in_doc: 0,
            char_count: text.len() as u32,
            token_count: 3,
            pagination_mode: crate::types::PaginationMode::Token,
        }];
        let manifest = build_scope_manifest(1, text, 0);
        let windows = vec![
            WindowData {
                window_id: "w1".into(),
                window_index: 0,
                page: 1,
                cluster_id: 1,
                embedding: vec![1.0, 0.0],
                text: "Alpha.".into(),
                doc_char_start: 0,
                doc_char_end: 6,
            },
            WindowData {
                window_id: "w2".into(),
                window_index: 1,
                page: 1,
                cluster_id: 1,
                embedding: vec![0.9, 0.1],
                text: "Beta.".into(),
                doc_char_start: 7,
                doc_char_end: 12,
            },
        ];
        let registry = crate::centroid::build_cluster_registry(&windows);
        let report =
            build_repetition_report_with_manifest("job", text, &windows, false, Some(&manifest));
        let highlights = build_text_highlights(&pages, &report, &registry);
        assert!(highlights.iter().all(|h| h.location.is_some()));
        assert!(highlights[0]
            .location
            .as_ref()
            .unwrap()
            .segment_id
            .starts_with("ch01_"));
    }
}
