"""Repetition analysis via similarity-core (PyO3)."""

from similarity_core._native import (
    analyze_prose,
    analyze_prose_multi_pass,
    build_scope_manifest,
    default_rf_pass_config,
    load_rf_chapter,
    pass_config_needs_embedder,
    to_export_json,
    validate_analysis_output,
)

__all__ = [
    "analyze_prose",
    "analyze_prose_multi_pass",
    "build_scope_manifest",
    "default_rf_pass_config",
    "load_rf_chapter",
    "pass_config_needs_embedder",
    "to_export_json",
    "validate_analysis_output",
]
