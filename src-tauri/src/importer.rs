use crate::types::{AppError, ImportError, Page, PaginationMode, ValidationError};
use std::path::Path;

/// Maximum number of PDF pages to process.
const MAX_PDF_PAGES: usize = 300;

/// Imports a PDF file, extracting text from each page.
///
/// Each PDF page maps to one `Page` struct. Pages with no extractable text are
/// included as empty pages (text = "", token_count = 0) — they will be excluded
/// from windowing later. Character offsets are cumulative across all pages.
///
/// Returns an error if:
/// - The file does not exist or cannot be read
/// - The file is not a valid PDF
/// - No page in the PDF contains any extractable text
///
/// If the PDF has more than 300 pages, only the first 300 are processed.
pub fn import_pdf(path: &Path) -> Result<Vec<Page>, AppError> {
    // Check file exists
    if !path.exists() {
        return Err(AppError::Import(ImportError {
            message: format!("File not found: {}", path.display()),
            path: Some(path.display().to_string()),
        }));
    }

    // Attempt to extract text from the PDF
    let doc = pdf_extract::Document::load(path).map_err(|e| {
        AppError::Import(ImportError {
            message: format!("Failed to open PDF: {}", e),
            path: Some(path.display().to_string()),
        })
    })?;

    let page_count = doc.get_pages().len();
    let pages_to_process = page_count.min(MAX_PDF_PAGES);

    let mut pages: Vec<Page> = Vec::with_capacity(pages_to_process);
    let mut doc_offset: u32 = 0;
    let mut has_any_text = false;

    // pdf_extract::Document::get_pages() returns a BTreeMap<u32, ObjectId>
    // where keys are 1-based page numbers, sorted in order.
    let page_ids: Vec<_> = doc.get_pages().into_iter().take(pages_to_process).collect();

    for (page_num_key, _page_id) in &page_ids {
        let text = doc
            .extract_text(&[*page_num_key])
            .unwrap_or_default();

        let token_count = text.split_whitespace().count() as u32;
        let char_count = text.len() as u32;

        if token_count > 0 {
            has_any_text = true;
        }

        pages.push(Page {
            page_num: pages.len() as u32 + 1,
            text,
            char_offset_in_doc: doc_offset,
            char_count,
            token_count,
            pagination_mode: PaginationMode::Pdf,
        });

        doc_offset += char_count;
    }

    if !has_any_text {
        return Err(AppError::Import(ImportError {
            message: "PDF contains no extractable text on any page".to_string(),
            path: Some(path.display().to_string()),
        }));
    }

    Ok(pages)
}

/// Splits plain text into pages using whitespace tokenization at the `tokens_per_page` boundary.
///
/// Each page preserves character-accurate offsets such that
/// `text[page.char_offset_in_doc as usize..(page.char_offset_in_doc + page.char_count) as usize]`
/// equals `page.text` for every page, and concatenating all page texts reproduces the original
/// document exactly.
///
/// Returns an error if the text is empty or contains only whitespace.
pub fn paginate_by_token_count(text: &str, tokens_per_page: u32) -> Result<Vec<Page>, AppError> {
    // Reject empty or whitespace-only input
    if text.is_empty() || text.chars().all(|c| c.is_whitespace()) {
        return Err(AppError::Import(ImportError {
            message: "File contains no analyzable text".to_string(),
            path: None,
        }));
    }

    let mut pages: Vec<Page> = Vec::new();
    let mut page_num: u32 = 1;
    let mut doc_offset: usize = 0;

    while doc_offset < text.len() {
        let remaining = &text[doc_offset..];

        // Count tokens (whitespace-split words) and find the byte offset where
        // we've consumed `tokens_per_page` tokens worth of text.
        let page_end_offset = find_page_boundary(remaining, tokens_per_page);

        let page_text = &remaining[..page_end_offset];
        let token_count = page_text.split_whitespace().count() as u32;

        pages.push(Page {
            page_num,
            text: page_text.to_string(),
            char_offset_in_doc: doc_offset as u32,
            char_count: page_end_offset as u32,
            token_count,
            pagination_mode: PaginationMode::Token,
        });

        doc_offset += page_end_offset;
        page_num += 1;
    }

    Ok(pages)
}

/// Finds the byte offset within `text` that represents the end of a page containing
/// up to `tokens_per_page` tokens.
///
/// The page boundary is placed immediately after the last character of the Nth token
/// (where N = tokens_per_page), including any trailing whitespace up to (but not including)
/// the next token. This ensures that concatenating all page slices reproduces the original
/// document exactly.
///
/// Uses `char::is_whitespace()` for Unicode-aware whitespace detection, matching
/// the behavior of `str::split_whitespace()`.
fn find_page_boundary(text: &str, tokens_per_page: u32) -> usize {
    let mut token_count: u32 = 0;
    let mut chars = text.char_indices().peekable();

    loop {
        // Skip whitespace
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        if chars.peek().is_none() {
            // Reached end of text
            break;
        }

        // We're at the start of a token
        token_count += 1;

        // Skip the token (non-whitespace characters)
        while let Some(&(_, c)) = chars.peek() {
            if !c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        if token_count == tokens_per_page {
            // We've consumed enough tokens. Now include trailing whitespace
            // up to the next token (or end of text) as part of this page.
            // This ensures the page boundary falls between tokens and
            // concatenation reproduces the original.
            while let Some(&(_, c)) = chars.peek() {
                if c.is_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }
            break;
        }
    }

    // Return the byte offset of the current position
    match chars.peek() {
        Some(&(idx, _)) => idx,
        None => text.len(),
    }
}

/// Splits plain text into pages using chapter break boundaries detected by a regex pattern.
///
/// The regex is applied line-by-line. When a line matches the pattern, it starts a new page
/// (the matching line becomes the first content of that new page). If a chapter exceeds
/// `tokens_per_page`, it is further split using token-count pagination at the overflow point.
///
/// When `chapter_break_regex` is empty, falls back to `paginate_by_token_count`.
/// Returns a validation error if the regex is syntactically invalid.
pub fn paginate_by_chapter_break(
    text: &str,
    chapter_break_regex: &str,
    tokens_per_page: u32,
) -> Result<Vec<Page>, AppError> {
    // Fall back to token-count pagination when regex is blank
    if chapter_break_regex.is_empty() {
        return paginate_by_token_count(text, tokens_per_page);
    }

    // Validate regex syntax
    let re = regex::Regex::new(chapter_break_regex).map_err(|e| {
        AppError::Validation(ValidationError {
            field: "chapter_break_regex".to_string(),
            message: format!("Invalid regex pattern: {}", e),
        })
    })?;

    // Reject empty or whitespace-only input
    if text.is_empty() || text.chars().all(|c| c.is_whitespace()) {
        return Err(AppError::Import(ImportError {
            message: "File contains no analyzable text".to_string(),
            path: None,
        }));
    }

    // Find chapter break positions by scanning lines.
    // Each break position is the byte offset of the start of a matching line.
    let mut break_positions: Vec<usize> = Vec::new();
    let mut line_start = 0;

    for line in text.split('\n') {
        if re.is_match(line) {
            break_positions.push(line_start);
        }
        // +1 for the '\n' delimiter (split doesn't include it)
        line_start += line.len() + 1;
    }

    // Build chapter segments as byte ranges [start, end) in the original text
    let mut segments: Vec<(usize, usize)> = Vec::new();

    if break_positions.is_empty() {
        // No chapter breaks found — entire text is one segment
        segments.push((0, text.len()));
    } else {
        // Content before the first chapter break (if any)
        if break_positions[0] > 0 {
            segments.push((0, break_positions[0]));
        }
        // Each chapter break starts a segment that ends at the next break (or end of text)
        for i in 0..break_positions.len() {
            let start = break_positions[i];
            let end = if i + 1 < break_positions.len() {
                break_positions[i + 1]
            } else {
                text.len()
            };
            segments.push((start, end));
        }
    }

    // Now paginate each segment, splitting oversized chapters by token count
    let mut pages: Vec<Page> = Vec::new();
    let mut page_num: u32 = 1;

    for (seg_start, seg_end) in segments {
        let segment_text = &text[seg_start..seg_end];

        // Skip empty segments
        if segment_text.is_empty() || segment_text.chars().all(|c| c.is_whitespace()) {
            continue;
        }

        let token_count = segment_text.split_whitespace().count() as u32;

        if token_count <= tokens_per_page {
            // Segment fits in one page
            pages.push(Page {
                page_num,
                text: segment_text.to_string(),
                char_offset_in_doc: seg_start as u32,
                char_count: segment_text.len() as u32,
                token_count,
                pagination_mode: PaginationMode::Chapter,
            });
            page_num += 1;
        } else {
            // Segment exceeds tokens_per_page — split using token-count pagination
            let sub_pages = split_segment_by_token_count(segment_text, tokens_per_page);
            for (sub_offset, sub_text, sub_token_count) in sub_pages {
                pages.push(Page {
                    page_num,
                    text: sub_text.to_string(),
                    char_offset_in_doc: (seg_start + sub_offset) as u32,
                    char_count: sub_text.len() as u32,
                    token_count: sub_token_count,
                    pagination_mode: PaginationMode::Chapter,
                });
                page_num += 1;
            }
        }
    }

    // If no pages were produced (all segments were whitespace), return error
    if pages.is_empty() {
        return Err(AppError::Import(ImportError {
            message: "File contains no analyzable text".to_string(),
            path: None,
        }));
    }

    Ok(pages)
}

/// Splits a text segment into sub-pages of at most `tokens_per_page` tokens each.
/// Returns a Vec of (byte_offset_within_segment, text_slice, token_count).
fn split_segment_by_token_count(
    text: &str,
    tokens_per_page: u32,
) -> Vec<(usize, &str, u32)> {
    let mut result = Vec::new();
    let mut offset: usize = 0;

    while offset < text.len() {
        let remaining = &text[offset..];
        let page_end = find_page_boundary(remaining, tokens_per_page);
        let page_text = &remaining[..page_end];
        let token_count = page_text.split_whitespace().count() as u32;

        if token_count > 0 || !page_text.trim().is_empty() {
            result.push((offset, page_text, token_count));
        }

        offset += page_end;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ─── PDF Import Tests ────────────────────────────────────────────────────

    #[test]
    fn test_import_pdf_nonexistent_file() {
        let path = PathBuf::from("/nonexistent/path/to/file.pdf");
        let result = import_pdf(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Import(e) => {
                assert!(e.message.contains("File not found"));
                assert_eq!(e.path, Some(path.display().to_string()));
            }
            _ => panic!("Expected Import error"),
        }
    }

    #[test]
    fn test_import_pdf_non_pdf_file() {
        // Create a temporary text file and try to import it as PDF
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("not_a_pdf.pdf");
        std::fs::write(&file_path, "This is just plain text, not a PDF.").unwrap();

        let result = import_pdf(&file_path);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Import(e) => {
                assert!(e.message.contains("Failed to open PDF"));
            }
            _ => panic!("Expected Import error"),
        }
    }

    #[test]
    fn test_import_pdf_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.pdf");
        std::fs::write(&file_path, "").unwrap();

        let result = import_pdf(&file_path);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Import(e) => {
                assert!(e.message.contains("Failed to open PDF"));
            }
            _ => panic!("Expected Import error"),
        }
    }

    // ─── Token Pagination Tests ──────────────────────────────────────────────

    #[test]
    fn test_basic_pagination() {
        // 10 words, 3 tokens per page -> 4 pages (3, 3, 3, 1)
        let text = "one two three four five six seven eight nine ten";
        let pages = paginate_by_token_count(text, 3).unwrap();

        assert_eq!(pages.len(), 4);
        assert_eq!(pages[0].page_num, 1);
        assert_eq!(pages[0].token_count, 3);
        assert_eq!(pages[1].page_num, 2);
        assert_eq!(pages[1].token_count, 3);
        assert_eq!(pages[2].page_num, 3);
        assert_eq!(pages[2].token_count, 3);
        assert_eq!(pages[3].page_num, 4);
        assert_eq!(pages[3].token_count, 1);
    }

    #[test]
    fn test_character_offsets_roundtrip() {
        let text = "Hello world, this is a test of the pagination system with multiple words.";
        let pages = paginate_by_token_count(text, 4).unwrap();

        // Verify each page's text matches the slice from the original
        for page in &pages {
            let start = page.char_offset_in_doc as usize;
            let end = start + page.char_count as usize;
            assert_eq!(
                &text[start..end],
                page.text,
                "Page {} text mismatch",
                page.page_num
            );
        }

        // Verify concatenation reproduces original
        let reconstructed: String = pages.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn test_final_short_page_included() {
        // 7 words, 5 tokens per page -> 2 pages (5 tokens, 2 tokens)
        let text = "alpha beta gamma delta epsilon zeta eta";
        let pages = paginate_by_token_count(text, 5).unwrap();

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].token_count, 5);
        assert_eq!(pages[1].token_count, 2);
        // Final page should not be padded
        assert_eq!(pages[1].text, "zeta eta");
    }

    #[test]
    fn test_empty_input_returns_error() {
        let result = paginate_by_token_count("", 400);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Import(e) => {
                assert!(e.message.contains("no analyzable text"));
            }
            _ => panic!("Expected Import error"),
        }
    }

    #[test]
    fn test_whitespace_only_input_returns_error() {
        let result = paginate_by_token_count("   \n\t  \r\n  ", 400);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Import(e) => {
                assert!(e.message.contains("no analyzable text"));
            }
            _ => panic!("Expected Import error"),
        }
    }

    #[test]
    fn test_single_page_case() {
        // Text shorter than tokens_per_page
        let text = "short text here";
        let pages = paginate_by_token_count(text, 400).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].page_num, 1);
        assert_eq!(pages[0].text, text);
        assert_eq!(pages[0].char_offset_in_doc, 0);
        assert_eq!(pages[0].char_count, text.len() as u32);
        assert_eq!(pages[0].token_count, 3);
        assert_eq!(pages[0].pagination_mode, PaginationMode::Token);
    }

    #[test]
    fn test_preserves_internal_whitespace() {
        // Text with multiple spaces and newlines between words
        let text = "word1  word2\n\nword3\tword4   word5";
        let pages = paginate_by_token_count(text, 3).unwrap();

        // Concatenation must reproduce original exactly
        let reconstructed: String = pages.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn test_leading_whitespace_preserved() {
        let text = "  leading spaces then words follow here now";
        let pages = paginate_by_token_count(text, 3).unwrap();

        let reconstructed: String = pages.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn test_trailing_whitespace_preserved() {
        let text = "words here now   ";
        let pages = paginate_by_token_count(text, 2).unwrap();

        let reconstructed: String = pages.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn test_exact_boundary() {
        // Exactly tokens_per_page tokens -> 1 page
        let text = "one two three four five";
        let pages = paginate_by_token_count(text, 5).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].token_count, 5);
        assert_eq!(pages[0].text, text);
    }

    #[test]
    fn test_pagination_mode_is_token() {
        let text = "some words to paginate";
        let pages = paginate_by_token_count(text, 2).unwrap();

        for page in &pages {
            assert_eq!(page.pagination_mode, PaginationMode::Token);
        }
    }

    #[test]
    fn test_page_numbers_are_one_based() {
        let text = "a b c d e f g h i j";
        let pages = paginate_by_token_count(text, 3).unwrap();

        for (i, page) in pages.iter().enumerate() {
            assert_eq!(page.page_num, (i + 1) as u32);
        }
    }

    // ─── Chapter Break Pagination Tests ──────────────────────────────────────

    #[test]
    fn test_chapter_break_basic_detection() {
        let text = "Prologue content here\nChapter 1\nFirst chapter text goes here\nChapter 2\nSecond chapter text goes here";
        let pages = paginate_by_chapter_break(text, r"^Chapter\s+\d+", 400).unwrap();

        // Should produce 3 pages: prologue, chapter 1, chapter 2
        assert_eq!(pages.len(), 3);

        // Prologue (content before first chapter break)
        assert_eq!(pages[0].page_num, 1);
        assert!(pages[0].text.starts_with("Prologue"));

        // Chapter 1 starts with the matching line
        assert_eq!(pages[1].page_num, 2);
        assert!(pages[1].text.starts_with("Chapter 1"));

        // Chapter 2 starts with the matching line
        assert_eq!(pages[2].page_num, 3);
        assert!(pages[2].text.starts_with("Chapter 2"));
    }

    #[test]
    fn test_chapter_break_line_is_first_content_of_new_page() {
        let text = "Some intro\nChapter 1\nContent of chapter one\nChapter 2\nContent of chapter two";
        let pages = paginate_by_chapter_break(text, r"^Chapter\s+\d+", 400).unwrap();

        // Chapter 1 page must start with "Chapter 1"
        assert!(
            pages[1].text.starts_with("Chapter 1"),
            "Expected page 2 to start with 'Chapter 1', got: {:?}",
            &pages[1].text[..20.min(pages[1].text.len())]
        );

        // Chapter 2 page must start with "Chapter 2"
        assert!(
            pages[2].text.starts_with("Chapter 2"),
            "Expected page 3 to start with 'Chapter 2', got: {:?}",
            &pages[2].text[..20.min(pages[2].text.len())]
        );
    }

    #[test]
    fn test_chapter_break_oversized_chapter_splitting() {
        // Create a chapter with many words that exceeds tokens_per_page
        let mut text = String::from("Chapter 1\n");
        for i in 0..50 {
            text.push_str(&format!("word{} ", i));
        }
        text.push_str("\nChapter 2\nShort chapter");

        let pages = paginate_by_chapter_break(&text, r"^Chapter\s+\d+", 10).unwrap();

        // Every page must have at most 10 tokens
        for page in &pages {
            assert!(
                page.token_count <= 10,
                "Page {} has {} tokens, exceeds cap of 10",
                page.page_num,
                page.token_count
            );
        }

        // First page should start with "Chapter 1"
        assert!(pages[0].text.starts_with("Chapter 1"));
    }

    #[test]
    fn test_chapter_break_empty_regex_falls_back_to_token_count() {
        let text = "one two three four five six seven eight nine ten";
        let pages = paginate_by_chapter_break(text, "", 3).unwrap();

        // Should behave exactly like paginate_by_token_count
        let expected = paginate_by_token_count(text, 3).unwrap();
        assert_eq!(pages.len(), expected.len());

        for (page, exp) in pages.iter().zip(expected.iter()) {
            assert_eq!(page.text, exp.text);
            assert_eq!(page.char_offset_in_doc, exp.char_offset_in_doc);
            assert_eq!(page.token_count, exp.token_count);
        }
    }

    #[test]
    fn test_chapter_break_invalid_regex_returns_validation_error() {
        let text = "Some text here";
        let result = paginate_by_chapter_break(text, r"[invalid(", 400);

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Validation(e) => {
                assert_eq!(e.field, "chapter_break_regex");
                assert!(e.message.contains("Invalid regex"));
            }
            other => panic!("Expected Validation error, got: {:?}", other),
        }
    }

    #[test]
    fn test_chapter_break_character_offset_roundtrip() {
        let text = "Intro text\nChapter 1\nFirst chapter content with several words\nChapter 2\nSecond chapter content";
        let pages = paginate_by_chapter_break(text, r"^Chapter\s+\d+", 400).unwrap();

        // Verify each page's text matches the slice from the original
        for page in &pages {
            let start = page.char_offset_in_doc as usize;
            let end = start + page.char_count as usize;
            assert_eq!(
                &text[start..end],
                page.text,
                "Page {} text mismatch: expected {:?}, got {:?}",
                page.page_num,
                &text[start..end],
                page.text
            );
        }

        // Verify concatenation reproduces original
        let reconstructed: String = pages.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn test_chapter_break_pagination_mode_is_chapter() {
        let text = "Chapter 1\nSome content\nChapter 2\nMore content";
        let pages = paginate_by_chapter_break(text, r"^Chapter\s+\d+", 400).unwrap();

        for page in &pages {
            assert_eq!(page.pagination_mode, PaginationMode::Chapter);
        }
    }

    #[test]
    fn test_chapter_break_no_matches_produces_single_segment() {
        // Regex doesn't match anything — entire text becomes one page
        let text = "This text has no chapter markers at all just words";
        let pages = paginate_by_chapter_break(text, r"^Chapter\s+\d+", 400).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].text, text);
    }

    #[test]
    fn test_chapter_break_text_starts_with_chapter() {
        // No prologue — text starts directly with a chapter break
        let text = "Chapter 1\nFirst chapter\nChapter 2\nSecond chapter";
        let pages = paginate_by_chapter_break(text, r"^Chapter\s+\d+", 400).unwrap();

        assert_eq!(pages.len(), 2);
        assert!(pages[0].text.starts_with("Chapter 1"));
        assert!(pages[1].text.starts_with("Chapter 2"));
    }

    #[test]
    fn test_chapter_break_empty_text_returns_error() {
        let result = paginate_by_chapter_break("", r"^Chapter\s+\d+", 400);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Import(e) => {
                assert!(e.message.contains("no analyzable text"));
            }
            _ => panic!("Expected Import error"),
        }
    }

    #[test]
    fn test_chapter_break_whitespace_only_returns_error() {
        let result = paginate_by_chapter_break("   \n\t  \n  ", r"^Chapter\s+\d+", 400);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Import(e) => {
                assert!(e.message.contains("no analyzable text"));
            }
            _ => panic!("Expected Import error"),
        }
    }

    #[test]
    fn test_chapter_break_oversized_roundtrip() {
        // Create text with chapters that exceed the token cap
        let mut text = String::from("Chapter 1\n");
        for i in 0..30 {
            text.push_str(&format!("word{} ", i));
        }
        text.push_str("\nChapter 2\n");
        for i in 0..20 {
            text.push_str(&format!("item{} ", i));
        }

        let pages = paginate_by_chapter_break(&text, r"^Chapter\s+\d+", 10).unwrap();

        // Verify concatenation reproduces original
        let reconstructed: String = pages.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(reconstructed, text);

        // Verify character offset round-trip
        for page in &pages {
            let start = page.char_offset_in_doc as usize;
            let end = start + page.char_count as usize;
            assert_eq!(&text[start..end], page.text);
        }
    }

    #[test]
    fn test_chapter_break_custom_regex() {
        // Use a custom regex pattern (scene breaks with "---")
        let text = "Scene one content\n---\nScene two content\n---\nScene three content";
        let pages = paginate_by_chapter_break(text, r"^---$", 400).unwrap();

        assert_eq!(pages.len(), 3);
        assert!(pages[0].text.starts_with("Scene one"));
        assert!(pages[1].text.starts_with("---"));
        assert!(pages[2].text.starts_with("---"));
    }

    #[test]
    fn test_chapter_break_page_numbers_sequential() {
        let text = "Chapter 1\nContent\nChapter 2\nContent\nChapter 3\nContent";
        let pages = paginate_by_chapter_break(text, r"^Chapter\s+\d+", 400).unwrap();

        for (i, page) in pages.iter().enumerate() {
            assert_eq!(page.page_num, (i + 1) as u32);
        }
    }
}
