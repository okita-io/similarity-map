use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use similarity_core::storage::JobRecord;
use similarity_core::types::{AppError, SessionError};

/// A named saved analysis result for a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedResultEntry {
    pub result_id: String,
    pub name: String,
    pub job_id: String,
    pub window_size: u32,
    pub stride: u32,
    pub page_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Catalog of saved results for one document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocumentResultsCatalog {
    pub document_path: String,
    pub active_result_id: Option<String>,
    pub results: Vec<SavedResultEntry>,
}

/// Response returned to the frontend when listing or mutating results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentResultsList {
    pub document_path: String,
    pub active_result_id: Option<String>,
    pub active_job_id: Option<String>,
    pub results: Vec<SavedResultEntry>,
}

pub fn results_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("results")
}

pub fn catalog_path(app_data_dir: &Path, document_path: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(document_path.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    results_dir(app_data_dir).join(format!("{}.json", hash))
}

pub fn load_catalog(app_data_dir: &Path, document_path: &str) -> DocumentResultsCatalog {
    let path = catalog_path(app_data_dir, document_path);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            return DocumentResultsCatalog {
                document_path: document_path.to_string(),
                ..Default::default()
            };
        }
    };

    match serde_json::from_str::<DocumentResultsCatalog>(&content) {
        Ok(mut catalog) => {
            catalog.document_path = document_path.to_string();
            catalog
        }
        Err(_) => DocumentResultsCatalog {
            document_path: document_path.to_string(),
            ..Default::default()
        },
    }
}

pub fn save_catalog(
    app_data_dir: &Path,
    catalog: &DocumentResultsCatalog,
) -> Result<(), AppError> {
    let dir = results_dir(app_data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to create results directory: {}", e),
        })
    })?;

    let path = catalog_path(app_data_dir, &catalog.document_path);
    let json = serde_json::to_string_pretty(catalog).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to serialize results catalog: {}", e),
        })
    })?;

    std::fs::write(&path, json).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to write results catalog: {}", e),
        })
    })?;

    Ok(())
}

pub fn default_result_name(window_size: u32, stride: u32) -> String {
    format!("{window_size} tokens (stride {stride})")
}

pub fn sync_catalog_with_jobs(
    catalog: &mut DocumentResultsCatalog,
    jobs: &[JobRecord],
    page_counts: &std::collections::HashMap<String, u32>,
    valid_job_ids: &HashSet<String>,
) {
    catalog.results.retain(|entry| valid_job_ids.contains(&entry.job_id));

    for job in jobs {
        if job.status != "complete" || !valid_job_ids.contains(&job.job_id) {
            continue;
        }

        if catalog.results.iter().any(|entry| entry.job_id == job.job_id) {
            continue;
        }

        let now = Utc::now().to_rfc3339();
        catalog.results.push(SavedResultEntry {
            result_id: Uuid::new_v4().to_string(),
            name: default_result_name(job.window_size, job.stride),
            job_id: job.job_id.clone(),
            window_size: job.window_size,
            stride: job.stride,
            page_count: page_counts.get(&job.job_id).copied().unwrap_or(0),
            created_at: job.created_at.clone(),
            updated_at: now,
        });
    }

    catalog.results.sort_by(|a, b| {
        a.window_size
            .cmp(&b.window_size)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });

    if let Some(active_id) = &catalog.active_result_id {
        if !catalog.results.iter().any(|entry| entry.result_id == *active_id) {
            catalog.active_result_id = None;
        }
    }
}

pub fn to_list(catalog: &DocumentResultsCatalog) -> DocumentResultsList {
    let active_job_id = catalog
        .active_result_id
        .as_ref()
        .and_then(|active_id| {
            catalog
                .results
                .iter()
                .find(|entry| entry.result_id == *active_id)
                .map(|entry| entry.job_id.clone())
        });

    DocumentResultsList {
        document_path: catalog.document_path.clone(),
        active_result_id: catalog.active_result_id.clone(),
        active_job_id,
        results: catalog.results.clone(),
    }
}

pub fn rename_result(
    catalog: &mut DocumentResultsCatalog,
    result_id: &str,
    name: &str,
) -> Result<(), AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(similarity_core::types::ValidationError {
            message: "Result name cannot be empty".to_string(),
            field: "name".to_string(),
        }));
    }

    if catalog
        .results
        .iter()
        .any(|entry| entry.name == trimmed && entry.result_id != result_id)
    {
        return Err(AppError::Validation(similarity_core::types::ValidationError {
            message: format!("A result named \"{trimmed}\" already exists"),
            field: "name".to_string(),
        }));
    }

    let entry = catalog
        .results
        .iter_mut()
        .find(|entry| entry.result_id == result_id)
        .ok_or_else(|| AppError::Session(SessionError {
            message: format!("Result not found: {result_id}"),
        }))?;

    entry.name = trimmed.to_string();
    entry.updated_at = Utc::now().to_rfc3339();
    Ok(())
}

pub fn add_result_alias(
    catalog: &mut DocumentResultsCatalog,
    job: &JobRecord,
    page_count: u32,
    name: &str,
) -> Result<SavedResultEntry, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(similarity_core::types::ValidationError {
            message: "Result name cannot be empty".to_string(),
            field: "name".to_string(),
        }));
    }

    if catalog.results.iter().any(|entry| entry.name == trimmed) {
        return Err(AppError::Validation(similarity_core::types::ValidationError {
            message: format!("A result named \"{trimmed}\" already exists"),
            field: "name".to_string(),
        }));
    }

    let now = Utc::now().to_rfc3339();
    let entry = SavedResultEntry {
        result_id: Uuid::new_v4().to_string(),
        name: trimmed.to_string(),
        job_id: job.job_id.clone(),
        window_size: job.window_size,
        stride: job.stride,
        page_count,
        created_at: now.clone(),
        updated_at: now,
    };

    catalog.results.push(entry.clone());
    catalog.results.sort_by(|a, b| {
        a.window_size
            .cmp(&b.window_size)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });

    Ok(entry)
}

pub fn remove_result(
    catalog: &mut DocumentResultsCatalog,
    result_id: &str,
) -> Result<Option<String>, AppError> {
    let index = catalog
        .results
        .iter()
        .position(|entry| entry.result_id == result_id)
        .ok_or_else(|| AppError::Session(SessionError {
            message: format!("Result not found: {result_id}"),
        }))?;

    let removed = catalog.results.remove(index);
    if catalog.active_result_id.as_deref() == Some(result_id) {
        catalog.active_result_id = None;
    }

    let should_discard_job = !catalog
        .results
        .iter()
        .any(|entry| entry.job_id == removed.job_id);

    Ok(if should_discard_job {
        Some(removed.job_id)
    } else {
        None
    })
}

pub fn set_active_result(catalog: &mut DocumentResultsCatalog, result_id: &str) -> Result<(), AppError> {
    if !catalog
        .results
        .iter()
        .any(|entry| entry.result_id == result_id)
    {
        return Err(AppError::Session(SessionError {
            message: format!("Result not found: {result_id}"),
        }));
    }

    catalog.active_result_id = Some(result_id.to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_job(id: &str, window_size: u32, stride: u32) -> JobRecord {
        JobRecord {
            job_id: id.to_string(),
            document_path: "/tmp/book.txt".to_string(),
            document_hash: "abc".to_string(),
            settings_hash: "def".to_string(),
            window_size,
            stride,
            tokens_per_page: Some(400),
            pagination_mode: "token".to_string(),
            min_repetitions: 3,
            min_samples: 3,
            chapter_break_re: None,
            windows_total: 100,
            windows_committed: 100,
            status: "complete".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn sync_adds_missing_complete_jobs() {
        let mut catalog = DocumentResultsCatalog::default();
        let jobs = vec![sample_job("job-1", 20, 5)];
        let mut page_counts = std::collections::HashMap::new();
        page_counts.insert("job-1".to_string(), 42);
        let valid = HashSet::from(["job-1".to_string()]);

        sync_catalog_with_jobs(&mut catalog, &jobs, &page_counts, &valid);

        assert_eq!(catalog.results.len(), 1);
        assert_eq!(catalog.results[0].job_id, "job-1");
        assert_eq!(catalog.results[0].page_count, 42);
    }

    #[test]
    fn delete_result_only_discards_when_last_reference() {
        let mut catalog = DocumentResultsCatalog::default();
        catalog.results.push(SavedResultEntry {
            result_id: "r1".to_string(),
            name: "A".to_string(),
            job_id: "job-1".to_string(),
            window_size: 20,
            stride: 5,
            page_count: 10,
            created_at: "2024".to_string(),
            updated_at: "2024".to_string(),
        });
        catalog.results.push(SavedResultEntry {
            result_id: "r2".to_string(),
            name: "B".to_string(),
            job_id: "job-1".to_string(),
            window_size: 20,
            stride: 5,
            page_count: 10,
            created_at: "2024".to_string(),
            updated_at: "2024".to_string(),
        });

        let discard = remove_result(&mut catalog, "r1").unwrap();
        assert!(discard.is_none());
        assert_eq!(catalog.results.len(), 1);

        let discard = remove_result(&mut catalog, "r2").unwrap();
        assert_eq!(discard.as_deref(), Some("job-1"));
        assert!(catalog.results.is_empty());
    }

    #[test]
    fn save_and_load_catalog_round_trip() {
        let tmp = TempDir::new().unwrap();
        let mut catalog = DocumentResultsCatalog {
            document_path: "/books/demo.txt".to_string(),
            active_result_id: None,
            results: vec![],
        };
        catalog.results.push(SavedResultEntry {
            result_id: "r1".to_string(),
            name: "20 tokens".to_string(),
            job_id: "job-1".to_string(),
            window_size: 20,
            stride: 5,
            page_count: 12,
            created_at: "2024".to_string(),
            updated_at: "2024".to_string(),
        });

        save_catalog(tmp.path(), &catalog).unwrap();
        let loaded = load_catalog(tmp.path(), "/books/demo.txt");
        assert_eq!(loaded.results.len(), 1);
        assert_eq!(loaded.results[0].name, "20 tokens");
    }
}
