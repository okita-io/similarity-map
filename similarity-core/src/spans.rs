//! Merge overlapping document spans and expand to sentence boundaries.

/// A merged non-overlapping document span built from overlapping windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedSpan {
    pub doc_char_start: u32,
    pub doc_char_end: u32,
    /// Number of sliding windows whose spans contributed to this instance.
    pub member_window_count: u32,
}

/// Group overlapping `(start, end)` document spans into merged instances.
///
/// Sliding windows from stride < phrase length often overlap; this merges any
/// spans that share document text into one instance and counts contributing windows.
pub fn merge_overlapping_spans(spans: &[(u32, u32)]) -> Vec<MergedSpan> {
    if spans.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<(u32, u32)> = spans
        .iter()
        .map(|&(start, end)| (start, end.max(start)))
        .collect();
    sorted.sort_by_key(|span| span.0);

    let mut merged: Vec<MergedSpan> = Vec::new();
    let (mut run_start, mut run_end) = sorted[0];
    let mut run_count = 1u32;

    for &(start, end) in sorted.iter().skip(1) {
        if start >= run_end {
            merged.push(MergedSpan {
                doc_char_start: run_start,
                doc_char_end: run_end,
                member_window_count: run_count,
            });
            run_start = start;
            run_end = end;
            run_count = 1;
        } else {
            run_end = run_end.max(end);
            run_count += 1;
        }
    }

    merged.push(MergedSpan {
        doc_char_start: run_start,
        doc_char_end: run_end,
        member_window_count: run_count,
    });

    merged
}

/// Expand a byte span to the nearest sentence boundaries in `document_text`.
///
/// Sentence ends are `.`, `!`, or `?` followed by whitespace or end-of-text.
/// Paragraph breaks (`\n\n`) also act as hard boundaries when expanding inward
/// from the span edges.
pub fn expand_to_sentence_boundaries(
    document_text: &str,
    start: u32,
    end: u32,
) -> (u32, u32) {
    let len = document_text.len();
    if len == 0 {
        return (0, 0);
    }

    let mut start = start.min(len as u32) as usize;
    let mut end = end.min(len as u32) as usize;
    if end < start {
        end = start;
    }

    start = expand_start_to_boundary(document_text, start);
    end = expand_end_to_boundary(document_text, start, end, len);

    (start as u32, end as u32)
}

fn expand_start_to_boundary(text: &str, start: usize) -> usize {
    if start == 0 {
        return 0;
    }

    let before = &text[..start];

    if let Some(pos) = before.rfind("\n\n") {
        let after_break = pos + 2;
        if after_break < start {
            return skip_leading_whitespace(text, after_break);
        }
    }

    if let Some(sentence_start) = find_sentence_start_after_boundary(before) {
        return sentence_start;
    }

    0
}

fn expand_end_to_boundary(text: &str, start: usize, end: usize, len: usize) -> usize {
    if end >= len {
        return len;
    }

    let anchor = if end > start { end - 1 } else { end };

    if anchor > 0 {
        let prev = text[..=anchor].chars().next_back();
        if prev.is_some_and(is_sentence_terminator) {
            return anchor + 1;
        }
    }

    if let Some(pos) = text[end..].find("\n\n") {
        return end + pos;
    }

    if let Some(sentence_end) = find_sentence_end_from(text, anchor) {
        return sentence_end;
    }

    len
}

/// Find the end byte index (exclusive) of the sentence containing or following `pos`.
fn find_sentence_end_from(text: &str, pos: usize) -> Option<usize> {
    for (byte_idx, ch) in text[pos..].char_indices() {
        if is_sentence_terminator(ch) {
            let abs = pos + byte_idx + ch.len_utf8();
            let rest = text.get(abs..).unwrap_or("");
            if rest.is_empty() || rest.chars().next().is_some_and(|c| c.is_whitespace()) {
                return Some(abs);
            }
        }
    }
    None
}

fn find_sentence_start_after_boundary(before: &str) -> Option<usize> {
    let mut i = before.len();

    while i > 0 {
        let ch = before[..i].chars().next_back()?;
        let ch_len = ch.len_utf8();
        i -= ch_len;

        if is_sentence_terminator(ch) {
            let after = i + ch_len;
            return Some(skip_leading_whitespace(before, after));
        }
    }

    None
}

fn skip_leading_whitespace(text: &str, mut idx: usize) -> usize {
    while idx < text.len() {
        if let Some(ch) = text[idx..].chars().next() {
            if ch.is_whitespace() {
                idx += ch.len_utf8();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    idx
}

fn is_sentence_terminator(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_overlapping_spans_groups_runs() {
        let spans = vec![(0, 100), (50, 150), (80, 180), (300, 400)];
        let merged = merge_overlapping_spans(&spans);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].doc_char_start, 0);
        assert_eq!(merged[0].doc_char_end, 180);
        assert_eq!(merged[0].member_window_count, 3);
        assert_eq!(merged[1].doc_char_start, 300);
        assert_eq!(merged[1].doc_char_end, 400);
        assert_eq!(merged[1].member_window_count, 1);
    }

    #[test]
    fn sentence_boundary_expansion() {
        let text = "First sentence. Second starts here and ends. Third one.";
        let start = "First sentence. ".len() as u32;
        let end = ("First sentence. Second starts here").len() as u32;
        let (exp_start, exp_end) = expand_to_sentence_boundaries(text, start, end);
        assert_eq!(
            &text[exp_start as usize..exp_end as usize],
            "Second starts here and ends."
        );
    }

    #[test]
    fn paragraph_boundary_expansion() {
        let text = "Para one.\n\nPara two has text.";
        let start = ("Para one.\n\nPara two").len() as u32;
        let end = start + 4;
        let (exp_start, exp_end) = expand_to_sentence_boundaries(text, start, end);
        assert_eq!(
            &text[exp_start as usize..exp_end as usize],
            "Para two has text."
        );
    }
}
