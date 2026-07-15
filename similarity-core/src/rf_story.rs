//! Romance Factory story directory helpers — act drafts, chapter scope, chapter listing.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::contract::assemble_rf_chapter_scope;
use crate::hash::compute_text_hash;
use crate::report::{AnalysisScope, ScopeManifest};
use crate::types::{AppError, ImportError, ValidationError};

/// Canonical act draft filename: `chapter_01_act_02.json` (not intro/rev variants).
static DRAFT_ACT_RE: &str = r"^chapter_(\d+)_act_(\d+)\.json$";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfChapterList {
    pub story_path: String,
    pub chapters: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfChapterScope {
    pub chapter: u32,
    pub text: String,
    pub scope_manifest: ScopeManifest,
    pub act_count: u32,
    pub paragraph_count: u32,
    pub document_hash: String,
    pub act_files: Vec<String>,
    pub story_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfChapterDraft {
    pub chapter: u32,
    pub text: String,
    pub scope_manifest: ScopeManifest,
    pub document_hash: String,
    pub story_path: PathBuf,
    pub act_files: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct DraftJson {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OutlineRoot {
    chapters: Option<Vec<OutlineChapter>>,
}

#[derive(Debug, Deserialize)]
struct OutlineChapter {
    chapter_number: Option<u32>,
}

fn draft_act_re() -> Result<Regex, AppError> {
    Regex::new(DRAFT_ACT_RE).map_err(|e| {
        AppError::Validation(ValidationError {
            field: "draft_act_re".into(),
            message: e.to_string(),
        })
    })
}

fn is_canonical_act_draft(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.contains("_intro") || lower.contains("_rev") {
        return false;
    }
    draft_act_re()
        .map(|re| re.is_match(&lower))
        .unwrap_or(false)
}

/// Discover chapter numbers from `drafts/chapter_*_act_*.json` and optional `story_outline.json`.
pub fn list_rf_chapters(story_path: &Path) -> Result<RfChapterList, AppError> {
    if !story_path.is_dir() {
        return Err(AppError::Import(ImportError {
            message: format!("story path is not a directory: {}", story_path.display()),
            path: Some(story_path.display().to_string()),
        }));
    }

    let mut chapters: Vec<u32> = Vec::new();
    let drafts_dir = story_path.join("drafts");
    if drafts_dir.is_dir() {
        let re = draft_act_re()?;
        for entry in std::fs::read_dir(&drafts_dir).map_err(|e| map_io_error(&drafts_dir, e))? {
            let entry = entry.map_err(|e| map_io_error(&drafts_dir, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_canonical_act_draft(&name) {
                continue;
            }
            if let Some(caps) = re.captures(&name.to_lowercase()) {
                if let Some(ch) = caps.get(1).and_then(|m| m.as_str().parse().ok()) {
                    chapters.push(ch);
                }
            }
        }
    }

    chapters.sort_unstable();
    chapters.dedup();

    if chapters.is_empty() {
        if let Ok(from_outline) = chapters_from_outline(story_path) {
            chapters = from_outline;
        }
    }

    Ok(RfChapterList {
        story_path: story_path.display().to_string(),
        chapters,
    })
}

fn chapters_from_outline(story_path: &Path) -> Result<Vec<u32>, AppError> {
    let outline_path = story_path.join("story_outline.json");
    if !outline_path.is_file() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&outline_path).map_err(|e| map_io_error(&outline_path, e))?;
    let parsed: OutlineRoot = serde_json::from_str(&raw).map_err(|e| {
        AppError::Import(ImportError {
            message: format!("invalid story_outline.json: {e}"),
            path: Some(outline_path.display().to_string()),
        })
    })?;
    let mut chapters: Vec<u32> = parsed
        .chapters
        .unwrap_or_default()
        .into_iter()
        .filter_map(|ch| ch.chapter_number.filter(|n| *n > 0))
        .collect();
    chapters.sort_unstable();
    chapters.dedup();
    Ok(chapters)
}

/// Load act draft paths for one chapter, sorted by act number.
pub fn act_draft_paths_for_chapter(
    story_path: &Path,
    chapter: u32,
) -> Result<Vec<PathBuf>, AppError> {
    let drafts_dir = story_path.join("drafts");
    if !drafts_dir.is_dir() {
        return Err(AppError::Import(ImportError {
            message: format!("drafts/ not found under {}", story_path.display()),
            path: Some(drafts_dir.display().to_string()),
        }));
    }

    let re = draft_act_re()?;
    let nn = format!("chapter_{chapter:02}_act_");
    let mut rows: Vec<(u32, PathBuf)> = Vec::new();

    for entry in std::fs::read_dir(&drafts_dir).map_err(|e| map_io_error(&drafts_dir, e))? {
        let entry = entry.map_err(|e| map_io_error(&drafts_dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&nn) || !is_canonical_act_draft(&name) {
            continue;
        }
        if let Some(caps) = re.captures(&name.to_lowercase()) {
            let ch: u32 = caps
                .get(1)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let act: u32 = caps
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            if ch == chapter && act > 0 {
                rows.push((act, entry.path()));
            }
        }
    }

    rows.sort_by_key(|(act, _)| *act);
    if rows.is_empty() {
        return Err(AppError::Import(ImportError {
            message: format!(
                "no act drafts found for chapter {chapter} under {}/drafts/",
                story_path.display()
            ),
            path: Some(drafts_dir.display().to_string()),
        }));
    }

    Ok(rows.into_iter().map(|(_, path)| path).collect())
}

fn load_act_text(path: &Path) -> Result<String, AppError> {
    let raw = std::fs::read_to_string(path).map_err(|e| map_io_error(path, e))?;
    let parsed: DraftJson = serde_json::from_str(&raw).map_err(|e| {
        AppError::Import(ImportError {
            message: format!("invalid JSON in {}: {e}", path.display()),
            path: Some(path.display().to_string()),
        })
    })?;
    parsed.text.filter(|t| !t.trim().is_empty()).ok_or_else(|| {
        AppError::Import(ImportError {
            message: format!("{} missing non-empty \"text\" field", path.display()),
            path: Some(path.display().to_string()),
        })
    })
}

/// Build chapter scope manifest and concatenated prose from on-disk act drafts.
pub fn build_rf_chapter_scope(story_path: &Path, chapter: u32) -> Result<RfChapterScope, AppError> {
    let draft = load_rf_chapter(story_path, chapter)?;
    let paragraph_count: u32 = draft
        .scope_manifest
        .acts
        .iter()
        .map(|a| a.paragraphs.len() as u32)
        .sum();

    Ok(RfChapterScope {
        chapter: draft.chapter,
        text: draft.text.clone(),
        scope_manifest: draft.scope_manifest.clone(),
        act_count: draft.scope_manifest.acts.len() as u32,
        paragraph_count,
        document_hash: draft.document_hash.clone(),
        act_files: draft
            .act_files
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        story_path: story_path.display().to_string(),
    })
}

/// Load chapter prose + manifest from RF act drafts (pipeline-compatible assembly).
pub fn load_rf_chapter(story_path: &Path, chapter: u32) -> Result<RfChapterDraft, AppError> {
    let act_paths = act_draft_paths_for_chapter(story_path, chapter)?;
    let re = draft_act_re()?;
    let mut act_bodies: Vec<(u32, String)> = Vec::with_capacity(act_paths.len());

    for path in &act_paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let act_num = re
            .captures(&name.to_lowercase())
            .and_then(|c| c.get(2))
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(act_bodies.len() as u32 + 1);
        let text = load_act_text(path)?;
        act_bodies.push((act_num, text));
    }

    let (text, scope_manifest) =
        assemble_rf_chapter_scope(chapter, &act_bodies).map_err(AppError::Validation)?;

    Ok(RfChapterDraft {
        chapter,
        document_hash: compute_text_hash(&text),
        text,
        scope_manifest,
        story_path: story_path.to_path_buf(),
        act_files: act_paths,
    })
}

/// Chapter-level analysis scope for multi-pass runs.
pub fn chapter_scope_from_manifest(text: &str, manifest: &ScopeManifest) -> AnalysisScope {
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

fn map_io_error(path: &Path, err: std::io::Error) -> AppError {
    AppError::Import(ImportError {
        message: format!("failed to read {}: {err}", path.display()),
        path: Some(path.display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::format_segment_id;
    use std::fs;

    fn write_act(drafts: &Path, chapter: u32, act: u32, text: &str) -> PathBuf {
        let path = drafts.join(format!("chapter_{chapter:02}_act_{act:02}.json"));
        let json = serde_json::json!({
            "artifact_type": "act",
            "text": text,
            "metadata": {},
            "created_at": "2026-01-01T00:00:00Z",
            "file_path": path.display().to_string(),
        });
        fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();
        path
    }

    #[test]
    fn list_chapters_from_act_drafts() {
        let tmp = tempfile::tempdir().unwrap();
        let drafts = tmp.path().join("drafts");
        fs::create_dir_all(&drafts).unwrap();
        write_act(&drafts, 2, 1, "b");
        write_act(&drafts, 1, 2, "a");
        write_act(&drafts, 1, 1, "first");
        fs::write(drafts.join("chapter_01_act_02_intro.json"), "{}").unwrap();
        fs::write(drafts.join("chapter_01_act_02_rev01.json"), "{}").unwrap();

        let list = list_rf_chapters(tmp.path()).unwrap();
        assert_eq!(list.chapters, vec![1, 2]);
    }

    #[test]
    fn build_scope_joins_acts_and_preserves_internal_paragraphs() {
        let tmp = tempfile::tempdir().unwrap();
        let drafts = tmp.path().join("drafts");
        fs::create_dir_all(&drafts).unwrap();
        write_act(&drafts, 1, 1, "para one\n\npara two");
        write_act(&drafts, 1, 2, "act two body");

        let scope = build_rf_chapter_scope(tmp.path(), 1).unwrap();
        assert_eq!(scope.act_count, 2);
        assert_eq!(scope.paragraph_count, 3);
        assert!(scope.text.contains("para one"));
        assert!(scope.text.contains("act two body"));
        assert_eq!(scope.scope_manifest.acts.len(), 2);
        assert_eq!(
            scope.scope_manifest.acts[0].paragraphs[0].segment_id,
            format_segment_id(1, 1, 1)
        );
        assert_eq!(
            scope.scope_manifest.acts[0].paragraphs[1].segment_id,
            format_segment_id(1, 1, 2)
        );
    }

    #[test]
    fn missing_drafts_returns_import_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = build_rf_chapter_scope(tmp.path(), 1).unwrap_err();
        assert!(matches!(err, AppError::Import(_)));
    }
}
