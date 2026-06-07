use serde::Deserialize;
use similarity_core::{AnalysisParams, MultiPassConfig, PassScope, PassSpec};

/// Multi-pass bundle mirroring Romance Factory `settings.yaml` `generate:similarity_map:`.
#[derive(Debug, Clone, Deserialize)]
pub struct PassConfigFile {
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
    pub passes: Vec<PassEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PassEntry {
    pub name: String,
    pub scope: PassScope,
    pub window_size: u32,
    pub stride: u32,
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

impl From<PassConfigFile> for MultiPassConfig {
    fn from(file: PassConfigFile) -> Self {
        MultiPassConfig {
            min_repetitions: file.min_repetitions,
            min_samples: file.min_samples,
            enable_hdbscan: file.enable_hdbscan,
            link_subphrases: file.link_subphrases,
            expand_to_sentences: file.expand_to_sentences,
            tokens_per_page: file.tokens_per_page,
            passes: file
                .passes
                .into_iter()
                .map(|p| PassSpec {
                    name: p.name,
                    scope: p.scope,
                    window_size: p.window_size,
                    stride: p.stride,
                })
                .collect(),
        }
    }
}

impl PassConfigFile {
    pub fn from_yaml_path(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read pass config {}: {e}", path.display()))?;
        serde_yaml::from_str(&raw).map_err(|e| format!("invalid pass config YAML: {e}"))
    }

    pub fn validate(&self) -> Result<(), String> {
        let config: MultiPassConfig = self.clone().into();
        config.validate().map_err(|e| e.to_string())
    }

    pub fn to_analysis_params(&self, pass: &PassEntry) -> AnalysisParams {
        let config: MultiPassConfig = self.clone().into();
        config.to_analysis_params(&PassSpec {
            name: pass.name.clone(),
            scope: pass.scope,
            window_size: pass.window_size,
            stride: pass.stride,
        })
    }
}
