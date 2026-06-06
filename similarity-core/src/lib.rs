pub mod analysis;
pub mod benchmark;
pub mod cancellation;
pub mod centroid;
pub mod clustering;
pub mod color;
pub mod contract;
pub mod embedding;
pub mod hash;
pub mod importer;
pub mod model;
pub mod ort_runtime;
pub mod rasterizer;
pub mod report;
pub mod spans;
pub mod storage;
pub mod subcell;
pub mod types;
pub mod visualization;
pub mod windowing;

pub mod job_data;

pub use analysis::{paginate_text, validate_analysis_params, AnalysisParams, ClusteringArtifacts};
pub use importer::{import_document, ImportDocumentParams};
pub use visualization::{
    build_text_highlights, build_visualization_payload, doc_char_to_page,
    load_visualization_payload, AnalysisSummary, HighlightRole, PageRaster, TextHighlight,
    VisualizationPayload, DEFAULT_GAMMA, DEFAULT_TOLERANCE,
};

pub use contract::{
    build_analysis_output, build_scope_manifest, from_export_json, merge_pass_reports,
    repetition_report_to_v1, to_export_json, validate_analysis_output, ActSegment,
    AnalysisOutput, AnalysisPassRecord, ClusterSummaryV1, ContractError, EditSpanV1,
    ParagraphIndexEntry, RepetitionReportV1,
};
pub use report::{
    build_repetition_report, build_repetition_report_from_registry,
    derive_cluster_enrichments, format_segment_id, load_repetition_report_from_storage,
    pages_to_document_text, AnalysisScope, AnalysisStats, ClusterSummary, EditSpan,
    ParagraphSpan, RepetitionReport, ReportAnalysisParams, ScopeManifest, ScopeSegment,
    SpanLocation, SuggestedOp, SCHEMA_VERSION,
};
pub use spans::{expand_to_sentence_boundaries, merge_overlapping_spans, MergedSpan};
