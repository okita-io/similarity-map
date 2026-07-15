use serde::Deserialize;
use similarity_core::{
    AnalysisParams, LexicalPassConfig, MultiPassConfig, PassMethod, PassScope, PassSpec,
};

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
    #[serde(default)]
    pub lexical: Option<LexicalPassConfig>,
    pub passes: Vec<PassEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PassEntry {
    pub name: String,
    pub scope: PassScope,
    /// Missing method defaults to embedding for backward compatibility.
    #[serde(default)]
    pub method: PassMethod,
    #[serde(default)]
    pub window_size: u32,
    #[serde(default)]
    pub stride: u32,
}

/// Optional wrapper for UI / RF nested `generate.similarity_map` YAML.
#[derive(Debug, Deserialize)]
struct NestedGenerateWrapper {
    generate: NestedSimilarityMap,
}

#[derive(Debug, Deserialize)]
struct NestedSimilarityMap {
    similarity_map: PassConfigFile,
}

#[derive(Debug, Deserialize)]
struct NestedSimilarityMapOnly {
    similarity_map: PassConfigFile,
}

fn default_min_repetitions() -> u32 {
    2
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
            lexical: file.lexical,
            passes: file
                .passes
                .into_iter()
                .map(|p| PassSpec {
                    name: p.name,
                    scope: p.scope,
                    method: p.method,
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
        Self::from_yaml_str(&raw)
    }

    /// Accept flat pass config YAML or nested `generate.similarity_map` / `similarity_map` wrappers.
    pub fn from_yaml_str(raw: &str) -> Result<Self, String> {
        if let Ok(cfg) = serde_yaml::from_str::<PassConfigFile>(raw) {
            if !cfg.passes.is_empty() {
                return Ok(cfg);
            }
        }
        if let Ok(wrapper) = serde_yaml::from_str::<NestedGenerateWrapper>(raw) {
            return Ok(wrapper.generate.similarity_map);
        }
        if let Ok(wrapper) = serde_yaml::from_str::<NestedSimilarityMapOnly>(raw) {
            return Ok(wrapper.similarity_map);
        }
        serde_yaml::from_str(raw).map_err(|e| format!("invalid pass config YAML: {e}"))
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
            method: pass.method,
            window_size: pass.window_size,
            stride: pass.stride,
        })
    }

    pub fn needs_embedder(&self) -> bool {
        let config: MultiPassConfig = self.clone().into();
        config.needs_embedder()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_generate_similarity_map() {
        let yaml = r#"
generate:
  similarity_map:
    min_repetitions: 2
    min_samples: 2
    enable_hdbscan: false
    passes:
      - name: chapter_lexical
        scope: chapter
        method: lexical
"#;
        let cfg = PassConfigFile::from_yaml_str(yaml).expect("parse nested");
        assert_eq!(cfg.passes.len(), 1);
        assert_eq!(cfg.passes[0].method, PassMethod::Lexical);
        assert!(!cfg.needs_embedder());
    }

    #[test]
    fn parses_flat_legacy_embedding_yaml() {
        let yaml = r#"
min_repetitions: 2
min_samples: 2
enable_hdbscan: false
passes:
  - name: chapter_5_5
    scope: chapter
    window_size: 5
    stride: 5
"#;
        let cfg = PassConfigFile::from_yaml_str(yaml).expect("parse flat");
        assert_eq!(cfg.passes[0].method, PassMethod::Embedding);
        assert!(cfg.needs_embedder());
    }
}
