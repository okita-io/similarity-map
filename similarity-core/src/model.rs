use std::path::{Path, PathBuf};

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::types::{AppError, ModelError};

const HF_BASE: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx";

/// Quantized ONNX (~23 MB) — matches design spec; full `model.onnx` is ~90 MB.
#[cfg(target_arch = "aarch64")]
const MODEL_REMOTE_NAME: &str = "model_qint8_arm64.onnx";

#[cfg(not(target_arch = "aarch64"))]
const MODEL_REMOTE_NAME: &str = "model_quint8_avx2.onnx";

fn model_download_url() -> String {
    format!("{HF_BASE}/{MODEL_REMOTE_NAME}")
}

/// Expected SHA-256 hash of the cached model (optional integrity check).
const MODEL_SHA256: &str = "";

/// Quantized ONNX is ~23 MB; reject partial or wrong artifacts outside this band.
const MIN_MODEL_SIZE: u64 = 20_000_000;
const MAX_MODEL_SIZE: u64 = 35_000_000;

/// Maximum number of download retry attempts.
const MAX_RETRIES: u32 = 3;

/// Subdirectory within app data for model storage.
const MODELS_DIR: &str = "models";

/// Model filename on disk.
const MODEL_FILENAME: &str = "all-MiniLM-L6-v2.onnx";

/// Returns the expected model file path within the given app data directory.
pub fn model_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(MODELS_DIR).join(MODEL_FILENAME)
}

/// Verify that a model file exists and is the expected quantized size (~23 MB).
///
/// This is a fast check that avoids computing the full SHA-256 hash on every startup.
/// A full hash verification can be done separately if needed.
pub fn verify_model(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let len = meta.len();
            meta.is_file() && len >= MIN_MODEL_SIZE && len <= MAX_MODEL_SIZE
        }
        Err(_) => false,
    }
}

/// Compute the SHA-256 hash of a file and compare it to the expected hash.
///
/// Returns `true` if the hash matches, `false` otherwise.
pub fn verify_model_hash(path: &Path) -> bool {
    if MODEL_SHA256.is_empty() {
        return false;
    }
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let result = hasher.finalize();
    let hex = format!("{:x}", result);
    hex == MODEL_SHA256
}

/// Ensure the embedding model is present and valid.
///
/// If the model file exists and passes size verification, returns its path.
/// If the model is missing or corrupt, downloads it from Hugging Face.
///
/// The `progress_callback` receives (percentage 0.0-1.0, bytes_received, total_bytes).
pub async fn ensure_model(
    app_data_dir: &Path,
    progress_callback: impl Fn(f32, u64, u64) + Send + 'static,
) -> Result<PathBuf, AppError> {
    let target = model_path(app_data_dir);

    // Check if model already exists and is valid
    if verify_model(&target) {
        return Ok(target);
    }

    // If file exists but is invalid (too small / corrupt), delete it
    if target.exists() {
        let _ = std::fs::remove_file(&target);
    }

    // Ensure the models directory exists
    let models_dir = app_data_dir.join(MODELS_DIR);
    std::fs::create_dir_all(&models_dir).map_err(|e| {
        AppError::Model(ModelError {
            message: format!("Failed to create models directory: {}", e),
            recoverable: true,
        })
    })?;

    // Download with retry
    download_model_with_retry(&target, progress_callback).await?;

    // Verify the downloaded file
    if !verify_model(&target) {
        // Clean up the bad file
        let _ = std::fs::remove_file(&target);
        return Err(AppError::Model(ModelError {
            message: "Downloaded model file is invalid (too small or corrupt)".to_string(),
            recoverable: true,
        }));
    }

    Ok(target)
}

/// Download the model with retry logic.
async fn download_model_with_retry(
    target_path: &Path,
    progress_callback: impl Fn(f32, u64, u64) + Send + 'static,
) -> Result<(), AppError> {
    let mut last_error = String::new();

    for attempt in 1..=MAX_RETRIES {
        match download_model(target_path, &progress_callback).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_error = format!("{}", e);
                // Clean up partial download
                let _ = std::fs::remove_file(target_path);

                if attempt < MAX_RETRIES {
                    // Brief delay before retry (exponential backoff)
                    let delay = std::time::Duration::from_secs(2u64.pow(attempt - 1));
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(AppError::Model(ModelError {
        message: format!(
            "Model download failed after {} attempts. Last error: {}",
            MAX_RETRIES, last_error
        ),
        recoverable: true,
    }))
}

/// Download the ONNX model from Hugging Face with streaming progress.
///
/// The `progress_callback` receives (percentage 0.0-1.0, bytes_received, total_bytes).
fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .user_agent(concat!("similarity-map/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::default())
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| {
            AppError::Model(ModelError {
                message: format!("Failed to create HTTP client: {}", e),
                recoverable: true,
            })
        })
}

pub async fn download_model(
    target_path: &Path,
    progress_callback: &(impl Fn(f32, u64, u64) + Send),
) -> Result<(), AppError> {
    let client = http_client()?;
    let url = model_download_url();

    let response = client.get(&url).send().await.map_err(|e| {
        AppError::Model(ModelError {
            message: format!("Failed to connect to model server: {}", e),
            recoverable: true,
        })
    })?;

    if !response.status().is_success() {
        return Err(AppError::Model(ModelError {
            message: format!(
                "Model download failed with HTTP status: {}",
                response.status()
            ),
            recoverable: true,
        }));
    }

    let total_bytes = response.content_length().unwrap_or(0);
    let mut bytes_received: u64 = 0;

    // Write to a temporary file first, then rename for atomicity
    let tmp_path = target_path.with_extension("onnx.tmp");

    let mut file = tokio::fs::File::create(&tmp_path).await.map_err(|e| {
        AppError::Model(ModelError {
            message: format!("Failed to create temporary file: {}", e),
            recoverable: true,
        })
    })?;

    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            AppError::Model(ModelError {
                message: format!("Download interrupted: {}", e),
                recoverable: true,
            })
        })?;

        file.write_all(&chunk).await.map_err(|e| {
            AppError::Model(ModelError {
                message: format!("Failed to write model data: {}", e),
                recoverable: true,
            })
        })?;

        bytes_received += chunk.len() as u64;

        let pct = if total_bytes > 0 {
            bytes_received as f32 / total_bytes as f32
        } else {
            0.0
        };

        progress_callback(pct, bytes_received, total_bytes);
    }

    file.flush().await.map_err(|e| {
        AppError::Model(ModelError {
            message: format!("Failed to flush model file: {}", e),
            recoverable: true,
        })
    })?;

    drop(file);

    // Atomically move temp file to final location
    tokio::fs::rename(&tmp_path, target_path)
        .await
        .map_err(|e| {
            AppError::Model(ModelError {
                message: format!("Failed to finalize model file: {}", e),
                recoverable: true,
            })
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_model_path_construction() {
        let app_data = Path::new("/home/user/.local/share/similarity-map");
        let path = model_path(app_data);
        assert_eq!(
            path,
            PathBuf::from("/home/user/.local/share/similarity-map/models/all-MiniLM-L6-v2.onnx")
        );
    }

    #[test]
    fn test_model_path_construction_macos() {
        let app_data = Path::new("/Users/user/Library/Application Support/similarity-map");
        let path = model_path(app_data);
        assert_eq!(
            path,
            PathBuf::from(
                "/Users/user/Library/Application Support/similarity-map/models/all-MiniLM-L6-v2.onnx"
            )
        );
    }

    #[test]
    fn test_verify_model_missing_file() {
        let path = Path::new("/nonexistent/path/model.onnx");
        assert!(!verify_model(path));
    }

    #[test]
    fn test_verify_model_too_small() {
        let dir = TempDir::new().unwrap();
        let model_file = dir.path().join("model.onnx");
        // Write a file smaller than MIN_MODEL_SIZE
        std::fs::write(&model_file, b"too small").unwrap();
        assert!(!verify_model(&model_file));
    }

    #[test]
    fn test_verify_model_valid_size() {
        let dir = TempDir::new().unwrap();
        let model_file = dir.path().join("model.onnx");
        let data = vec![0u8; (MIN_MODEL_SIZE + 1_000_000) as usize];
        std::fs::write(&model_file, &data).unwrap();
        assert!(verify_model(&model_file));
    }

    #[test]
    fn test_verify_model_hash_missing_file() {
        let path = Path::new("/nonexistent/path/model.onnx");
        assert!(!verify_model_hash(path));
    }

    #[tokio::test]
    async fn test_ensure_model_returns_existing_valid_model() {
        let dir = TempDir::new().unwrap();
        let models_dir = dir.path().join(MODELS_DIR);
        std::fs::create_dir_all(&models_dir).unwrap();

        let model_file = models_dir.join(MODEL_FILENAME);
        let data = vec![0u8; (MIN_MODEL_SIZE + 1_000_000) as usize];
        std::fs::write(&model_file, &data).unwrap();

        let result = ensure_model(dir.path(), |_, _, _| {}).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn test_constants() {
        assert!(!model_download_url().is_empty());
        assert!(model_download_url().contains(MODEL_REMOTE_NAME));
        assert!(MIN_MODEL_SIZE > 0);
        assert!(MAX_RETRIES >= 1);
    }

    /// Requires network. Run with: cargo test test_download_quantized_model -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_download_quantized_model() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("downloaded.onnx");
        download_model(&target, &|pct, received, total| {
            eprintln!("{:.0}% {} / {}", pct * 100.0, received, total);
        })
        .await
        .expect("download from Hugging Face");
        assert!(verify_model(&target));
    }
}
