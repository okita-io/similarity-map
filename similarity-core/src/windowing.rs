use uuid::Uuid;

use crate::types::{Page, Window};

/// Generate overlapping text windows from a set of pages.
///
/// Windows slide across each page with the given `window_size` (in tokens) and `stride`.
/// Tokens are defined by whitespace splitting. Windows never cross page boundaries.
/// Terminal windows (remaining text at end of page) are included if they contain ≥ 3 tokens;
/// segments with < 3 tokens are discarded. Pages with fewer than 3 tokens are skipped entirely.
/// `window_index` is sequential zero-based across the entire job.
pub fn generate_windows(pages: &[Page], window_size: u32, stride: u32) -> Vec<Window> {
    let mut windows = Vec::new();
    let mut global_index: u32 = 0;

    for page in pages {
        // Skip pages with fewer than 3 tokens
        if page.token_count < 3 {
            continue;
        }

        let text = &page.text;
        // Collect token byte-offset spans: (start, end) where end is exclusive
        let token_spans: Vec<(usize, usize)> = token_byte_spans(text);

        // Double-check actual token count
        if token_spans.len() < 3 {
            continue;
        }

        let num_tokens = token_spans.len();
        let ws = window_size as usize;
        let st = stride as usize;

        let mut token_offset: usize = 0;

        loop {
            let remaining = num_tokens - token_offset;

            if remaining >= ws {
                // Full window
                let char_start = token_spans[token_offset].0 as u32;
                let char_end = token_spans[token_offset + ws - 1].1 as u32;
                let window_text = text[char_start as usize..char_end as usize].to_string();

                windows.push(Window {
                    window_id: Uuid::new_v4().to_string(),
                    window_index: global_index,
                    page: page.page_num,
                    char_start,
                    char_end,
                    doc_char_start: page.char_offset_in_doc + char_start,
                    text: window_text,
                });
                global_index += 1;
                token_offset += st;
            } else {
                // Terminal segment: include if ≥ 3 tokens, discard otherwise
                if remaining >= 3 {
                    let char_start = token_spans[token_offset].0 as u32;
                    let char_end = token_spans[num_tokens - 1].1 as u32;
                    let window_text = text[char_start as usize..char_end as usize].to_string();

                    windows.push(Window {
                        window_id: Uuid::new_v4().to_string(),
                        window_index: global_index,
                        page: page.page_num,
                        char_start,
                        char_end,
                        doc_char_start: page.char_offset_in_doc + char_start,
                        text: window_text,
                    });
                    global_index += 1;
                }
                break;
            }
        }
    }

    windows
}

/// Estimate the number of windows that will be generated for a given token count.
///
/// Uses the formula: `floor((total_tokens - window_size) / stride) + 1`
/// When `total_tokens <= window_size`, returns 1 if there are at least 3 tokens
/// (enough for a valid terminal window), otherwise returns 0.
pub fn estimate_window_count(total_tokens: u32, window_size: u32, stride: u32) -> u32 {
    if total_tokens <= window_size {
        return if total_tokens >= 3 { 1 } else { 0 };
    }
    ((total_tokens - window_size) / stride) + 1
}

/// Returns byte-offset spans (start, end) for each whitespace-delimited token in the text.
/// `end` is exclusive (one past the last byte of the token).
fn token_byte_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }
        let start = i;
        // Consume non-whitespace (handle multi-byte UTF-8 correctly by iterating bytes
        // that are not ASCII whitespace — this works because ASCII whitespace bytes
        // never appear as continuation bytes in valid UTF-8)
        while i < len && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        spans.push((start, i));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PaginationMode;

    /// Helper to create a Page for testing.
    fn make_page(page_num: u32, text: &str, char_offset_in_doc: u32) -> Page {
        let token_count = text.split_whitespace().count() as u32;
        let char_count = text.len() as u32;
        Page {
            page_num,
            text: text.to_string(),
            char_offset_in_doc,
            char_count,
            token_count,
            pagination_mode: PaginationMode::Token,
        }
    }

    #[test]
    fn test_basic_window_generation() {
        // 10 tokens, window_size=5, stride=2
        let text = "one two three four five six seven eight nine ten";
        let page = make_page(1, text, 0);
        let windows = generate_windows(&[page], 5, 2);

        // Expected windows:
        // offset 0: tokens 0..5 -> "one two three four five"
        // offset 2: tokens 2..7 -> "three four five six seven"
        // offset 4: tokens 4..9 -> "five six seven eight nine"
        // offset 6: tokens 6..10 -> remaining 4 tokens >= 3 -> "seven eight nine ten"
        // offset 6: remaining = 4, which is < window_size(5), so terminal window with 4 tokens
        assert_eq!(windows.len(), 4);
        assert_eq!(windows[0].text, "one two three four five");
        assert_eq!(windows[1].text, "three four five six seven");
        assert_eq!(windows[2].text, "five six seven eight nine");
        assert_eq!(windows[3].text, "seven eight nine ten");
    }

    #[test]
    fn test_char_start_char_end_accuracy() {
        let text = "alpha beta gamma delta epsilon";
        let page = make_page(1, text, 100);
        let windows = generate_windows(&[page.clone()], 3, 2);

        for window in &windows {
            // Round-trip: extracting from page text must equal window text
            let extracted = &page.text[window.char_start as usize..window.char_end as usize];
            assert_eq!(
                extracted, window.text,
                "Round-trip failed for window_index {}",
                window.window_index
            );
        }
    }

    #[test]
    fn test_doc_char_start() {
        let text = "hello world foo bar baz";
        let page = make_page(1, text, 500);
        let windows = generate_windows(&[page], 3, 2);

        for window in &windows {
            assert_eq!(window.doc_char_start, 500 + window.char_start);
        }
    }

    #[test]
    fn test_windows_dont_cross_page_boundaries() {
        let text1 = "one two three four five";
        let text2 = "six seven eight nine ten";
        let page1 = make_page(1, text1, 0);
        let page2 = make_page(2, text2, text1.len() as u32 + 1);

        let windows = generate_windows(&[page1.clone(), page2.clone()], 3, 1);

        for window in &windows {
            if window.page == 1 {
                assert!(
                    (window.char_end as usize) <= page1.text.len(),
                    "Window crosses page 1 boundary"
                );
            } else {
                assert!(
                    (window.char_end as usize) <= page2.text.len(),
                    "Window crosses page 2 boundary"
                );
            }
        }
    }

    #[test]
    fn test_terminal_window_gte_3_tokens_included() {
        // 7 tokens, window_size=5, stride=5
        // offset 0: tokens 0..5 (full window)
        // offset 5: remaining = 2 tokens < 3 -> discarded
        // But let's use 8 tokens so remaining is 3
        let text = "a b c d e f g h";
        let page = make_page(1, text, 0);
        let windows = generate_windows(&[page], 5, 5);

        // offset 0: tokens 0..5 -> "a b c d e"
        // offset 5: remaining = 3 tokens >= 3 -> "f g h"
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[1].text, "f g h");
    }

    #[test]
    fn test_terminal_window_lt_3_tokens_discarded() {
        // 7 tokens, window_size=5, stride=5
        // offset 0: tokens 0..5 (full window)
        // offset 5: remaining = 2 tokens < 3 -> discarded
        let text = "a b c d e f g";
        let page = make_page(1, text, 0);
        let windows = generate_windows(&[page], 5, 5);

        // Only 1 full window, terminal has 2 tokens -> discarded
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].text, "a b c d e");
    }

    #[test]
    fn test_page_with_fewer_than_3_tokens_skipped() {
        let text1 = "hi there"; // 2 tokens -> skip
        let text2 = "one two three four five"; // 5 tokens -> produce windows
        let page1 = make_page(1, text1, 0);
        let page2 = make_page(2, text2, text1.len() as u32 + 1);

        let windows = generate_windows(&[page1, page2], 3, 1);

        // All windows should be from page 2
        assert!(!windows.is_empty());
        for window in &windows {
            assert_eq!(window.page, 2);
        }
    }

    #[test]
    fn test_window_index_sequential_across_pages() {
        let text1 = "one two three four five six";
        let text2 = "alpha beta gamma delta epsilon";
        let page1 = make_page(1, text1, 0);
        let page2 = make_page(2, text2, text1.len() as u32 + 1);

        let windows = generate_windows(&[page1, page2], 3, 2);

        // window_index should be 0, 1, 2, ... contiguous
        for (i, window) in windows.iter().enumerate() {
            assert_eq!(
                window.window_index, i as u32,
                "Expected window_index {} but got {}",
                i, window.window_index
            );
        }
    }

    #[test]
    fn test_stride_overlap() {
        // 6 tokens, window_size=4, stride=2
        let text = "the quick brown fox jumps over";
        let page = make_page(1, text, 0);
        let windows = generate_windows(&[page], 4, 2);

        // offset 0: tokens 0..4 -> "the quick brown fox"
        // offset 2: tokens 2..6 -> "brown fox jumps over"
        // offset 4: remaining = 2 < 3 -> discarded
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].text, "the quick brown fox");
        assert_eq!(windows[1].text, "brown fox jumps over");

        // Verify overlap: windows share "brown fox"
        assert!(windows[0].text.contains("brown fox"));
        assert!(windows[1].text.contains("brown fox"));
    }

    #[test]
    fn test_single_page_exact_window_size() {
        // Exactly window_size tokens -> 1 window, no terminal
        let text = "one two three four five";
        let page = make_page(1, text, 0);
        let windows = generate_windows(&[page], 5, 3);

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].text, "one two three four five");
        assert_eq!(windows[0].char_start, 0);
        assert_eq!(windows[0].char_end, text.len() as u32);
    }

    #[test]
    fn test_window_id_is_unique() {
        let text = "one two three four five six seven eight nine ten";
        let page = make_page(1, text, 0);
        let windows = generate_windows(&[page], 3, 1);

        let ids: Vec<&str> = windows.iter().map(|w| w.window_id.as_str()).collect();
        let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "Window IDs are not unique");
    }

    #[test]
    fn test_empty_pages_vec() {
        let windows = generate_windows(&[], 5, 2);
        assert!(windows.is_empty());
    }

    #[test]
    fn test_page_with_exactly_3_tokens() {
        // Page with exactly 3 tokens and window_size > 3 -> terminal window with 3 tokens
        let text = "foo bar baz";
        let page = make_page(1, text, 0);
        let windows = generate_windows(&[page], 5, 2);

        // 3 tokens < window_size(5), so it's a terminal segment with 3 tokens >= 3 -> included
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].text, "foo bar baz");
    }

    // === estimate_window_count tests ===

    #[test]
    fn test_estimate_window_count_normal() {
        // 100 tokens, window_size=20, stride=5
        // floor((100 - 20) / 5) + 1 = floor(80/5) + 1 = 16 + 1 = 17
        assert_eq!(estimate_window_count(100, 20, 5), 17);
    }

    #[test]
    fn test_estimate_window_count_total_equals_window_size() {
        // total_tokens == window_size -> exactly 1 window
        assert_eq!(estimate_window_count(20, 20, 5), 1);
    }

    #[test]
    fn test_estimate_window_count_total_less_than_window_size_but_gte_3() {
        // total_tokens < window_size but >= 3 -> 1 (valid terminal window)
        assert_eq!(estimate_window_count(10, 20, 5), 1);
        assert_eq!(estimate_window_count(3, 20, 5), 1);
    }

    #[test]
    fn test_estimate_window_count_total_less_than_3() {
        // total_tokens < 3 -> 0 (not enough tokens for any window)
        assert_eq!(estimate_window_count(2, 20, 5), 0);
        assert_eq!(estimate_window_count(1, 20, 5), 0);
        assert_eq!(estimate_window_count(0, 20, 5), 0);
    }

    #[test]
    fn test_estimate_window_count_various_strides() {
        // 50 tokens, window_size=10
        // stride=1: floor((50-10)/1) + 1 = 40 + 1 = 41
        assert_eq!(estimate_window_count(50, 10, 1), 41);
        // stride=5: floor((50-10)/5) + 1 = 8 + 1 = 9
        assert_eq!(estimate_window_count(50, 10, 5), 9);
        // stride=10: floor((50-10)/10) + 1 = 4 + 1 = 5
        assert_eq!(estimate_window_count(50, 10, 10), 5);
        // stride=20: floor((50-10)/20) + 1 = 2 + 1 = 3
        assert_eq!(estimate_window_count(50, 10, 20), 3);
        // stride=40: floor((50-10)/40) + 1 = 1 + 1 = 2
        assert_eq!(estimate_window_count(50, 10, 40), 2);
    }

    #[test]
    fn test_estimate_window_count_stride_equals_window_size() {
        // Non-overlapping windows: 100 tokens, window_size=25, stride=25
        // floor((100-25)/25) + 1 = 3 + 1 = 4
        assert_eq!(estimate_window_count(100, 25, 25), 4);
    }

    #[test]
    fn test_estimate_window_count_integer_division_floor() {
        // Verify floor behavior: 15 tokens, window_size=5, stride=4
        // floor((15-5)/4) + 1 = floor(10/4) + 1 = 2 + 1 = 3
        assert_eq!(estimate_window_count(15, 5, 4), 3);
    }
}
