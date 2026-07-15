"""Offline tests for similarity_core PyO3 bindings (no ONNX required)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

similarity_core = pytest.importorskip("similarity_core")

SCHEMA_PATH = (
    Path(__file__).resolve().parents[2]
    / "similarity-core"
    / "schemas"
    / "analysis_output_v1.schema.json"
)


def _sample_text() -> str:
    phrase = "alpha beta gamma delta epsilon alpha beta gamma delta epsilon"
    return "\n\n".join([phrase] * 3)


def _scope_manifest(text: str) -> dict:
    """Minimal manifest matching build_scope_manifest for a single-act chapter."""
    return {
        "chapter": 1,
        "acts": [
            {
                "act": 1,
                "scope_char_start": 0,
                "scope_char_end": len(text),
                "doc_char_start": 0,
                "doc_char_end": len(text),
                "paragraphs": [
                    {
                        "paragraph_index": 1,
                        "segment_id": "ch01_a01_p01",
                        "scope_char_start": 0,
                        "scope_char_end": len(text),
                        "doc_char_start": 0,
                        "doc_char_end": len(text),
                    }
                ],
            }
        ],
    }


def test_analyze_prose_returns_v1_dict():
    text = _sample_text()
    params = {
        "window_size": 5,
        "stride": 5,
        "min_repetitions": 2,
        "min_samples": 2,
        "enable_hdbscan": False,
        "link_subphrases": False,
    }
    result = similarity_core.analyze_prose(
        text,
        _scope_manifest(text),
        params,
        test_embedder=True,
    )
    assert isinstance(result, dict)
    assert result["schema_version"] == "1"
    assert result["passes"]
    assert result["merged_repetition_report"]["stats"]["cluster_count"] >= 1


@pytest.mark.skipif(not SCHEMA_PATH.is_file(), reason="schema fixture missing")
def test_analyze_prose_matches_json_schema():
    jsonschema = pytest.importorskip("jsonschema")
    text = _sample_text()
    result = similarity_core.analyze_prose(
        text,
        _scope_manifest(text),
        {
            "window_size": 5,
            "stride": 5,
            "min_repetitions": 2,
            "min_samples": 2,
            "enable_hdbscan": False,
        },
        test_embedder=True,
    )
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.validate(instance=result, schema=schema)


def test_analyze_prose_multi_pass_merges_passes():
    text = _sample_text()
    pass_config = {
        "min_repetitions": 2,
        "min_samples": 2,
        "enable_hdbscan": False,
        "link_subphrases": False,
        "expand_to_sentences": True,
        "tokens_per_page": 400,
        "passes": [
            {
                "name": "chapter_5_5",
                "scope": "chapter",
                "window_size": 5,
                "stride": 5,
            }
        ],
    }
    result = similarity_core.analyze_prose_multi_pass(
        text,
        _scope_manifest(text),
        pass_config,
        test_embedder=True,
    )
    assert result["schema_version"] == "1"
    assert len(result["passes"]) == 1
    assert "merged_repetition_report" in result


def test_lexical_only_multi_pass_without_onnx():
    sentence = (
        "I've found it, she declared, her voice echoing through the vast chamber "
        "like a prophecy fulfilled and ancient drums."
    )
    text = f"{sentence}\n\nBridge keeps copies apart.\n\n{sentence}"
    pass_config = {
        "min_repetitions": 2,
        "min_samples": 2,
        "enable_hdbscan": False,
        "passes": [
            {
                "name": "chapter_lexical",
                "scope": "chapter",
                "method": "lexical",
            }
        ],
    }
    assert similarity_core.pass_config_needs_embedder(pass_config) is False
    result = similarity_core.analyze_prose_multi_pass(
        text,
        similarity_core.build_scope_manifest(text, chapter=1),
        pass_config,
        test_embedder=False,
    )
    assert result["schema_version"] == "1"
    assert result["passes"][0]["pass_id"] == "chapter_lexical"
    assert result["passes"][0].get("method", "lexical") in (None, "lexical")
    assert result["merged_repetition_report"]["stats"]["cluster_count"] >= 1
    assert similarity_core.validate_analysis_output(result) is True


def test_default_rf_pass_config_has_lexical_primary():
    cfg = similarity_core.default_rf_pass_config()
    assert cfg["min_repetitions"] == 2
    assert cfg["passes"][0]["name"] == "chapter_lexical"
    assert cfg["passes"][0]["method"] == "lexical"
