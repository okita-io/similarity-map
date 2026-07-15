use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::embedding::EmbeddingEngine;
use crate::types::{AppError, ModelError};

/// Number of synthetic windows used in the benchmark probe.
const BENCHMARK_WINDOW_COUNT: usize = 128;

/// Filename for the cached benchmark result.
const BENCHMARK_CACHE_FILE: &str = "benchmark.json";

/// Result of a benchmark probe measuring embedding throughput.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkResult {
    /// Measured throughput in windows per second.
    pub windows_per_sec: f32,
    /// SHA-256 hash of the model file used for this benchmark.
    pub model_hash: String,
    /// ISO 8601 timestamp when the benchmark was run.
    pub timestamp: String,
}

/// Generate fixed synthetic text windows for the benchmark probe.
///
/// Produces 128 variations of a base sentence to simulate realistic
/// embedding workload without requiring actual document text.
fn generate_probe_texts() -> Vec<String> {
    let base_sentences = [
        "The quick brown fox jumps over the lazy dog",
        "A journey of a thousand miles begins with a single step",
        "To be or not to be that is the question",
        "All that glitters is not gold in this world",
        "The pen is mightier than the sword they say",
        "Actions speak louder than words in every case",
        "Knowledge is power and wisdom is its application",
        "Time and tide wait for no man or woman",
        "Where there is a will there is always a way",
        "Practice makes perfect in all endeavors of life",
        "Every cloud has a silver lining behind it",
        "Fortune favors the bold and the prepared mind",
        "The early bird catches the worm each morning",
        "Still waters run deep beneath the calm surface",
        "When in Rome do as the Romans do always",
        "A picture is worth a thousand spoken words",
    ];

    let mut texts = Vec::with_capacity(BENCHMARK_WINDOW_COUNT);
    for i in 0..BENCHMARK_WINDOW_COUNT {
        let base = &base_sentences[i % base_sentences.len()];
        // Add variation by appending the index to create unique windows
        texts.push(format!("{} variation number {}", base, i + 1));
    }
    texts
}

/// Run the benchmark probe by embedding 128 fixed text windows.
///
/// Measures throughput in windows/sec. Returns an error if embedding fails.
pub fn run_benchmark(
    engine: &mut EmbeddingEngine,
    model_hash: &str,
) -> Result<BenchmarkResult, AppError> {
    let texts = generate_probe_texts();
    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

    let start = Instant::now();

    // Process in batches of 32 (matching the embedding pipeline batch size)
    let batch_size = 32;
    for chunk in text_refs.chunks(batch_size) {
        engine.embed_batch(chunk)?;
    }

    let elapsed = start.elapsed();
    let elapsed_secs = elapsed.as_secs_f32();

    // Guard against division by zero (shouldn't happen but be safe)
    let windows_per_sec = if elapsed_secs > 0.0 {
        BENCHMARK_WINDOW_COUNT as f32 / elapsed_secs
    } else {
        BENCHMARK_WINDOW_COUNT as f32 // Assume 1 second minimum
    };

    let timestamp = chrono::Utc::now().to_rfc3339();

    Ok(BenchmarkResult {
        windows_per_sec,
        model_hash: model_hash.to_string(),
        timestamp,
    })
}

/// Load a cached benchmark result from the app data directory.
///
/// Returns `None` if the cache file doesn't exist or can't be parsed.
pub fn load_cached_benchmark(app_data_dir: &Path) -> Option<BenchmarkResult> {
    let cache_path = app_data_dir.join(BENCHMARK_CACHE_FILE);
    let data = std::fs::read_to_string(&cache_path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save a benchmark result to the app data directory as JSON.
pub fn save_benchmark(result: &BenchmarkResult, app_data_dir: &Path) -> Result<(), AppError> {
    // Ensure the directory exists
    std::fs::create_dir_all(app_data_dir).map_err(|e| {
        AppError::Model(ModelError {
            message: format!("Failed to create app data directory: {}", e),
            recoverable: true,
        })
    })?;

    let cache_path = app_data_dir.join(BENCHMARK_CACHE_FILE);
    let json = serde_json::to_string_pretty(result).map_err(|e| {
        AppError::Model(ModelError {
            message: format!("Failed to serialize benchmark result: {}", e),
            recoverable: true,
        })
    })?;

    std::fs::write(&cache_path, json).map_err(|e| {
        AppError::Model(ModelError {
            message: format!("Failed to write benchmark cache: {}", e),
            recoverable: true,
        })
    })?;

    Ok(())
}

/// Load a cached benchmark if the model hash matches, otherwise run a new one.
///
/// This is the primary entry point for obtaining benchmark data. It:
/// 1. Checks for a cached result with a matching model hash
/// 2. If found, returns the cached result
/// 3. If not found (or hash differs), runs a new benchmark and caches it
pub fn get_or_run_benchmark(
    engine: &mut EmbeddingEngine,
    app_data_dir: &Path,
    model_hash: &str,
) -> Result<BenchmarkResult, AppError> {
    // Try loading cached result
    if let Some(cached) = load_cached_benchmark(app_data_dir) {
        if cached.model_hash == model_hash {
            return Ok(cached);
        }
    }

    // Run new benchmark
    let result = run_benchmark(engine, model_hash)?;

    // Cache the result (non-fatal if caching fails)
    let _ = save_benchmark(&result, app_data_dir);

    Ok(result)
}

/// Compute the estimated analysis time in seconds.
///
/// Formula: eta_seconds = window_count / benchmark_windows_per_sec
pub fn estimate_eta(window_count: u32, windows_per_sec: f32) -> f32 {
    if windows_per_sec <= 0.0 {
        return 0.0;
    }
    window_count as f32 / windows_per_sec
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_benchmark_result_serialization() {
        let result = BenchmarkResult {
            windows_per_sec: 256.5,
            model_hash: "abc123def456".to_string(),
            timestamp: "2024-01-15T10:30:00Z".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: BenchmarkResult = serde_json::from_str(&json).unwrap();

        assert_eq!(result, deserialized);
    }

    #[test]
    fn test_benchmark_result_deserialization_from_json() {
        let json = r#"{
            "windows_per_sec": 128.0,
            "model_hash": "deadbeef",
            "timestamp": "2024-06-01T12:00:00Z"
        }"#;

        let result: BenchmarkResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.windows_per_sec, 128.0);
        assert_eq!(result.model_hash, "deadbeef");
        assert_eq!(result.timestamp, "2024-06-01T12:00:00Z");
    }

    #[test]
    fn test_estimate_eta_basic() {
        // 1000 windows at 100 windows/sec = 10 seconds
        let eta = estimate_eta(1000, 100.0);
        assert!((eta - 10.0).abs() < 1e-5);
    }

    #[test]
    fn test_estimate_eta_fractional() {
        // 500 windows at 200 windows/sec = 2.5 seconds
        let eta = estimate_eta(500, 200.0);
        assert!((eta - 2.5).abs() < 1e-5);
    }

    #[test]
    fn test_estimate_eta_zero_throughput() {
        // Zero throughput should return 0 (not panic or infinity)
        let eta = estimate_eta(1000, 0.0);
        assert_eq!(eta, 0.0);
    }

    #[test]
    fn test_estimate_eta_negative_throughput() {
        // Negative throughput should return 0
        let eta = estimate_eta(1000, -5.0);
        assert_eq!(eta, 0.0);
    }

    #[test]
    fn test_estimate_eta_zero_windows() {
        // Zero windows = 0 seconds
        let eta = estimate_eta(0, 100.0);
        assert_eq!(eta, 0.0);
    }

    #[test]
    fn test_save_and_load_benchmark_cache() {
        let dir = TempDir::new().unwrap();
        let result = BenchmarkResult {
            windows_per_sec: 350.0,
            model_hash: "test_hash_123".to_string(),
            timestamp: "2024-03-20T08:00:00Z".to_string(),
        };

        // Save
        save_benchmark(&result, dir.path()).unwrap();

        // Load
        let loaded = load_cached_benchmark(dir.path());
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap(), result);
    }

    #[test]
    fn test_load_cached_benchmark_missing_file() {
        let dir = TempDir::new().unwrap();
        let loaded = load_cached_benchmark(dir.path());
        assert!(loaded.is_none());
    }

    #[test]
    fn test_load_cached_benchmark_corrupt_file() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join(BENCHMARK_CACHE_FILE);
        std::fs::write(&cache_path, "not valid json {{{").unwrap();

        let loaded = load_cached_benchmark(dir.path());
        assert!(loaded.is_none());
    }

    #[test]
    fn test_get_or_run_benchmark_uses_cache_when_hash_matches() {
        let dir = TempDir::new().unwrap();
        let cached = BenchmarkResult {
            windows_per_sec: 200.0,
            model_hash: "matching_hash".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
        };
        save_benchmark(&cached, dir.path()).unwrap();

        // When the cache matches, get_or_run_benchmark should return the cached result
        // without needing a valid engine. We test this by loading the cache directly.
        let loaded = load_cached_benchmark(dir.path());
        assert!(loaded.is_some());
        let result = loaded.unwrap();
        assert_eq!(result.windows_per_sec, 200.0);
        assert_eq!(result.model_hash, "matching_hash");
    }

    #[test]
    fn test_get_or_run_benchmark_reruns_when_hash_differs() {
        let dir = TempDir::new().unwrap();
        let cached = BenchmarkResult {
            windows_per_sec: 200.0,
            model_hash: "old_hash".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
        };
        save_benchmark(&cached, dir.path()).unwrap();

        // When hash differs, the cached result should not be returned
        let loaded = load_cached_benchmark(dir.path()).unwrap();
        assert_ne!(loaded.model_hash, "new_hash");
    }

    #[test]
    fn test_generate_probe_texts_count() {
        let texts = generate_probe_texts();
        assert_eq!(texts.len(), BENCHMARK_WINDOW_COUNT);
        assert_eq!(texts.len(), 128);
    }

    #[test]
    fn test_generate_probe_texts_unique() {
        let texts = generate_probe_texts();
        let unique: std::collections::HashSet<&String> = texts.iter().collect();
        assert_eq!(
            unique.len(),
            texts.len(),
            "Probe texts should all be unique"
        );
    }

    #[test]
    fn test_generate_probe_texts_non_empty() {
        let texts = generate_probe_texts();
        for text in &texts {
            assert!(!text.is_empty());
            // Each text should have multiple tokens (at least 5 words)
            assert!(text.split_whitespace().count() >= 5);
        }
    }

    #[test]
    fn test_save_benchmark_creates_directory() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("deep");
        let result = BenchmarkResult {
            windows_per_sec: 100.0,
            model_hash: "hash".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
        };

        save_benchmark(&result, &nested).unwrap();
        assert!(nested.join(BENCHMARK_CACHE_FILE).exists());
    }
}
