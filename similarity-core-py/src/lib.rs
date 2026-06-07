//! PyO3 extension exposing `analyze_prose` and `analyze_prose_multi_pass` to Python.

use std::path::{Path, PathBuf};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyAny;
use serde::Deserialize;
use similarity_core::analysis::AnalysisParams;
use similarity_core::contract::{to_export_json, validate_analysis_output};
use similarity_core::embedding::EmbeddingEngine;
use similarity_core::model;
use similarity_core::report::AnalysisScope;
use similarity_core::report::ScopeManifest;
use similarity_core::{
    analyze_prose as core_analyze_prose,
    analyze_prose_multi_pass as core_analyze_prose_multi_pass, AnalysisInput,
    AnalyzeProseOptions, DeterministicTestEmbedder, MultiPassConfig, MultiPassInput,
};

/// JSON params for single-pass analysis (mirrors similarity-cli stdin envelope).
#[derive(Debug, Deserialize)]
struct JsonAnalysisParams {
    window_size: u32,
    stride: u32,
    #[serde(default)]
    tokens_per_page: Option<u32>,
    #[serde(default)]
    chapter_break_regex: Option<String>,
    #[serde(default = "default_min_repetitions")]
    min_repetitions: u32,
    #[serde(default = "default_min_samples")]
    min_samples: u32,
    #[serde(default = "default_true")]
    enable_hdbscan: bool,
    #[serde(default)]
    link_subphrases: bool,
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

impl From<JsonAnalysisParams> for AnalysisParams {
    fn from(p: JsonAnalysisParams) -> Self {
        Self {
            window_size: p.window_size,
            stride: p.stride,
            tokens_per_page: p.tokens_per_page,
            chapter_break_regex: p.chapter_break_regex,
            min_repetitions: p.min_repetitions,
            min_samples: p.min_samples,
            enable_hdbscan: p.enable_hdbscan,
            link_subphrases: p.link_subphrases,
        }
    }
}

fn py_dict_to_json(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = obj.py();
    let json = py.import_bound("json")?;
    let dumped = json.call_method1("dumps", (obj,))?;
    dumped.extract()
}

fn py_dict_to_type<T: for<'de> Deserialize<'de>>(obj: &Bound<'_, PyAny>) -> PyResult<T> {
    let json = py_dict_to_json(obj)?;
    serde_json::from_str(&json).map_err(|e| PyRuntimeError::new_err(format!("invalid input: {e}")))
}

fn analysis_output_to_py_dict(py: Python<'_>, output: &similarity_core::AnalysisOutput) -> PyResult<PyObject> {
    validate_analysis_output(output).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    let json = to_export_json(output).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    let json_mod = py.import_bound("json")?;
    let value = json_mod.call_method1("loads", (json,))?;
    Ok(value.into())
}

fn app_error_to_py(err: similarity_core::types::AppError) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

/// Resolve ONNX model path from env or default app-data layout.
///
/// Precedence: `SIMILARITY_MAP_MODEL_PATH` → `SIMILARITY_MAP_MODEL_DIR` →
/// `SIMILARITY_MAP_DATA_DIR` → platform app-data dir.
fn resolve_model_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("SIMILARITY_MAP_MODEL_PATH") {
        return Ok(PathBuf::from(path));
    }

    if let Ok(dir) = std::env::var("SIMILARITY_MAP_MODEL_DIR") {
        let dir_path = PathBuf::from(&dir);
        let direct = dir_path.join("all-MiniLM-L6-v2.onnx");
        if direct.is_file() {
            return Ok(direct);
        }
        let nested = model::model_path(&dir_path);
        if nested.is_file() {
            return Ok(nested);
        }
        return Err(format!(
            "ONNX model not found under SIMILARITY_MAP_MODEL_DIR={dir} \
             (expected all-MiniLM-L6-v2.onnx or models/all-MiniLM-L6-v2.onnx)"
        ));
    }

    let data_dir = std::env::var("SIMILARITY_MAP_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs_data_home)
        .ok_or_else(|| {
            "embedding model not found; set SIMILARITY_MAP_MODEL_PATH, \
             SIMILARITY_MAP_MODEL_DIR, or SIMILARITY_MAP_DATA_DIR"
                .to_string()
        })?;

    let path = model::model_path(&data_dir);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "ONNX model not found at {}; download via Similarity Map app or set \
             SIMILARITY_MAP_MODEL_PATH",
            path.display()
        ))
    }
}

fn dirs_data_home() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| Path::new(&h).join("Library/Application Support"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share"))
            })
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share"))
    }
}

fn chapter_scope_from_manifest(text: &str, manifest: &ScopeManifest) -> AnalysisScope {
    let text_len = text.len() as u32;
    AnalysisScope {
        chapter: manifest.chapter,
        act: None,
        document_path: None,
        document_hash: None,
        scope_char_start: 0,
        scope_char_end: text_len,
        doc_char_start: 0,
        doc_char_end: text_len,
    }
}

fn build_single_pass_options(
    scope: &AnalysisScope,
    params: &AnalysisParams,
    expand_to_sentences: bool,
) -> AnalyzeProseOptions {
    AnalyzeProseOptions {
        scope: scope.clone(),
        job_id: None,
        pass_id: format!("chapter-window-{}-{}", params.window_size, params.stride),
        pass_label: format!(
            "Chapter-scoped phrase pass ({}/{})",
            params.window_size, params.stride
        ),
        include_visualization: false,
        tolerance: similarity_core::DEFAULT_TOLERANCE,
        gamma: similarity_core::DEFAULT_GAMMA,
        expand_to_sentences,
    }
}

/// Run single-pass headless analysis; returns AnalysisOutput v1 as a dict.
#[pyfunction]
#[pyo3(signature = (text, scope_manifest, params, *, expand_to_sentences=true, test_embedder=false))]
fn analyze_prose(
    py: Python<'_>,
    text: String,
    scope_manifest: &Bound<'_, PyAny>,
    params: &Bound<'_, PyAny>,
    expand_to_sentences: bool,
    test_embedder: bool,
) -> PyResult<PyObject> {
    let manifest: ScopeManifest = py_dict_to_type(scope_manifest)?;
    let json_params: JsonAnalysisParams = py_dict_to_type(params)?;
    let analysis_params: AnalysisParams = json_params.into();

    let scope = chapter_scope_from_manifest(&text, &manifest);
    let options = build_single_pass_options(&scope, &analysis_params, expand_to_sentences);
    let input = AnalysisInput {
        text,
        scope_manifest: manifest,
        params: analysis_params,
    };

    let output = if test_embedder {
        let mut embedder = DeterministicTestEmbedder::new(384);
        core_analyze_prose(&input, &options, &mut embedder)
            .map_err(app_error_to_py)?
            .output
    } else {
        let model_path = resolve_model_path().map_err(PyRuntimeError::new_err)?;
        let mut engine = EmbeddingEngine::new(&model_path).map_err(app_error_to_py)?;
        core_analyze_prose(&input, &options, &mut engine)
            .map_err(app_error_to_py)?
            .output
    };

    analysis_output_to_py_dict(py, &output)
}

/// Run multi-pass analysis (act + chapter bundles); returns AnalysisOutput v1 as a dict.
#[pyfunction]
#[pyo3(signature = (text, scope_manifest, pass_config, *, test_embedder=false))]
fn analyze_prose_multi_pass(
    py: Python<'_>,
    text: String,
    scope_manifest: &Bound<'_, PyAny>,
    pass_config: &Bound<'_, PyAny>,
    test_embedder: bool,
) -> PyResult<PyObject> {
    let manifest: ScopeManifest = py_dict_to_type(scope_manifest)?;
    let config: MultiPassConfig = py_dict_to_type(pass_config)?;
    config.validate().map_err(app_error_to_py)?;

    let chapter_scope = chapter_scope_from_manifest(&text, &manifest);
    let job_id = format!("py-ch{}", manifest.chapter);
    let input = MultiPassInput {
        text,
        scope_manifest: manifest,
        config,
        chapter_scope,
        job_id,
    };

    let output = if test_embedder {
        let mut embedder = DeterministicTestEmbedder::new(384);
        core_analyze_prose_multi_pass(&input, &mut embedder)
            .map_err(app_error_to_py)?
            .output
    } else {
        let model_path = resolve_model_path().map_err(PyRuntimeError::new_err)?;
        let mut engine = EmbeddingEngine::new(&model_path).map_err(app_error_to_py)?;
        core_analyze_prose_multi_pass(&input, &mut engine)
            .map_err(app_error_to_py)?
            .output
    };

    analysis_output_to_py_dict(py, &output)
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(analyze_prose, m)?)?;
    m.add_function(wrap_pyfunction!(analyze_prose_multi_pass, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use similarity_core::build_scope_manifest;

    fn sample_text() -> String {
        let phrase = "alpha beta gamma delta epsilon alpha beta gamma delta epsilon";
        [phrase, phrase, phrase].join("\n\n")
    }

    #[test]
    fn resolve_model_path_errors_without_env() {
        std::env::remove_var("SIMILARITY_MAP_MODEL_PATH");
        std::env::remove_var("SIMILARITY_MAP_MODEL_DIR");
        std::env::remove_var("SIMILARITY_MAP_DATA_DIR");
        let err = resolve_model_path().expect_err("missing model should error");
        assert!(err.contains("SIMILARITY_MAP_MODEL"));
    }

    #[test]
    fn single_pass_with_test_embedder_logic() {
        let text = sample_text();
        let manifest = build_scope_manifest(1, &text, 0);
        let params = AnalysisParams {
            window_size: 5,
            stride: 5,
            tokens_per_page: None,
            chapter_break_regex: None,
            min_repetitions: 2,
            min_samples: 2,
            enable_hdbscan: false,
            link_subphrases: false,
        };
        let scope = chapter_scope_from_manifest(&text, &manifest);
        let options = build_single_pass_options(&scope, &params, true);
        let input = AnalysisInput {
            text,
            scope_manifest: manifest,
            params,
        };
        let mut embedder = DeterministicTestEmbedder::new(384);
        let result = similarity_core::analyze_prose(&input, &options, &mut embedder).unwrap();
        validate_analysis_output(&result.output).unwrap();
    }
}
