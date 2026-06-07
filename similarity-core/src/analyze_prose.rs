//! Headless prose analysis pipeline — shared by UI, CLI, PyO3, and unit tests.
//!
//! Stages: paginate → window → embed → cluster → report → optional visualization.
//! Runs entirely in memory with no LanceDB / Tauri dependency.

use std::path::Path;

use uuid::Uuid;

use crate::analysis::{
    build_clustering_artifacts, paginate_text, run_clustering, AnalysisParams,
};
use crate::contract::{
    build_analysis_output_with_manifest, repetition_report_to_v1, AnalysisOutput,
    AnalysisPassRecord,
};
use crate::embedding::{embed_windows, l2_normalize, EmbeddingEngine, DEFAULT_BATCH_SIZE};
use crate::report::{
    build_repetition_report_with_manifest, AnalysisScope, RepetitionReport, ScopeManifest,
};
use crate::types::{AppError, ClusterRegistry, ImportError, Page, PageSubGrid, Window};
use crate::visualization::{
    build_visualization_payload, VisualizationPayload, DEFAULT_GAMMA, DEFAULT_TOLERANCE,
};
use crate::windowing::generate_windows;

/// Input for a single headless analysis run.
#[derive(Debug, Clone)]
pub struct AnalysisInput {
    pub text: String,
    pub scope_manifest: ScopeManifest,
    pub params: AnalysisParams,
}

/// Options controlling scope metadata, pass labelling, and optional visualization.
#[derive(Debug, Clone)]
pub struct AnalyzeProseOptions {
    pub scope: AnalysisScope,
    pub job_id: Option<String>,
    pub pass_id: String,
    pub pass_label: String,
    pub include_visualization: bool,
    pub tolerance: f32,
    pub gamma: f32,
    pub expand_to_sentences: bool,
}

impl AnalyzeProseOptions {
    pub fn chapter_pass(chapter: u32, window_size: u32, stride: u32) -> Self {
        Self {
            scope: AnalysisScope {
                chapter,
                act: None,
                document_path: None,
                document_hash: None,
                scope_char_start: 0,
                scope_char_end: 0,
                doc_char_start: 0,
                doc_char_end: 0,
            },
            job_id: None,
            pass_id: format!("chapter-window-{window_size}-{stride}"),
            pass_label: format!("Chapter-scoped phrase pass ({window_size}/{stride})"),
            include_visualization: false,
            tolerance: DEFAULT_TOLERANCE,
            gamma: DEFAULT_GAMMA,
            expand_to_sentences: true,
        }
    }
}

/// In-memory artifacts produced by the analysis stages (before contract assembly).
#[derive(Debug, Clone)]
pub struct AnalysisArtifacts {
    pub pages: Vec<Page>,
    pub windows: Vec<Window>,
    pub window_data: Vec<crate::centroid::WindowData>,
    pub cluster_registry: ClusterRegistry,
    pub page_sub_grids: Vec<PageSubGrid>,
    pub repetition_report: RepetitionReport,
    pub hdbscan_labels: Vec<i32>,
    pub stable_labels: Vec<i32>,
    pub sim_to_centroids: Vec<f32>,
}

/// Full headless result: contract v1 envelope plus optional UI visualization payload.
#[derive(Debug, Clone)]
pub struct AnalyzeProseResult {
    pub output: AnalysisOutput,
    pub visualization: Option<VisualizationPayload>,
    pub artifacts: AnalysisArtifacts,
}

/// Embeds window text — implemented by ONNX runtime and deterministic test doubles.
pub trait TextEmbedder {
    fn embed_all_windows(&mut self, windows: &[Window]) -> Result<Vec<Vec<f32>>, AppError>;
}

impl TextEmbedder for EmbeddingEngine {
    fn embed_all_windows(&mut self, windows: &[Window]) -> Result<Vec<Vec<f32>>, AppError> {
        if windows.is_empty() {
            return Ok(vec![]);
        }

        let mut all_embeddings = vec![Vec::new(); windows.len()];
        let pairs = embed_windows(self, windows, DEFAULT_BATCH_SIZE, |_, _| {});
        for (window_index, embedding) in pairs {
            let idx = window_index as usize;
            if idx < all_embeddings.len() {
                all_embeddings[idx] = embedding;
            }
        }

        if all_embeddings.iter().any(|e| e.is_empty()) {
            return Err(AppError::Embedding(crate::types::EmbeddingError {
                message: "One or more windows failed to embed".into(),
                window_indices: all_embeddings
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| if e.is_empty() { Some(i as u32) } else { None })
                    .collect(),
            }));
        }

        Ok(all_embeddings)
    }
}

/// Deterministic L2-normalized vectors from token hashes — for unit tests without ONNX.
#[derive(Debug, Clone, Default)]
pub struct DeterministicTestEmbedder {
    pub dim: usize,
}

impl DeterministicTestEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(1) }
    }

    fn embed_text(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dim];
        for token in text.split_whitespace() {
            let mut hash: u64 = 5381;
            for byte in token.bytes() {
                hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
            }
            vec[(hash as usize) % self.dim] += 1.0;
        }
        if vec.iter().all(|v| *v == 0.0) {
            vec[0] = 1.0;
        }
        l2_normalize(vec)
    }
}

impl TextEmbedder for DeterministicTestEmbedder {
    fn embed_all_windows(&mut self, windows: &[Window]) -> Result<Vec<Vec<f32>>, AppError> {
        Ok(windows
            .iter()
            .map(|w| self.embed_text(&w.text))
            .collect())
    }
}

/// Run paginate → window → embed → cluster → report entirely in memory.
pub fn run_analysis_stages(
    text: &str,
    params: &AnalysisParams,
    scope_manifest: &ScopeManifest,
    job_id: &str,
    embedder: &mut impl TextEmbedder,
    expand_to_sentences: bool,
) -> Result<AnalysisArtifacts, AppError> {
    crate::validate_analysis_params(params, None)?;

    let pages = paginate_text(text, params, Some(scope_manifest))?;
    run_analysis_stages_from_pages(
        text,
        &pages,
        params,
        scope_manifest,
        job_id,
        embedder,
        expand_to_sentences,
    )
}

/// Like [`run_analysis_stages`] when pages are already imported (e.g. from a file path).
pub fn run_analysis_stages_from_pages(
    document_text: &str,
    pages: &[Page],
    params: &AnalysisParams,
    scope_manifest: &ScopeManifest,
    job_id: &str,
    embedder: &mut impl TextEmbedder,
    expand_to_sentences: bool,
) -> Result<AnalysisArtifacts, AppError> {
    let windows = generate_windows(pages, params.window_size, params.stride);
    if windows.is_empty() {
        return Err(AppError::Import(ImportError {
            message: "Text produced no analyzable windows".into(),
            path: None,
        }));
    }

    let embeddings = embedder.embed_all_windows(&windows)?;
    run_clustering_stages_from_embeddings(
        document_text,
        pages,
        &windows,
        &embeddings,
        params,
        scope_manifest,
        job_id,
        expand_to_sentences,
    )
}

/// Clustering and report stages when embeddings are already computed (e.g. batched ONNX pipeline).
pub fn run_clustering_stages_from_embeddings(
    document_text: &str,
    pages: &[Page],
    windows: &[Window],
    embeddings: &[Vec<f32>],
    params: &AnalysisParams,
    scope_manifest: &ScopeManifest,
    job_id: &str,
    expand_to_sentences: bool,
) -> Result<AnalysisArtifacts, AppError> {
    if windows.is_empty() {
        return Err(AppError::Import(ImportError {
            message: "Text produced no analyzable windows".into(),
            path: None,
        }));
    }
    if embeddings.len() != windows.len() {
        return Err(AppError::Validation(crate::types::ValidationError {
            field: "embeddings".into(),
            message: format!(
                "embedding count {} does not match window count {}",
                embeddings.len(),
                windows.len()
            ),
        }));
    }

    let (hdbscan_labels, stable_labels) = run_clustering(params, embeddings, windows)?;
    let clustering = build_clustering_artifacts(pages, windows, embeddings, &stable_labels);

    let mut sim_to_centroids: Vec<f32> = vec![0.0; windows.len()];
    for (i, wd) in clustering.window_data.iter().enumerate() {
        if wd.cluster_id >= 0 {
            if let Some(info) = clustering.cluster_registry.clusters.get(&wd.cluster_id) {
                sim_to_centroids[i] = cosine_similarity(&wd.embedding, &info.centroid);
            }
        }
    }

    let repetition_report = build_repetition_report_with_manifest(
        job_id,
        document_text,
        &clustering.window_data,
        expand_to_sentences,
        Some(scope_manifest),
    );

    Ok(AnalysisArtifacts {
        pages: pages.to_vec(),
        windows: windows.to_vec(),
        window_data: clustering.window_data,
        cluster_registry: clustering.cluster_registry,
        page_sub_grids: clustering.page_sub_grids,
        repetition_report,
        hdbscan_labels,
        stable_labels,
        sim_to_centroids,
    })
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

/// Single headless entry point: contract v1 [`AnalysisOutput`] plus optional visualization.
pub fn analyze_prose(
    input: &AnalysisInput,
    options: &AnalyzeProseOptions,
    embedder: &mut impl TextEmbedder,
) -> Result<AnalyzeProseResult, AppError> {
    let job_id = options
        .job_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut scope = options.scope.clone();
    if scope.scope_char_end == 0 {
        scope.scope_char_end = input.text.len() as u32;
    }
    if scope.doc_char_end == 0 {
        scope.doc_char_end = scope.doc_char_start + scope.scope_char_end;
    }

    let artifacts = run_analysis_stages(
        &input.text,
        &input.params,
        &input.scope_manifest,
        &job_id,
        embedder,
        options.expand_to_sentences,
    )?;

    let v1_report = repetition_report_to_v1(
        &artifacts.repetition_report,
        &input.scope_manifest,
        scope.doc_char_start,
        &input.text,
    );

    let pass = AnalysisPassRecord {
        pass_id: options.pass_id.clone(),
        pass_label: options.pass_label.clone(),
        scope: scope.clone(),
        window_size: input.params.window_size,
        stride: input.params.stride,
        tokens_per_page: input.params.tokens_per_page,
        repetition_report: v1_report,
    };

    let output = build_analysis_output_with_manifest(
        scope,
        input.scope_manifest.clone(),
        vec![pass],
    );

    let visualization = if options.include_visualization {
        Some(build_visualization_payload(
            &job_id,
            &artifacts.pages,
            &artifacts.window_data,
            &artifacts.cluster_registry,
            &artifacts.page_sub_grids,
            &input.params,
            options.tolerance,
            options.gamma,
            options.expand_to_sentences,
        ))
    } else {
        None
    };

    Ok(AnalyzeProseResult {
        output,
        visualization,
        artifacts,
    })
}

/// Convenience: analyze prose using an ONNX model on disk.
pub fn analyze_prose_with_model(
    input: &AnalysisInput,
    options: &AnalyzeProseOptions,
    model_path: &Path,
) -> Result<AnalyzeProseResult, AppError> {
    let mut engine = EmbeddingEngine::new(model_path)?;
    analyze_prose(input, options, &mut engine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::build_scope_manifest;

    fn repeated_phrase_text() -> String {
        let phrase = "alpha beta gamma delta epsilon alpha beta gamma delta epsilon";
        [phrase, phrase, phrase].join("\n\n")
    }

    fn test_params() -> AnalysisParams {
        AnalysisParams {
            window_size: 5,
            stride: 5,
            tokens_per_page: None,
            chapter_break_regex: None,
            min_repetitions: 2,
            min_samples: 2,
            enable_hdbscan: false,
            link_subphrases: false,
        }
    }

    #[test]
    fn analyze_prose_in_memory_finds_repeated_phrase() {
        let text = repeated_phrase_text();
        let manifest = build_scope_manifest(1, &text, 0);
        let input = AnalysisInput {
            text: text.clone(),
            scope_manifest: manifest,
            params: test_params(),
        };
        let mut options = AnalyzeProseOptions::chapter_pass(1, 5, 5);
        options.scope.doc_char_end = text.len() as u32;
        options.scope.scope_char_end = text.len() as u32;

        let mut embedder = DeterministicTestEmbedder::new(64);
        let result = analyze_prose(&input, &options, &mut embedder).expect("analysis succeeds");

        assert_eq!(result.output.schema_version, crate::report::SCHEMA_VERSION);
        assert_eq!(result.output.passes.len(), 1);
        assert_eq!(
            result.output.scope_manifest.chapter,
            input.scope_manifest.chapter
        );
        assert!(
            !result.output.merged_repetition_report.clusters.is_empty(),
            "expected at least one repetition cluster"
        );
        assert!(result.visualization.is_none());

        let cluster = &result.output.merged_repetition_report.clusters[0];
        assert!(cluster.instance_count >= 2);
        assert!(!cluster.spans.is_empty());
        assert!(!cluster.spans[0].location.segment_id.is_empty());

        crate::validate_analysis_output(&result.output).expect("output validates against contract");
    }

    #[test]
    fn analyze_prose_optional_visualization_payload() {
        let text = repeated_phrase_text();
        let manifest = build_scope_manifest(1, &text, 0);
        let input = AnalysisInput {
            text: text.clone(),
            scope_manifest: manifest,
            params: test_params(),
        };
        let mut options = AnalyzeProseOptions::chapter_pass(1, 5, 5);
        options.scope.doc_char_end = text.len() as u32;
        options.scope.scope_char_end = text.len() as u32;
        options.include_visualization = true;

        let mut embedder = DeterministicTestEmbedder::new(64);
        let result = analyze_prose(&input, &options, &mut embedder).unwrap();
        let viz = result.visualization.expect("visualization requested");
        assert!(!viz.page_rasters.is_empty());
        assert_eq!(viz.job_id, result.output.merged_repetition_report.job_id);
    }

    #[test]
    fn run_analysis_stages_rejects_empty_text() {
        let manifest = ScopeManifest {
            chapter: 1,
            acts: vec![],
        };
        let params = test_params();
        let mut embedder = DeterministicTestEmbedder::new(8);
        let err = run_analysis_stages("", &params, &manifest, "job", &mut embedder, false)
            .expect_err("empty text should fail");
        assert!(matches!(err, AppError::Import(_)));
    }
}
