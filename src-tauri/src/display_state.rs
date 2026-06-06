use std::path::{Path, PathBuf};

use similarity_core::types::{AppError, DisplayState, SessionError};

/// Returns the sessions directory path: `<app_data_dir>/sessions`
pub fn sessions_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("sessions")
}

/// Returns the JSON file path for a job's display state: `<app_data_dir>/sessions/<job_id>.json`
pub fn display_state_path(app_data_dir: &Path, job_id: &str) -> PathBuf {
    sessions_dir(app_data_dir).join(format!("{}.json", job_id))
}

/// Write display state to the sidecar JSON file.
/// Creates the sessions directory if it doesn't exist.
pub fn save_display_state(app_data_dir: &Path, state: &DisplayState) -> Result<(), AppError> {
    let dir = sessions_dir(app_data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to create sessions directory: {}", e),
        })
    })?;

    let path = display_state_path(app_data_dir, &state.job_id);
    let json = serde_json::to_string_pretty(state).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to serialize display state: {}", e),
        })
    })?;

    std::fs::write(&path, json).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to write display state file: {}", e),
        })
    })?;

    Ok(())
}

/// Load display state from the sidecar JSON file.
/// Returns defaults if the file is missing or corrupt.
pub fn load_display_state(app_data_dir: &Path, job_id: &str) -> DisplayState {
    let path = display_state_path(app_data_dir, job_id);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            return DisplayState {
                job_id: job_id.to_string(),
                ..Default::default()
            };
        }
    };

    match serde_json::from_str::<DisplayState>(&content) {
        Ok(state) => state,
        Err(_) => DisplayState {
            job_id: job_id.to_string(),
            ..Default::default()
        },
    }
}

/// Delete the display state JSON file for a job.
/// Returns Ok if the file was deleted or didn't exist.
pub fn delete_display_state(app_data_dir: &Path, job_id: &str) -> Result<(), AppError> {
    let path = display_state_path(app_data_dir, job_id);

    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Session(SessionError {
            message: format!("Failed to delete display state file: {}", e),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_state(job_id: &str) -> DisplayState {
        DisplayState {
            job_id: job_id.to_string(),
            tolerance: 0.92,
            gamma: 2.0,
            hidden_clusters: vec![1, 3, 5],
            zoom: 2.5,
            scroll_x: 100.0,
            scroll_y: 200.0,
            saved_at: "2024-01-15T10:30:00Z".to_string(),
        }
    }

    #[test]
    fn test_save_and_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let app_data_dir = tmp.path();
        let state = sample_state("job-123");

        save_display_state(app_data_dir, &state).unwrap();
        let loaded = load_display_state(app_data_dir, "job-123");

        assert_eq!(loaded.job_id, "job-123");
        assert_eq!(loaded.tolerance, 0.92);
        assert_eq!(loaded.gamma, 2.0);
        assert_eq!(loaded.hidden_clusters, vec![1, 3, 5]);
        assert_eq!(loaded.zoom, 2.5);
        assert_eq!(loaded.scroll_x, 100.0);
        assert_eq!(loaded.scroll_y, 200.0);
        assert_eq!(loaded.saved_at, "2024-01-15T10:30:00Z");
    }

    #[test]
    fn test_missing_file_returns_defaults() {
        let tmp = TempDir::new().unwrap();
        let app_data_dir = tmp.path();

        let loaded = load_display_state(app_data_dir, "nonexistent-job");

        assert_eq!(loaded.job_id, "nonexistent-job");
        assert_eq!(loaded.tolerance, 0.88);
        assert_eq!(loaded.gamma, 1.5);
        assert!(loaded.hidden_clusters.is_empty());
        assert_eq!(loaded.zoom, 1.0);
        assert_eq!(loaded.scroll_x, 0.0);
        assert_eq!(loaded.scroll_y, 0.0);
    }

    #[test]
    fn test_corrupt_file_returns_defaults() {
        let tmp = TempDir::new().unwrap();
        let app_data_dir = tmp.path();

        // Create sessions directory and write corrupt JSON
        let dir = sessions_dir(app_data_dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = display_state_path(app_data_dir, "corrupt-job");
        std::fs::write(&path, "{ this is not valid json !!!").unwrap();

        let loaded = load_display_state(app_data_dir, "corrupt-job");

        assert_eq!(loaded.job_id, "corrupt-job");
        assert_eq!(loaded.tolerance, 0.88);
        assert_eq!(loaded.gamma, 1.5);
        assert!(loaded.hidden_clusters.is_empty());
        assert_eq!(loaded.zoom, 1.0);
        assert_eq!(loaded.scroll_x, 0.0);
        assert_eq!(loaded.scroll_y, 0.0);
    }

    #[test]
    fn test_delete_removes_file() {
        let tmp = TempDir::new().unwrap();
        let app_data_dir = tmp.path();
        let state = sample_state("job-to-delete");

        save_display_state(app_data_dir, &state).unwrap();

        // Verify file exists
        let path = display_state_path(app_data_dir, "job-to-delete");
        assert!(path.exists());

        // Delete it
        delete_display_state(app_data_dir, "job-to-delete").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_delete_nonexistent_file_is_ok() {
        let tmp = TempDir::new().unwrap();
        let app_data_dir = tmp.path();

        // Deleting a file that doesn't exist should succeed
        let result = delete_display_state(app_data_dir, "no-such-job");
        assert!(result.is_ok());
    }

    #[test]
    fn test_sessions_dir_path() {
        let base = Path::new("/tmp/app_data");
        assert_eq!(sessions_dir(base), PathBuf::from("/tmp/app_data/sessions"));
    }

    #[test]
    fn test_display_state_path_format() {
        let base = Path::new("/tmp/app_data");
        assert_eq!(
            display_state_path(base, "abc-123"),
            PathBuf::from("/tmp/app_data/sessions/abc-123.json")
        );
    }
}
