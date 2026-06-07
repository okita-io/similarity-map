use std::path::{Path, PathBuf};

use serde::Deserialize;
use similarity_core::hash::compute_text_hash;

#[derive(Debug, Clone)]
pub struct ChapterDraft {
    pub chapter: u32,
    pub text: String,
    pub source_path: PathBuf,
    pub document_hash: String,
}

#[derive(Debug, Deserialize)]
struct DraftJson {
    text: Option<String>,
}

/// Load chapter prose from a Romance Factory story directory.
pub fn load_chapter(story_path: &Path, chapter: u32) -> Result<ChapterDraft, String> {
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
        return Ok(ChapterDraft {
            chapter,
            text: trimmed.to_string(),
            document_hash: compute_text_hash(trimmed),
            source_path: path,
        });
    }

    Err(format!(
        "no chapter source found for chapter {chapter} under {} (tried drafts/chapter_{chapter:02}.json, chapters/chapter_{chapter:02}.md, drafts/chapter_{chapter:02}.md)",
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
    let parsed: DraftJson = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid JSON in {}: {e}", path.display()))?;
    parsed
        .text
        .filter(|t| !t.trim().is_empty())
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

    #[test]
    fn strip_header_keeps_body() {
        let md = "---\ntitle: x\n---\n\nFirst paragraph.\n";
        assert_eq!(strip_markdown_header(md).trim(), "First paragraph.");
    }
}
