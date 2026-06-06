use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;

/// Compute the SHA-256 hash of raw text, returned as a lowercase hex string.
pub fn compute_text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute the SHA-256 hash of a file's contents, returned as a lowercase hex string.
pub fn compute_document_hash(path: &Path) -> Result<String, io::Error> {
    let contents = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&contents);
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// Compute a deterministic SHA-256 hash of analysis parameters, returned as a lowercase hex string.
///
/// Uses a canonical serialization format: parameters are concatenated in a fixed order
/// separated by `|`, with `Option<u32>` represented as "none" or the numeric value.
pub fn compute_settings_hash(
    window_size: u32,
    stride: u32,
    tokens_per_page: Option<u32>,
    min_repetitions: u32,
    min_samples: u32,
    enable_hdbscan: bool,
    link_subphrases: bool,
) -> String {
    let tokens_per_page_str = match tokens_per_page {
        Some(v) => v.to_string(),
        None => "none".to_string(),
    };

    let canonical = format!(
        "window_size={}|stride={}|tokens_per_page={}|min_repetitions={}|min_samples={}|enable_hdbscan={}|link_subphrases={}",
        window_size,
        stride,
        tokens_per_page_str,
        min_repetitions,
        min_samples,
        enable_hdbscan,
        link_subphrases
    );

    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn settings_hash_deterministic_same_params() {
        let h1 = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        let h2 = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        assert_eq!(h1, h2);
    }

    #[test]
    fn settings_hash_different_window_size() {
        let h1 = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        let h2 = compute_settings_hash(25, 5, Some(400), 3, 3, true, false);
        assert_ne!(h1, h2);
    }

    #[test]
    fn settings_hash_different_stride() {
        let h1 = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        let h2 = compute_settings_hash(20, 10, Some(400), 3, 3, true, false);
        assert_ne!(h1, h2);
    }

    #[test]
    fn settings_hash_different_tokens_per_page() {
        let h1 = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        let h2 = compute_settings_hash(20, 5, Some(800), 3, 3, true, false);
        assert_ne!(h1, h2);
    }

    #[test]
    fn settings_hash_none_vs_some_tokens_per_page() {
        let h1 = compute_settings_hash(20, 5, None, 3, 3, true, false);
        let h2 = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        assert_ne!(h1, h2);
    }

    #[test]
    fn settings_hash_different_min_repetitions() {
        let h1 = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        let h2 = compute_settings_hash(20, 5, Some(400), 5, 3, true, false);
        assert_ne!(h1, h2);
    }

    #[test]
    fn settings_hash_different_min_samples() {
        let h1 = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        let h2 = compute_settings_hash(20, 5, Some(400), 3, 5, true, false);
        assert_ne!(h1, h2);
    }

    #[test]
    fn settings_hash_is_valid_sha256_hex() {
        let h = compute_settings_hash(20, 5, Some(400), 3, 3, true, false);
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn document_hash_deterministic_same_content() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "Hello, world! This is a test document.").unwrap();
        file.flush().unwrap();

        let h1 = compute_document_hash(file.path()).unwrap();
        let h2 = compute_document_hash(file.path()).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn document_hash_different_content() {
        let mut file1 = NamedTempFile::new().unwrap();
        write!(file1, "Content A").unwrap();
        file1.flush().unwrap();

        let mut file2 = NamedTempFile::new().unwrap();
        write!(file2, "Content B").unwrap();
        file2.flush().unwrap();

        let h1 = compute_document_hash(file1.path()).unwrap();
        let h2 = compute_document_hash(file2.path()).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn document_hash_is_valid_sha256_hex() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "test content").unwrap();
        file.flush().unwrap();

        let h = compute_document_hash(file.path()).unwrap();
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn document_hash_nonexistent_file_returns_error() {
        let result = compute_document_hash(Path::new("/nonexistent/path/file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn document_hash_empty_file() {
        let file = NamedTempFile::new().unwrap();
        // File is empty by default
        let h = compute_document_hash(file.path()).unwrap();
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
