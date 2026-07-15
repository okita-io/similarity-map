//! Persist `AnalysisOutput` JSON sidecars next to display-state session files.

use std::path::{Path, PathBuf};

use similarity_core::types::{AppError, SessionError};
use similarity_core::AnalysisOutput;

use crate::display_state::sessions_dir;

/// `<app_data_dir>/sessions/<job_id>.analysis_output.json`
pub fn analysis_output_path(app_data_dir: &Path, job_id: &str) -> PathBuf {
    sessions_dir(app_data_dir).join(format!("{job_id}.analysis_output.json"))
}

pub fn save_analysis_output(
    app_data_dir: &Path,
    job_id: &str,
    output: &AnalysisOutput,
) -> Result<(), AppError> {
    let dir = sessions_dir(app_data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to create sessions directory: {e}"),
        })
    })?;

    similarity_core::validate_analysis_output(output).map_err(|e| {
        AppError::Validation(similarity_core::types::ValidationError {
            field: "analysis_output".into(),
            message: e.to_string(),
        })
    })?;

    let json = similarity_core::to_export_json(output).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to serialize analysis output: {e}"),
        })
    })?;

    let path = analysis_output_path(app_data_dir, job_id);
    std::fs::write(&path, json).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to write analysis output sidecar: {e}"),
        })
    })?;
    Ok(())
}

pub fn load_analysis_output(app_data_dir: &Path, job_id: &str) -> Option<AnalysisOutput> {
    let path = analysis_output_path(app_data_dir, job_id);
    let raw = std::fs::read_to_string(path).ok()?;
    let output: AnalysisOutput = serde_json::from_str(&raw).ok()?;
    similarity_core::validate_analysis_output(&output).ok()?;
    Some(output)
}

pub fn delete_analysis_output(app_data_dir: &Path, job_id: &str) -> Result<(), AppError> {
    let path = analysis_output_path(app_data_dir, job_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Session(SessionError {
            message: format!("Failed to delete analysis output sidecar: {e}"),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similarity_core::contract::{
        build_analysis_output_with_manifest, repetition_report_to_v1, AnalysisPassRecord,
        PassMethod,
    };
    use similarity_core::report::AnalysisScope;
    use similarity_core::{analyze_lexical, build_scope_manifest, LexicalPassConfig};

    #[test]
    fn round_trip_analysis_output_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let sentence = "I've found it, she declared, her voice echoing through the vast chamber like a prophecy fulfilled and ancient drums.";
        let text = format!("{sentence}\n\nBridge.\n\n{sentence}");
        let manifest = build_scope_manifest(1, &text, 0);
        let (report, _) =
            analyze_lexical(&text, &manifest, &LexicalPassConfig::default(), "job").unwrap();
        let len = text.len() as u32;
        let scope = AnalysisScope {
            chapter: 1,
            act: None,
            document_path: None,
            document_hash: None,
            scope_char_start: 0,
            scope_char_end: len,
            doc_char_start: 0,
            doc_char_end: len,
        };
        let v1 = repetition_report_to_v1(&report, &manifest, 0, &text);
        let output = build_analysis_output_with_manifest(
            scope,
            manifest,
            vec![AnalysisPassRecord {
                pass_id: "chapter_lexical".into(),
                pass_label: "lexical".into(),
                method: PassMethod::Lexical,
                scope: AnalysisScope {
                    chapter: 1,
                    act: None,
                    document_path: None,
                    document_hash: None,
                    scope_char_start: 0,
                    scope_char_end: len,
                    doc_char_start: 0,
                    doc_char_end: len,
                },
                window_size: 0,
                stride: 0,
                tokens_per_page: None,
                repetition_report: v1,
            }],
        );
        save_analysis_output(dir.path(), "job-1", &output).unwrap();
        let loaded = load_analysis_output(dir.path(), "job-1").expect("sidecar");
        assert_eq!(loaded.passes[0].pass_id, "chapter_lexical");
        delete_analysis_output(dir.path(), "job-1").unwrap();
        assert!(load_analysis_output(dir.path(), "job-1").is_none());
    }
}
