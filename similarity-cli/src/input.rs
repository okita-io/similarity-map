use std::path::Path;

use serde::Deserialize;
use similarity_core::analysis::AnalysisParams;
use similarity_core::report::ScopeManifest;
use similarity_core::AnalysisInput;

/// JSON envelope accepted on stdin or via `--input-file`.
#[derive(Debug, Deserialize)]
pub struct JsonAnalysisRequest {
    pub text: String,
    pub scope_manifest: ScopeManifest,
    pub params: JsonAnalysisParams,
}

#[derive(Debug, Deserialize)]
pub struct JsonAnalysisParams {
    pub window_size: u32,
    pub stride: u32,
    #[serde(default)]
    pub tokens_per_page: Option<u32>,
    #[serde(default)]
    pub chapter_break_regex: Option<String>,
    #[serde(default = "default_min_repetitions")]
    pub min_repetitions: u32,
    #[serde(default = "default_min_samples")]
    pub min_samples: u32,
    #[serde(default = "default_true")]
    pub enable_hdbscan: bool,
    #[serde(default)]
    pub link_subphrases: bool,
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

impl JsonAnalysisRequest {
    pub fn from_reader(mut reader: impl std::io::Read) -> Result<Self, String> {
        serde_json::from_reader(&mut reader).map_err(|e| format!("invalid JSON input: {e}"))
    }

    pub fn from_path(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path)
            .map_err(|e| format!("failed to open input file {}: {e}", path.display()))?;
        Self::from_reader(file)
    }

    pub fn into_analysis_input(self) -> AnalysisInput {
        AnalysisInput {
            text: self.text,
            scope_manifest: self.scope_manifest,
            params: self.params.into(),
        }
    }
}
