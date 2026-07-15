pub mod analysis;
pub mod analyze_prose;
pub mod benchmark;
pub mod cancellation;
pub mod centroid;
pub mod clustering;
pub mod color;
pub mod contract;
pub mod embedding;
pub mod hash;
pub mod importer;
pub mod lexical;
pub mod model;
pub mod multi_pass;
pub mod ort_runtime;
pub mod rasterizer;
pub mod report;
pub mod rf_story;
pub mod spans;
pub mod storage;
pub mod subcell;
pub mod types;
pub mod visualization;
pub mod windowing;

pub mod job_data;

pub use analysis::{paginate_text, validate_analysis_params, AnalysisParams, ClusteringArtifacts};
pub use analyze_prose::{
    analyze_prose, analyze_prose_with_model, run_analysis_stages, run_analysis_stages_from_pages,
    run_clustering_stages_from_embeddings, AnalysisArtifacts, AnalysisInput, AnalyzeProseOptions,
    AnalyzeProseResult, DeterministicTestEmbedder, TextEmbedder,
};
pub use importer::{import_document, paginate_scope, ImportDocumentParams};
pub use lexical::{analyze_lexical, LexicalAnalysisStats, LexicalCandidateKind, LexicalPassConfig};
pub use multi_pass::{
    analyze_prose_multi_pass, default_rf_multi_pass_config, estimate_rf_chapter_passes,
    multi_pass_config_for_preset, MultiPassConfig, MultiPassInput, MultiPassResult, PassScope,
    PassSpec, RfChapterPassEstimate, RfChapterPreset, RfPassEstimate,
};
pub use visualization::{
    build_text_highlights, build_visualization_payload, doc_char_to_page,
    load_visualization_payload, load_visualization_payload_with_analysis_output, AnalysisSummary,
    HighlightRole, PageRaster, TextHighlight, VisualizationPayload, DEFAULT_GAMMA,
    DEFAULT_TOLERANCE,
};

pub use contract::{
    assemble_rf_chapter_scope, build_analysis_output, build_analysis_output_with_manifest,
    build_scope_manifest, from_export_json, merge_pass_reports, parse_act_paragraphs,
    repetition_report_to_v1, to_export_json, validate_analysis_output, ActSegment, AnalysisOutput,
    AnalysisPassRecord, ClusterSummaryV1, ContractError, EditSpanV1, ParagraphIndexEntry,
    PassMethod, RepetitionReportV1,
};
pub use report::{
    build_repetition_report, build_repetition_report_from_registry,
    build_repetition_report_with_manifest, derive_cluster_enrichments,
    derive_cluster_enrichments_v1, duplicate_blast_radius_words, format_segment_id,
    load_repetition_report_from_storage, pages_to_document_text, resolve_span_location,
    AnalysisScope, AnalysisStats, ClusterSummary, EditSpan, ParagraphSpan, RepetitionReport,
    ReportAnalysisParams, ScopeManifest, ScopeSegment, SpanLocation, SuggestedOp, BOUNDARY_VERSION,
    DELETE_SPAN_MAX_WORDS, HIGH_SIMILARITY, PARAGRAPH_WORD_THRESHOLD, SCHEMA_VERSION,
};
pub use rf_story::{
    act_draft_paths_for_chapter, build_rf_chapter_scope, chapter_scope_from_manifest,
    list_rf_chapters, load_rf_chapter, RfChapterDraft, RfChapterList, RfChapterScope,
};
pub use spans::{
    expand_to_sentence_boundaries, merge_overlapping_spans, sentence_index_at_char_offset,
    MergedSpan,
};
