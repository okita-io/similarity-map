use std::path::{Path, PathBuf};

use similarity_core::hash::compute_text_hash;
use similarity_core::{load_rf_chapter, ScopeManifest};

#[derive(Debug, Clone)]
pub struct ChapterDraft {
    pub chapter: u32,
    pub text: String,
    pub source_path: PathBuf,
    pub document_hash: String,
    pub scope_manifest: ScopeManifest,
}

/// Load chapter prose from a Romance Factory story directory.
///
/// Prefers act drafts (`drafts/chapter_NN_act_MM.json`); falls back to assembled
/// chapter JSON or markdown when no act drafts exist.
pub fn load_chapter(story_path: &Path, chapter: u32) -> Result<ChapterDraft, String> {
    if let Ok(draft) = load_rf_chapter(story_path, chapter) {
        return Ok(ChapterDraft {
            chapter: draft.chapter,
            text: draft.text,
            source_path: draft
                .act_files
                .first()
                .cloned()
                .unwrap_or_else(|| story_path.join("drafts")),
            document_hash: draft.document_hash,
            scope_manifest: draft.scope_manifest,
        });
    }

    let candidates = chapter_source_candidates(story_path, chapter);
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        let text = if path.extension().and_then(|e| e.to_str()) == Some("json") {
            load_chapter_json(&path)?
        } else {
            load_chapter_markdown(&path)?
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(format!(
                "chapter source {} contains no prose",
                path.display()
            ));
        }
        let scope = similarity_core::build_scope_manifest(chapter, trimmed, 0);
        return Ok(ChapterDraft {
            chapter,
            text: trimmed.to_string(),
            document_hash: compute_text_hash(trimmed),
            source_path: path,
            scope_manifest: scope,
        });
    }

    Err(format!(
        "no chapter source found for chapter {chapter} under {} (tried act drafts, drafts/chapter_{chapter:02}.json, chapters/chapter_{chapter:02}.md)",
        story_path.display()
    ))
}

fn chapter_source_candidates(story_path: &Path, chapter: u32) -> Vec<PathBuf> {
    let nn = format!("chapter_{chapter:02}");
    vec![
        story_path.join("drafts").join(format!("{nn}.json")),
        story_path.join("chapters").join(format!("{nn}.md")),
        story_path.join("drafts").join(format!("{nn}.md")),
    ]
}

fn load_chapter_json(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid JSON in {}: {e}", path.display()))?;
    parsed
        .get("text")
        .and_then(|v| v.as_str())
        .filter(|t| !t.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("{} missing non-empty \"text\" field", path.display()))
}

fn load_chapter_markdown(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    Ok(strip_markdown_header(&raw))
}

/// Return prose body after an optional YAML-style `---` header block.
fn strip_markdown_header(content: &str) -> String {
    for sep in ["---\n\n", "---\r\n\r\n"] {
        if let Some((_header, body)) = content.split_once(sep) {
            return body.to_string();
        }
    }
    content.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_act(drafts: &Path, chapter: u32, act: u32, text: &str) {
        let path = drafts.join(format!("chapter_{chapter:02}_act_{act:02}.json"));
        let json = serde_json::json!({
            "artifact_type": "act",
            "text": text,
            "metadata": {},
            "created_at": "2026-01-01T00:00:00Z",
            "file_path": path.display().to_string(),
        });
        fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();
    }

    #[test]
    fn load_chapter_prefers_act_drafts() {
        let tmp = tempfile::tempdir().unwrap();
        let drafts = tmp.path().join("drafts");
        fs::create_dir_all(&drafts).unwrap();
        write_act(&drafts, 1, 1, "from acts");
        let chapter_json = drafts.join("chapter_01.json");
        fs::write(
            &chapter_json,
            serde_json::json!({"text": "from chapter json"}).to_string(),
        )
        .unwrap();

        let draft = load_chapter(tmp.path(), 1).expect("loads act drafts");
        assert_eq!(draft.text, "from acts");
        assert_eq!(draft.scope_manifest.acts.len(), 1);
    }

    #[test]
    fn strip_header_keeps_body() {
        let md = "---\ntitle: x\n---\n\nFirst paragraph.\n";
        assert_eq!(strip_markdown_header(md).trim(), "First paragraph.");
    }
}
