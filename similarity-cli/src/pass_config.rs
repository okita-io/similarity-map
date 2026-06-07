use serde::Deserialize;
use similarity_core::AnalysisParams;

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
    pub passes: Vec<PassEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PassEntry {
    pub name: String,
    pub scope: PassScope,
    pub window_size: u32,
    pub stride: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PassScope {
    Act,
    Chapter,
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

impl PassConfigFile {
    pub fn from_yaml_path(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read pass config {}: {e}", path.display()))?;
        serde_yaml::from_str(&raw).map_err(|e| format!("invalid pass config YAML: {e}"))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.passes.is_empty() {
            return Err("pass config must contain at least one pass".into());
        }
        if !(2..=20).contains(&self.min_repetitions) {
            return Err(format!(
                "min_repetitions must be in [2, 20], got {}",
                self.min_repetitions
            ));
        }
        if !(1..=10).contains(&self.min_samples) {
            return Err(format!(
                "min_samples must be in [1, 10], got {}",
                self.min_samples
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for (i, pass) in self.passes.iter().enumerate() {
            if pass.name.trim().is_empty() {
                return Err(format!("passes[{i}].name must be non-empty"));
            }
            if !(5..=1500).contains(&pass.window_size) {
                return Err(format!(
                    "passes[{i}].window_size must be in [5, 1500], got {}",
                    pass.window_size
                ));
            }
            if !(1..=200).contains(&pass.stride) {
                return Err(format!(
                    "passes[{i}].stride must be in [1, 200], got {}",
                    pass.stride
                ));
            }
            if pass.stride > pass.window_size {
                return Err(format!(
                    "passes[{i}].stride ({}) must be <= window_size ({})",
                    pass.stride, pass.window_size
                ));
            }
            if !seen.insert(pass.name.clone()) {
                return Err(format!("duplicate pass name {:?}", pass.name));
            }
        }
        Ok(())
    }

    pub fn to_analysis_params(&self, pass: &PassEntry) -> AnalysisParams {
        AnalysisParams {
            window_size: pass.window_size,
            stride: pass.stride,
            tokens_per_page: None,
            chapter_break_regex: None,
            min_repetitions: self.min_repetitions,
            min_samples: self.min_samples,
            enable_hdbscan: self.enable_hdbscan,
            link_subphrases: self.link_subphrases,
        }
    }
}
