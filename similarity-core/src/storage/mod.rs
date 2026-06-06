//! LanceDB storage layer for persisting embeddings, cluster assignments, and job metadata.
//!
//! This module manages the local LanceDB database connection and defines the schemas
//! for the `windows`, `pages`, and `jobs` tables.

pub mod crud;
mod schema;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lancedb::connection::Connection;
use tokio::sync::OnceCell;

pub use crud::{EmbeddingRecord, InsertJobParams, JobRecord, PageRecord, WindowRecord};
pub use schema::{jobs_schema, pages_schema, windows_schema};

/// Table name constants.
pub const TABLE_WINDOWS: &str = "windows";
pub const TABLE_PAGES: &str = "pages";
pub const TABLE_JOBS: &str = "jobs";

/// Errors specific to the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("LanceDB error: {0}")]
    Lance(#[from] lancedb::Error),

    #[error("Database path error: {0}")]
    PathError(String),
}

/// The main storage handle wrapping a LanceDB connection.
///
/// Use [`Storage::open`] to connect to (or create) the database at a given directory.
/// Tables are created lazily on first use via [`Storage::ensure_tables`].
#[derive(Clone)]
pub struct Storage {
    connection: Connection,
    db_path: PathBuf,
}

/// Global singleton for the storage connection, initialized once per app lifetime.
static STORAGE: OnceCell<Storage> = OnceCell::const_new();

impl Storage {
    /// Opens (or creates) a LanceDB database at the given directory path.
    ///
    /// The directory will be created if it does not exist.
    pub async fn open(db_dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db_path = db_dir.as_ref().to_path_buf();

        // Ensure the parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError::PathError(format!("Cannot create directory: {e}")))?;
        }

        let uri = db_path
            .to_str()
            .ok_or_else(|| StorageError::PathError("Invalid UTF-8 in database path".into()))?;

        let connection = lancedb::connect(uri).execute().await?;

        Ok(Self {
            connection,
            db_path,
        })
    }

    /// Returns the database directory path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Returns a reference to the underlying LanceDB connection.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Ensures all required tables exist, creating any that are missing.
    ///
    /// This is idempotent — calling it multiple times is safe.
    pub async fn ensure_tables(&self) -> Result<(), StorageError> {
        let existing = self.connection.table_names().execute().await?;

        if !existing.contains(&TABLE_WINDOWS.to_string()) {
            self.connection
                .create_empty_table(TABLE_WINDOWS, Arc::new(windows_schema()))
                .execute()
                .await?;
        }

        if !existing.contains(&TABLE_PAGES.to_string()) {
            self.connection
                .create_empty_table(TABLE_PAGES, Arc::new(pages_schema()))
                .execute()
                .await?;
        }

        if !existing.contains(&TABLE_JOBS.to_string()) {
            self.connection
                .create_empty_table(TABLE_JOBS, Arc::new(jobs_schema()))
                .execute()
                .await?;
        }

        Ok(())
    }

    /// Opens an existing table by name.
    pub async fn open_table(&self, name: &str) -> Result<lancedb::Table, StorageError> {
        let table = self.connection.open_table(name).execute().await?;
        Ok(table)
    }
}

/// Initialize the global storage singleton.
///
/// Call this once during app startup with the app data directory.
/// Subsequent calls to [`get_storage`] will return the same instance.
pub async fn init_storage(db_dir: impl AsRef<Path>) -> Result<&'static Storage, StorageError> {
    STORAGE
        .get_or_try_init(|| async {
            let storage = Storage::open(db_dir).await?;
            storage.ensure_tables().await?;
            Ok(storage)
        })
        .await
}

/// Get the global storage instance.
///
/// Panics if [`init_storage`] has not been called yet.
pub fn get_storage() -> &'static Storage {
    STORAGE
        .get()
        .expect("Storage not initialized. Call init_storage() during app startup.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_open_and_ensure_tables() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test_lance_db");

        let storage = Storage::open(&db_path).await.unwrap();
        storage.ensure_tables().await.unwrap();

        // Verify tables exist
        let tables = storage.connection().table_names().execute().await.unwrap();
        assert!(tables.contains(&TABLE_WINDOWS.to_string()));
        assert!(tables.contains(&TABLE_PAGES.to_string()));
        assert!(tables.contains(&TABLE_JOBS.to_string()));
    }

    #[tokio::test]
    async fn test_ensure_tables_idempotent() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test_lance_db_idem");

        let storage = Storage::open(&db_path).await.unwrap();
        storage.ensure_tables().await.unwrap();
        // Calling again should not error
        storage.ensure_tables().await.unwrap();

        let tables = storage.connection().table_names().execute().await.unwrap();
        assert_eq!(tables.len(), 3);
    }
}
