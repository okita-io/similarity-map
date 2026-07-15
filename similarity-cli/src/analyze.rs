use std::path::Path;

use similarity_core::contract::validate_analysis_output;
use similarity_core::embedding::EmbeddingEngine;
use similarity_core::report::AnalysisScope;
use similarity_core::ScopeManifest;
use similarity_core::{
    analyze_prose, analyze_prose_multi_pass, AnalysisInput, AnalyzeProseOptions,
    AnalyzeProseResult, DeterministicTestEmbedder, MultiPassInput,
};
use similarity_core::{MultiPassConfig, PassScope, PassSpec};

use crate::pass_config::PassConfigFile;
use crate::story::ChapterDraft;

pub struct AnalyzeContext {
    pub text: String,
    pub chapter: u32,
    pub scope_manifest: ScopeManifest,
    pub document_path: Option<String>,
    pub document_hash: Option<String>,
    pub pass_config: Option<PassConfigFile>,
    pub single_params: Option<similarity_core::AnalysisParams>,
    pub expand_to_sentences: bool,
    pub model_path: Option<std::path::PathBuf>,
    pub test_embedder: bool,
}

pub fn run_analyze(ctx: AnalyzeContext) -> Result<String, String> {
    let text_len = ctx.text.len() as u32;
    let scope = AnalysisScope {
        chapter: ctx.chapter,
        act: None,
        document_path: ctx.document_path.clone(),
        document_hash: ctx.document_hash.clone(),
        scope_char_start: 0,
        scope_char_end: text_len,
        doc_char_start: 0,
        doc_char_end: text_len,
    };

    let output = if let Some(ref config) = ctx.pass_config {
        let multi_config: MultiPassConfig = config.clone().into();
        multi_config.validate().map_err(|e| e.to_string())?;
        let input = MultiPassInput {
            text: ctx.text.clone(),
            scope_manifest: ctx.scope_manifest.clone(),
            config: multi_config,
            chapter_scope: scope,
            job_id: format!("cli-ch{}", ctx.chapter),
        };
        run_multi_pass(&input, &ctx)?.output
    } else {
        let params = ctx
            .single_params
            .as_ref()
            .ok_or_else(|| "internal error: single-pass analysis requires params".to_string())?;
        let options = build_options(
            &scope,
            &PassSpec {
                name: format!("chapter-window-{}-{}", params.window_size, params.stride),
                scope: PassScope::Chapter,
                method: similarity_core::PassMethod::Embedding,
                window_size: params.window_size,
                stride: params.stride,
            },
            ctx.expand_to_sentences,
        );
        let input = AnalysisInput {
            text: ctx.text.clone(),
            scope_manifest: ctx.scope_manifest.clone(),
            params: params.clone(),
        };
        run_one_pass(&input, &options, &ctx)?.output
    };

    validate_analysis_output(&output).map_err(|e| e.to_string())?;
    similarity_core::to_export_json(&output).map_err(|e| e.to_string())
}

fn run_multi_pass(
    input: &MultiPassInput,
    ctx: &AnalyzeContext,
) -> Result<similarity_core::MultiPassResult, String> {
    if !input.config.needs_embedder() {
        return analyze_prose_multi_pass::<DeterministicTestEmbedder>(input, None)
            .map_err(|e| e.to_string());
    }
    if ctx.test_embedder {
        let mut embedder = DeterministicTestEmbedder::new(384);
        analyze_prose_multi_pass(input, Some(&mut embedder)).map_err(|e| e.to_string())
    } else {
        let model_path = ctx
            .model_path
            .clone()
            .or_else(default_model_path)
            .ok_or_else(|| {
                "embedding model not found; set --model-path or SIMILARITY_MAP_MODEL_PATH"
                    .to_string()
            })?;
        if !model_path.is_file() {
            return Err(format!("ONNX model not found at {}", model_path.display()));
        }
        let mut engine = EmbeddingEngine::new(&model_path)
            .map_err(|e: similarity_core::types::AppError| e.to_string())?;
        analyze_prose_multi_pass(input, Some(&mut engine)).map_err(|e| e.to_string())
    }
}

fn build_options(
    scope: &AnalysisScope,
    pass: &PassSpec,
    expand_to_sentences: bool,
) -> AnalyzeProseOptions {
    let label = match pass.scope {
        PassScope::Act => format!(
            "Act-scoped phrase pass ({}/{})",
            pass.window_size, pass.stride
        ),
        PassScope::Chapter => format!(
            "Chapter-scoped phrase pass ({}/{})",
            pass.window_size, pass.stride
        ),
    };
    AnalyzeProseOptions {
        scope: scope.clone(),
        job_id: None,
        pass_id: pass.name.clone(),
        pass_label: label,
        include_visualization: false,
        tolerance: similarity_core::DEFAULT_TOLERANCE,
        gamma: similarity_core::DEFAULT_GAMMA,
        expand_to_sentences,
    }
}

fn run_one_pass(
    input: &AnalysisInput,
    options: &AnalyzeProseOptions,
    ctx: &AnalyzeContext,
) -> Result<AnalyzeProseResult, String> {
    if ctx.test_embedder {
        let mut embedder = DeterministicTestEmbedder::new(384);
        analyze_prose(input, options, &mut embedder).map_err(|e| e.to_string())
    } else {
        let model_path = ctx
            .model_path
            .clone()
            .or_else(default_model_path)
            .ok_or_else(|| {
                "embedding model not found; set --model-path or SIMILARITY_MAP_MODEL_PATH"
                    .to_string()
            })?;
        if !model_path.is_file() {
            return Err(format!("ONNX model not found at {}", model_path.display()));
        }
        let mut engine = EmbeddingEngine::new(&model_path)
            .map_err(|e: similarity_core::types::AppError| e.to_string())?;
        analyze_prose(input, options, &mut engine).map_err(|e| e.to_string())
    }
}

fn default_model_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("SIMILARITY_MAP_MODEL_PATH") {
        return Some(std::path::PathBuf::from(path));
    }
    let data_dir = std::env::var("SIMILARITY_MAP_DATA_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| dirs_data_home().map(|home| home.join("similarity-map")))?;
    Some(similarity_core::model::model_path(&data_dir))
}

fn dirs_data_home() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| Path::new(&h).join("Library/Application Support"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(Path::new)
            .map(|p| p.to_path_buf())
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(Path::new)
            .map(|p| p.to_path_buf())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share"))
    }
}

pub fn context_from_story(
    draft: ChapterDraft,
    pass_config: Option<PassConfigFile>,
    single_params: Option<similarity_core::AnalysisParams>,
    expand_to_sentences: bool,
    model_path: Option<std::path::PathBuf>,
    test_embedder: bool,
) -> AnalyzeContext {
    AnalyzeContext {
        text: draft.text,
        chapter: draft.chapter,
        scope_manifest: draft.scope_manifest,
        document_path: Some(draft.source_path.to_string_lossy().into_owned()),
        document_hash: Some(draft.document_hash),
        pass_config,
        single_params,
        expand_to_sentences,
        model_path,
        test_embedder,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similarity_core::analysis::AnalysisParams;
    use similarity_core::build_scope_manifest;

    fn sample_text() -> String {
        let phrase = "alpha beta gamma delta epsilon alpha beta gamma delta epsilon";
        [phrase, phrase, phrase].join("\n\n")
    }

    #[test]
    fn analyze_with_test_embedder_produces_valid_json() {
        let text = sample_text();
        let manifest = build_scope_manifest(1, &text, 0);
        let ctx = AnalyzeContext {
            text,
            chapter: 1,
            scope_manifest: manifest,
            document_path: None,
            document_hash: None,
            pass_config: None,
            single_params: Some(AnalysisParams {
                window_size: 5,
                stride: 5,
                tokens_per_page: None,
                chapter_break_regex: None,
                min_repetitions: 2,
                min_samples: 2,
                enable_hdbscan: false,
                link_subphrases: false,
            }),
            expand_to_sentences: true,
            model_path: None,
            test_embedder: true,
        };
        let json = run_analyze(ctx).expect("analysis succeeds");
        let output: similarity_core::AnalysisOutput =
            serde_json::from_str(&json).expect("stdout is valid JSON");
        validate_analysis_output(&output).expect("valid v1 contract");
    }
}
