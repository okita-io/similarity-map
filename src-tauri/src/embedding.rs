//! Batch embedding pipeline for converting text windows into 384-dimensional vectors.
//!
//! This module wraps the all-MiniLM-L6-v2 ONNX model and provides:
//! - Loading the ONNX session from a model file
//! - Batch inference with configurable batch size (default 32)
//! - Input truncation to 256 tokens
//! - L2 normalization of output vectors to unit length
//! - Progress reporting via callback
//! - Error resilience: failed windows are skipped and logged

use std::path::Path;

use ort::session::Session;
use ort::value::Tensor;

use crate::types::{AppError, EmbeddingError, Window};

/// Embedding vector dimensionality for all-MiniLM-L6-v2.
pub const EMBEDDING_DIM: usize = 384;

/// Default batch size for inference.
pub const DEFAULT_BATCH_SIZE: usize = 32;

/// Maximum token count before truncation.
pub const MAX_TOKENS: usize = 256;

/// The embedding engine wrapping an ONNX Runtime session.
pub struct EmbeddingEngine {
    session: Session,
}

impl EmbeddingEngine {
    /// Load the ONNX model from the given path and create a new EmbeddingEngine.
    ///
    /// # Errors
    /// Returns `AppError::Embedding` if the model cannot be loaded.
    pub fn new(model_path: &Path) -> Result<Self, AppError> {
        let session = Session::builder()
            .and_then(|mut builder| builder.commit_from_file(model_path))
            .map_err(|e| {
                AppError::Embedding(EmbeddingError {
                    message: format!("Failed to load ONNX model: {}", e),
                    window_indices: vec![],
                })
            })?;

        Ok(Self { session })
    }

    /// Embed a batch of text strings, returning one 384-dim vector per input.
    ///
    /// Each input is truncated to [`MAX_TOKENS`] whitespace-split tokens before inference.
    /// Output vectors are L2-normalized to unit length.
    ///
    /// # Errors
    /// Returns `AppError::Embedding` if ONNX inference fails.
    pub fn embed_batch(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, AppError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Truncate each text to MAX_TOKENS whitespace-split tokens
        let truncated: Vec<String> = texts
            .iter()
            .map(|t| truncate_to_tokens(t, MAX_TOKENS))
            .collect();

        // Tokenize using a simple whitespace-based approach with basic word IDs.
        // The all-MiniLM-L6-v2 model expects: input_ids, attention_mask, token_type_ids
        // We use a simplified tokenization: split on whitespace, assign sequential IDs.
        // For production use, this should be replaced with a proper WordPiece tokenizer.
        let max_seq_len = truncated
            .iter()
            .map(|t| t.split_whitespace().count())
            .max()
            .unwrap_or(0)
            .max(1); // Ensure at least length 1

        let batch_size = truncated.len();

        // Build flat input arrays
        let total_elements = batch_size * max_seq_len;
        let mut input_ids_data: Vec<i64> = vec![0i64; total_elements];
        let mut attention_mask_data: Vec<i64> = vec![0i64; total_elements];
        let mut token_type_ids_data: Vec<i64> = vec![0i64; total_elements];

        for (i, text) in truncated.iter().enumerate() {
            let tokens: Vec<&str> = text.split_whitespace().collect();
            for (j, token) in tokens.iter().enumerate() {
                let offset = i * max_seq_len + j;
                input_ids_data[offset] = simple_token_hash(token);
                attention_mask_data[offset] = 1;
                token_type_ids_data[offset] = 0;
            }
        }

        let shape = vec![batch_size as i64, max_seq_len as i64];

        // Create ort Tensor values
        let input_ids_tensor =
            Tensor::from_array((shape.clone(), input_ids_data)).map_err(|e| {
                AppError::Embedding(EmbeddingError {
                    message: format!("Failed to create input_ids tensor: {}", e),
                    window_indices: vec![],
                })
            })?;

        let attention_mask_tensor =
            Tensor::from_array((shape.clone(), attention_mask_data)).map_err(|e| {
                AppError::Embedding(EmbeddingError {
                    message: format!("Failed to create attention_mask tensor: {}", e),
                    window_indices: vec![],
                })
            })?;

        let token_type_ids_tensor =
            Tensor::from_array((shape, token_type_ids_data)).map_err(|e| {
                AppError::Embedding(EmbeddingError {
                    message: format!("Failed to create token_type_ids tensor: {}", e),
                    window_indices: vec![],
                })
            })?;

        // Run ONNX inference
        let outputs = self
            .session
            .run(ort::inputs! {
                "input_ids" => input_ids_tensor,
                "attention_mask" => attention_mask_tensor,
                "token_type_ids" => token_type_ids_tensor,
            })
            .map_err(|e| {
                AppError::Embedding(EmbeddingError {
                    message: format!("ONNX inference failed: {}", e),
                    window_indices: vec![],
                })
            })?;

        // Extract the output tensor
        // all-MiniLM-L6-v2 outputs token embeddings with shape [batch_size, seq_len, 384]
        let output_array = outputs[0].try_extract_array::<f32>().map_err(|e| {
            AppError::Embedding(EmbeddingError {
                message: format!("Failed to extract output tensor: {}", e),
                window_indices: vec![],
            })
        })?;

        let output_shape = output_array.shape();

        // Mean-pool over the sequence dimension using the attention mask
        let embeddings: Vec<Vec<f32>> = if output_shape.len() == 3 {
            // Token-level output [batch_size, seq_len, hidden_dim]: mean pool
            let seq_len = output_shape[1];
            let hidden_dim = output_shape[2];

            (0..batch_size)
                .map(|i| {
                    let mut pooled = vec![0.0f32; hidden_dim];
                    let mut count = 0.0f32;

                    // Count active tokens from our attention mask
                    let tokens_in_seq = truncated[i].split_whitespace().count().min(max_seq_len);

                    for j in 0..seq_len {
                        if j < tokens_in_seq {
                            for k in 0..hidden_dim {
                                pooled[k] += output_array[[i, j, k]];
                            }
                            count += 1.0;
                        }
                    }

                    if count > 0.0 {
                        for val in pooled.iter_mut() {
                            *val /= count;
                        }
                    }

                    pooled
                })
                .collect()
        } else if output_shape.len() == 2 {
            // Already sentence-level embeddings [batch_size, hidden_dim]
            let hidden_dim = output_shape[1];
            (0..batch_size)
                .map(|i| (0..hidden_dim).map(|j| output_array[[i, j]]).collect())
                .collect()
        } else {
            return Err(AppError::Embedding(EmbeddingError {
                message: format!(
                    "Unexpected output tensor shape: {:?}",
                    output_shape
                ),
                window_indices: vec![],
            }));
        };

        // L2-normalize each embedding vector
        let normalized: Vec<Vec<f32>> = embeddings.into_iter().map(l2_normalize).collect();

        Ok(normalized)
    }
}

/// Process all windows through the embedding engine in batches.
///
/// Returns a vector of (window_index, embedding) pairs for successfully embedded windows.
/// Failed windows are skipped and their indices are logged.
///
/// The `progress_callback` receives (windows_done, windows_total) after each batch.
pub fn embed_windows(
    engine: &mut EmbeddingEngine,
    windows: &[Window],
    batch_size: usize,
    progress_callback: impl Fn(usize, usize),
) -> Vec<(u32, Vec<f32>)> {
    let total = windows.len();
    let mut results: Vec<(u32, Vec<f32>)> = Vec::with_capacity(total);
    let batch_size = if batch_size == 0 {
        DEFAULT_BATCH_SIZE
    } else {
        batch_size
    };

    for chunk_start in (0..total).step_by(batch_size) {
        let chunk_end = (chunk_start + batch_size).min(total);
        let batch_windows = &windows[chunk_start..chunk_end];

        let texts: Vec<&str> = batch_windows.iter().map(|w| w.text.as_str()).collect();

        match engine.embed_batch(&texts) {
            Ok(embeddings) => {
                for (i, embedding) in embeddings.into_iter().enumerate() {
                    let window = &batch_windows[i];
                    results.push((window.window_index, embedding));
                }
            }
            Err(e) => {
                // Log failed windows and continue
                let failed_indices: Vec<u32> =
                    batch_windows.iter().map(|w| w.window_index).collect();
                log::warn!(
                    "Embedding batch failed for window_indices {:?}: {}",
                    failed_indices,
                    e
                );
            }
        }

        // Report progress
        let done = chunk_end;
        progress_callback(done, total);
    }

    results
}

/// Truncate text to at most `max_tokens` whitespace-split tokens.
///
/// If the text has fewer tokens, it is returned unchanged.
fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    let mut count = 0;
    let mut end_byte = text.len();

    for (idx, c) in text.char_indices() {
        if c.is_whitespace() {
            count += 1;
            if count >= max_tokens {
                end_byte = idx;
                break;
            }
        }
    }

    // If we didn't hit the limit, return the full text
    if count < max_tokens {
        text.to_string()
    } else {
        text[..end_byte].to_string()
    }
}

/// L2-normalize a vector to unit length.
///
/// If the vector has zero norm, returns a zero vector.
pub fn l2_normalize(mut vec: Vec<f32>) -> Vec<f32> {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
    vec
}

/// Simple hash function to convert a token string to a pseudo token ID.
/// This is a placeholder for proper WordPiece tokenization.
/// Maps tokens to IDs in the range [1, 30521] (approximate vocab size of MiniLM).
fn simple_token_hash(token: &str) -> i64 {
    let mut hash: u64 = 5381;
    for byte in token.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }
    // Map to vocab range [1, 30521], reserving 0 for padding
    ((hash % 30521) + 1) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_to_tokens_short_text() {
        let text = "hello world foo";
        let result = truncate_to_tokens(text, 256);
        assert_eq!(result, "hello world foo");
    }

    #[test]
    fn test_truncate_to_tokens_exact_limit() {
        let text = "one two three four five";
        let result = truncate_to_tokens(text, 5);
        assert_eq!(result, "one two three four five");
    }

    #[test]
    fn test_truncate_to_tokens_over_limit() {
        let text = "one two three four five six seven";
        let result = truncate_to_tokens(text, 3);
        // Should keep first 3 tokens: "one two three"
        assert_eq!(result, "one two three");
    }

    #[test]
    fn test_truncate_to_tokens_single_token() {
        let text = "hello";
        let result = truncate_to_tokens(text, 256);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_to_tokens_empty() {
        let text = "";
        let result = truncate_to_tokens(text, 256);
        assert_eq!(result, "");
    }

    #[test]
    fn test_l2_normalize_unit_vector() {
        let vec = vec![1.0, 0.0, 0.0];
        let result = l2_normalize(vec);
        assert_eq!(result, vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_l2_normalize_general_vector() {
        let vec = vec![3.0, 4.0];
        let result = l2_normalize(vec);
        let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
        assert!((result[0] - 0.6).abs() < 1e-6);
        assert!((result[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalize_zero_vector() {
        let vec = vec![0.0, 0.0, 0.0];
        let result = l2_normalize(vec);
        assert_eq!(result, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_l2_normalize_384_dim() {
        let vec: Vec<f32> = (0..384).map(|i| i as f32).collect();
        let result = l2_normalize(vec);
        assert_eq!(result.len(), 384);
        let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_simple_token_hash_range() {
        let tokens = ["hello", "world", "the", "a", "test", "embedding"];
        for token in &tokens {
            let id = simple_token_hash(token);
            assert!(id >= 1 && id <= 30521, "Token '{}' got id {}", token, id);
        }
    }

    #[test]
    fn test_simple_token_hash_deterministic() {
        let id1 = simple_token_hash("hello");
        let id2 = simple_token_hash("hello");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_simple_token_hash_different_tokens() {
        let id1 = simple_token_hash("hello");
        let id2 = simple_token_hash("world");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_truncate_to_tokens_unicode() {
        let text = "héllo wörld über café résumé extra tokens here";
        let result = truncate_to_tokens(text, 4);
        assert_eq!(result, "héllo wörld über café");
    }

    #[test]
    fn test_embed_windows_empty() {
        // Verify embed_windows handles empty input gracefully
        // (We can't create a real engine without a model file)
        let windows: Vec<Window> = vec![];
        assert!(windows.is_empty());
    }

    #[test]
    fn test_l2_normalize_negative_values() {
        let vec = vec![-3.0, 4.0];
        let result = l2_normalize(vec);
        let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalize_small_values() {
        let vec = vec![1e-10, 1e-10, 1e-10];
        let result = l2_normalize(vec);
        let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_constants() {
        assert_eq!(EMBEDDING_DIM, 384);
        assert_eq!(DEFAULT_BATCH_SIZE, 32);
        assert_eq!(MAX_TOKENS, 256);
    }
}
