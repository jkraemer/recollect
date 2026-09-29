//! Splits memory content into chunks that fit the embedding model's window.

/// Splits `content` into chunks of at most `max_tokens` tokens according to `count`.
///
/// Boundaries prefer Markdown headings, then blank-line paragraphs, then line
/// breaks, then spaces; a single word longer than the budget is cut by
/// characters. Neighbouring pieces are packed into one chunk while they fit.
/// Packing sums the counts of the pieces, which is exact for WordPiece
/// tokenizers such as BGE's: they never merge tokens across whitespace.
pub fn chunk(content: &str, max_tokens: usize, count: &dyn Fn(&str) -> usize) -> Vec<String> {
    let content = content.trim();
    if content.is_empty() {
        return Vec::new();
    }
    split(content, Level::Section, max_tokens, count)
}

#[derive(Clone, Copy)]
enum Level {
    Section,
    Paragraph,
    Line,
    Word,
}

impl Level {
    fn finer(self) -> Option<Level> {
        match self {
            Level::Section => Some(Level::Paragraph),
            Level::Paragraph => Some(Level::Line),
            Level::Line => Some(Level::Word),
            Level::Word => None,
        }
    }

    fn separator(self) -> &'static str {
        match self {
            Level::Section | Level::Paragraph => "\n\n",
            Level::Line => "\n",
            Level::Word => " ",
        }
    }

    fn pieces(self, text: &str) -> Vec<String> {
        match self {
            Level::Section => split_before_headings(text),
            Level::Paragraph => text
                .split("\n\n")
                .map(str::trim)
                .filter(|piece| !piece.is_empty())
                .map(String::from)
                .collect(),
            Level::Line => text
                .lines()
                .map(str::trim_end)
                .filter(|line| !line.trim().is_empty())
                .map(String::from)
                .collect(),
            Level::Word => text.split_whitespace().map(String::from).collect(),
        }
    }
}

fn split(
    text: &str,
    level: Level,
    max_tokens: usize,
    count: &dyn Fn(&str) -> usize,
) -> Vec<String> {
    if count(text) <= max_tokens {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_tokens = 0;
    for piece in level.pieces(text) {
        let tokens = count(&piece);
        if tokens > max_tokens {
            flush(&mut chunks, &mut current, &mut current_tokens);
            match level.finer() {
                Some(finer) => chunks.extend(split(&piece, finer, max_tokens, count)),
                None => chunks.extend(split_characters(&piece, max_tokens, count)),
            }
            continue;
        }
        if !current.is_empty() && current_tokens + tokens > max_tokens {
            flush(&mut chunks, &mut current, &mut current_tokens);
        }
        if !current.is_empty() {
            current.push_str(level.separator());
        }
        current.push_str(&piece);
        current_tokens += tokens;
    }
    flush(&mut chunks, &mut current, &mut current_tokens);
    chunks
}

fn flush(chunks: &mut Vec<String>, current: &mut String, current_tokens: &mut usize) {
    if !current.is_empty() {
        chunks.push(std::mem::take(current));
    }
    *current_tokens = 0;
}

/// Cuts a word with no whitespace into the longest prefixes that fit.
fn split_characters(word: &str, max_tokens: usize, count: &dyn Fn(&str) -> usize) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let (mut fits, mut too_long) = (start + 1, chars.len());
        while fits < too_long {
            let middle = (fits + too_long).div_ceil(2);
            let candidate: String = chars[start..middle].iter().collect();
            if count(&candidate) <= max_tokens {
                fits = middle;
            } else {
                too_long = middle - 1;
            }
        }
        pieces.push(chars[start..fits].iter().collect());
        start = fits;
    }
    pieces
}

/// Splits before every Markdown heading line; each section keeps its heading.
fn split_before_headings(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if is_heading(line) && !current.trim().is_empty() {
            sections.push(current.trim().to_string());
            current.clear();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        sections.push(current.trim().to_string());
    }
    sections
}

/// One to six `#` followed by a space.
fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whitespace-separated words: additive like a WordPiece count.
    fn words(text: &str) -> usize {
        text.split_whitespace().count()
    }

    fn chars(text: &str) -> usize {
        text.chars().count()
    }

    #[test]
    fn short_content_is_one_chunk_and_blank_content_none() {
        assert_eq!(chunk("  a b c \n", 10, &words), ["a b c"]);
        assert!(chunk(" \n\t", 10, &words).is_empty());
    }

    #[test]
    fn splits_before_markdown_headings_first() {
        let text = "# A\none two\n# B\nthree four";
        assert_eq!(chunk(text, 4, &words), ["# A\none two", "# B\nthree four"]);
    }

    #[test]
    fn small_sections_are_packed_together() {
        let text = "# A\nx\n# B\ny\n# C\nz z z z z";
        assert_eq!(
            chunk(text, 7, &words),
            ["# A\nx\n\n# B\ny", "# C\nz z z z z"]
        );
    }

    #[test]
    fn without_headings_paragraphs_then_lines_are_used() {
        assert_eq!(
            chunk("p1 p1 p1\n\np2 p2 p2", 4, &words),
            ["p1 p1 p1", "p2 p2 p2"]
        );
        assert_eq!(
            chunk("l1 l1 l1\nl2 l2 l2", 4, &words),
            ["l1 l1 l1", "l2 l2 l2"]
        );
    }

    #[test]
    fn an_overlong_line_is_split_into_word_windows() {
        assert_eq!(chunk("w1 w2 w3 w4 w5", 2, &words), ["w1 w2", "w3 w4", "w5"]);
    }

    #[test]
    fn an_overlong_word_is_cut_by_characters() {
        assert_eq!(chunk("abcdefghij", 4, &chars), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn only_real_markdown_headings_split_sections() {
        assert!(is_heading("## Title"));
        assert!(!is_heading("#!/bin/bash"));
        assert!(!is_heading("#hashtag"));
        assert!(!is_heading("####### seven"));
    }

    #[test]
    fn every_chunk_fits_and_no_words_are_lost() {
        let mut text = String::new();
        for section in 0..5 {
            text.push_str(&format!("# Section {section}\n"));
            for paragraph in 0..4 {
                let line = format!("s{section} p{paragraph} ").repeat(9);
                text.push_str(&format!("{line}\n{line}\n\n"));
            }
        }
        let chunks = chunk(&text, 25, &words);
        assert!(chunks.len() > 5);
        assert!(chunks.iter().all(|c| words(c) <= 25), "{chunks:?}");
        let rejoined: Vec<&str> = chunks.iter().flat_map(|c| c.split_whitespace()).collect();
        let original: Vec<&str> = text.split_whitespace().collect();
        assert_eq!(rejoined, original);
    }
}
