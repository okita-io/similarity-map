//! Arrow schema definitions for LanceDB tables.
//!
//! These schemas define the column structure for the `windows`, `pages`, and `jobs` tables.

use arrow_schema::{DataType, Field, Schema};
use std::sync::Arc;

/// Embedding vector dimensionality (all-MiniLM-L6-v2 produces 384-dim vectors).
pub const EMBEDDING_DIM: i32 = 384;

/// Schema for the `windows` table.
///
/// Each row represents a single text window — the atomic unit of comparison.
///
/// | Field | Type | Description |
/// |---|---|---|
/// | window_id | Utf8 | Unique window identifier (UUID) |
/// | job_id | Utf8 | Parent analysis job (UUID) |
/// | window_index | UInt32 | Sequential index within job (0-based) |
/// | page | UInt32 | 1-based page number |
/// | char_start | UInt32 | Character offset from start of page text |
/// | char_end | UInt32 | End of window in page text (exclusive) |
/// | doc_char_start | UInt32 | Character offset from start of full document |
/// | text | Utf8 | Raw window text |
/// | embedding | FixedSizeList(Float32, 384) | L2-normalized embedding vector |
/// | cluster_id | Int32 | Stable KMeans cluster label (-1 if noise) |
/// | hdbscan_label | Int32 | HDBSCAN label (-1 = noise) |
/// | sim_to_centroid | Float32 | Cosine similarity to cluster centroid |
/// | sub_cell_row | UInt8 | Pre-computed sub-cell row (0–19) |
/// | sub_cell_col | UInt8 | Pre-computed sub-cell col (0–19) |
pub fn windows_schema() -> Schema {
    Schema::new(vec![
        Field::new("window_id", DataType::Utf8, false),
        Field::new("job_id", DataType::Utf8, false),
        Field::new("window_index", DataType::UInt32, false),
        Field::new("page", DataType::UInt32, false),
        Field::new("char_start", DataType::UInt32, false),
        Field::new("char_end", DataType::UInt32, false),
        Field::new("doc_char_start", DataType::UInt32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBEDDING_DIM,
            ),
            false,
        ),
        Field::new("cluster_id", DataType::Int32, false),
        Field::new("hdbscan_label", DataType::Int32, false),
        Field::new("sim_to_centroid", DataType::Float32, false),
        Field::new("sub_cell_row", DataType::UInt8, false),
        Field::new("sub_cell_col", DataType::UInt8, false),
    ])
}

/// Schema for the `pages` table.
///
/// Each row represents one page of the imported document.
///
/// | Field | Type | Description |
/// |---|---|---|
/// | job_id | Utf8 | Parent analysis job (UUID) |
/// | page | UInt32 | 1-based page number |
/// | doc_char_start | UInt32 | Character offset of page start in full document |
/// | doc_char_end | UInt32 | Character offset of page end in full document |
/// | char_count | UInt32 | Character count for this page |
/// | token_count | UInt32 | Approximate token count |
/// | pagination_mode | Utf8 | "pdf", "token", or "chapter" |
pub fn pages_schema() -> Schema {
    Schema::new(vec![
        Field::new("job_id", DataType::Utf8, false),
        Field::new("page", DataType::UInt32, false),
        Field::new("doc_char_start", DataType::UInt32, false),
        Field::new("doc_char_end", DataType::UInt32, false),
        Field::new("char_count", DataType::UInt32, false),
        Field::new("token_count", DataType::UInt32, false),
        Field::new("pagination_mode", DataType::Utf8, false),
    ])
}

/// Schema for the `jobs` table.
///
/// Each row represents a single analysis run (complete or partial).
///
/// | Field | Type | Description |
/// |---|---|---|
/// | job_id | Utf8 | Unique run identifier (UUID) |
/// | document_path | Utf8 | Absolute path to source file |
/// | document_hash | Utf8 | SHA-256 hash of file contents |
/// | settings_hash | Utf8 | SHA-256 hash of analysis parameters |
/// | window_size | UInt32 | Phrase length used |
/// | stride | UInt32 | Stride used |
/// | tokens_per_page | UInt32 | Tokens per page (0 = PDF natural pages) |
/// | pagination_mode | Utf8 | "pdf", "token", or "chapter" |
/// | min_repetitions | UInt32 | Minimum cluster recurrences |
/// | min_samples | UInt32 | HDBSCAN min_samples |
/// | chapter_break_re | Utf8 | Chapter break regex (empty string if unused) |
/// | windows_total | UInt32 | Total windows planned |
/// | windows_committed | UInt32 | Windows successfully embedded |
/// | status | Utf8 | "running", "partial", "complete", "discarded" |
/// | created_at | Utf8 | ISO 8601 timestamp when analysis started |
/// | updated_at | Utf8 | ISO 8601 timestamp of last batch commit or status change |
pub fn jobs_schema() -> Schema {
    // Note: Optional fields (tokens_per_page, chapter_break_re) are stored as
    // non-nullable with sentinel values (0 for None tokens_per_page, empty string
    // for None chapter_break_re) to simplify LanceDB queries. The application layer
    // handles the conversion.
    Schema::new(vec![
        Field::new("job_id", DataType::Utf8, false),
        Field::new("document_path", DataType::Utf8, false),
        Field::new("document_hash", DataType::Utf8, false),
        Field::new("settings_hash", DataType::Utf8, false),
        Field::new("window_size", DataType::UInt32, false),
        Field::new("stride", DataType::UInt32, false),
        Field::new("tokens_per_page", DataType::UInt32, true),
        Field::new("pagination_mode", DataType::Utf8, false),
        Field::new("min_repetitions", DataType::UInt32, false),
        Field::new("min_samples", DataType::UInt32, false),
        Field::new("chapter_break_re", DataType::Utf8, true),
        Field::new("windows_total", DataType::UInt32, false),
        Field::new("windows_committed", DataType::UInt32, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("created_at", DataType::Utf8, false),
        Field::new("updated_at", DataType::Utf8, false),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_windows_schema_field_count() {
        let schema = windows_schema();
        assert_eq!(schema.fields().len(), 14);
    }

    #[test]
    fn test_windows_schema_embedding_dimension() {
        let schema = windows_schema();
        let embedding_field = schema.field_with_name("embedding").unwrap();
        match embedding_field.data_type() {
            DataType::FixedSizeList(_, dim) => assert_eq!(*dim, EMBEDDING_DIM),
            other => panic!("Expected FixedSizeList, got {:?}", other),
        }
    }

    #[test]
    fn test_pages_schema_field_count() {
        let schema = pages_schema();
        assert_eq!(schema.fields().len(), 7);
    }

    #[test]
    fn test_jobs_schema_field_count() {
        let schema = jobs_schema();
        assert_eq!(schema.fields().len(), 16);
    }

    #[test]
    fn test_jobs_schema_nullable_fields() {
        let schema = jobs_schema();
        let tokens_field = schema.field_with_name("tokens_per_page").unwrap();
        assert!(tokens_field.is_nullable());
        let chapter_field = schema.field_with_name("chapter_break_re").unwrap();
        assert!(chapter_field.is_nullable());
    }

    #[test]
    fn test_windows_schema_has_required_fields() {
        let schema = windows_schema();
        let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert!(field_names.contains(&"window_id"));
        assert!(field_names.contains(&"job_id"));
        assert!(field_names.contains(&"window_index"));
        assert!(field_names.contains(&"page"));
        assert!(field_names.contains(&"char_start"));
        assert!(field_names.contains(&"char_end"));
        assert!(field_names.contains(&"doc_char_start"));
        assert!(field_names.contains(&"text"));
        assert!(field_names.contains(&"embedding"));
        assert!(field_names.contains(&"cluster_id"));
        assert!(field_names.contains(&"hdbscan_label"));
        assert!(field_names.contains(&"sim_to_centroid"));
        assert!(field_names.contains(&"sub_cell_row"));
        assert!(field_names.contains(&"sub_cell_col"));
    }

    #[test]
    fn test_pages_schema_has_required_fields() {
        let schema = pages_schema();
        let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert!(field_names.contains(&"job_id"));
        assert!(field_names.contains(&"page"));
        assert!(field_names.contains(&"doc_char_start"));
        assert!(field_names.contains(&"doc_char_end"));
        assert!(field_names.contains(&"char_count"));
        assert!(field_names.contains(&"token_count"));
        assert!(field_names.contains(&"pagination_mode"));
    }

    #[test]
    fn test_jobs_schema_has_required_fields() {
        let schema = jobs_schema();
        let field_names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert!(field_names.contains(&"job_id"));
        assert!(field_names.contains(&"document_path"));
        assert!(field_names.contains(&"document_hash"));
        assert!(field_names.contains(&"settings_hash"));
        assert!(field_names.contains(&"window_size"));
        assert!(field_names.contains(&"stride"));
        assert!(field_names.contains(&"tokens_per_page"));
        assert!(field_names.contains(&"pagination_mode"));
        assert!(field_names.contains(&"min_repetitions"));
        assert!(field_names.contains(&"min_samples"));
        assert!(field_names.contains(&"chapter_break_re"));
        assert!(field_names.contains(&"windows_total"));
        assert!(field_names.contains(&"windows_committed"));
        assert!(field_names.contains(&"status"));
        assert!(field_names.contains(&"created_at"));
        assert!(field_names.contains(&"updated_at"));
    }
}
