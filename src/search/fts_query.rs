//! Turns free text into an FTS5 query that cannot be misread as FTS5 syntax.

/// English words that carry no meaning of their own. Terms are OR-ed, so each
/// of them would match nearly every memory and bury the relevant ones. This is
/// NLTK's English stopword list without its contraction fragments (`s`, `t`,
/// `ll`, `don`, `isn`, ...): queries are split at whitespace, so those only
/// ever occur as words in their own right.
const STOPWORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "against",
    "all",
    "am",
    "an",
    "and",
    "any",
    "are",
    "aren't",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can",
    "couldn't",
    "did",
    "didn't",
    "do",
    "does",
    "doesn't",
    "doing",
    "don't",
    "down",
    "during",
    "each",
    "few",
    "for",
    "from",
    "further",
    "had",
    "hadn't",
    "has",
    "hasn't",
    "have",
    "haven't",
    "having",
    "he",
    "her",
    "here",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "i",
    "if",
    "in",
    "into",
    "is",
    "isn't",
    "it",
    "it's",
    "its",
    "itself",
    "just",
    "me",
    "mightn't",
    "more",
    "most",
    "mustn't",
    "my",
    "myself",
    "needn't",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "on",
    "once",
    "only",
    "or",
    "other",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "same",
    "shan't",
    "she",
    "she's",
    "should",
    "should've",
    "shouldn't",
    "so",
    "some",
    "such",
    "than",
    "that",
    "that'll",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "very",
    "was",
    "wasn't",
    "we",
    "were",
    "weren't",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "whom",
    "why",
    "will",
    "with",
    "won't",
    "wouldn't",
    "you",
    "you'd",
    "you'll",
    "you're",
    "you've",
    "your",
    "yours",
    "yourself",
    "yourselves",
];

/// Every word becomes a quoted term and double-quoted segments stay phrases;
/// terms are OR-ed so BM25 ranks memories matching more of them higher.
/// Stopwords are left out, except inside phrases and when nothing else would
/// remain: a query made only of stopwords keeps them all. Pieces without
/// letters or digits are dropped: the tokenizer would index nothing for them.
/// NUL separates words, since SQLite reads the query as a C string. Returns
/// `None` when no term remains.
pub fn build_fts_query(text: &str) -> Option<String> {
    let mut content = Vec::new();
    let mut stopwords = Vec::new();
    for (index, segment) in text.replace('\0', " ").split('"').enumerate() {
        if index % 2 == 1 {
            let phrase = segment.split_whitespace().collect::<Vec<_>>().join(" ");
            push_term(&mut content, &phrase);
        } else {
            for word in segment.split_whitespace() {
                let terms = if is_stopword(word) {
                    &mut stopwords
                } else {
                    &mut content
                };
                push_term(terms, word);
            }
        }
    }
    let terms = if content.is_empty() {
        stopwords
    } else {
        content
    };
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Quoting makes FTS5 treat the text as a string; splitting on `"` above
/// guarantees the term itself contains no quote to escape.
fn push_term(terms: &mut Vec<String>, term: &str) {
    if term.chars().any(char::is_alphanumeric) {
        terms.push(format!("\"{term}\""));
    }
}

/// Whether the word is an English stopword, whatever its case and the
/// punctuation around it (quotes, brackets, commas). The typographic
/// apostrophe counts as the plain one so "don’t" matches "don't", whose
/// apostrophe lies inside the word and stays.
fn is_stopword(word: &str) -> bool {
    let word = word.to_lowercase().replace('’', "'");
    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
    STOPWORDS.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_become_quoted_terms_joined_by_or() {
        assert_eq!(
            build_fts_query("auth bug login").unwrap(),
            r#""auth" OR "bug" OR "login""#
        );
    }

    #[test]
    fn quoted_segments_stay_phrases() {
        assert_eq!(
            build_fts_query(r#"deploy "exact  phrase" x"#).unwrap(),
            r#""deploy" OR "exact phrase" OR "x""#
        );
    }

    #[test]
    fn fts5_syntax_is_neutralized() {
        assert_eq!(
            build_fts_query("auth-bug: NEAR(x) AND * col:val").unwrap(),
            r#""auth-bug:" OR "NEAR(x)" OR "col:val""#
        );
    }

    #[test]
    fn stopwords_are_dropped() {
        assert_eq!(
            build_fts_query("how is data persisted on disk").unwrap(),
            r#""data" OR "persisted" OR "disk""#
        );
    }

    #[test]
    fn stopwords_match_whatever_their_case_or_punctuation() {
        assert_eq!(build_fts_query("The Bug").unwrap(), r#""Bug""#);
        assert_eq!(build_fts_query("the, bug.").unwrap(), r#""bug.""#);
        assert_eq!(build_fts_query("(which) bug").unwrap(), r#""bug""#);
        assert_eq!(build_fts_query("'the' bug").unwrap(), r#""bug""#);
        assert_eq!(build_fts_query("don't, bug").unwrap(), r#""bug""#);
        assert_eq!(build_fts_query("don’t panic").unwrap(), r#""panic""#);
        assert_eq!(build_fts_query("don't panic").unwrap(), r#""panic""#);
    }

    #[test]
    fn quoted_phrases_keep_their_stopwords() {
        assert_eq!(
            build_fts_query(r#""in progress" work"#).unwrap(),
            r#""in progress" OR "work""#
        );
        assert_eq!(
            build_fts_query(r#""in progress" the"#).unwrap(),
            r#""in progress""#
        );
    }

    #[test]
    fn a_query_of_only_stopwords_keeps_its_words() {
        assert_eq!(build_fts_query("the who").unwrap(), r#""the" OR "who""#);
        assert_eq!(build_fts_query("the *").unwrap(), r#""the""#);
        assert_eq!(
            build_fts_query("AND OR NOT").unwrap(),
            r#""AND" OR "OR" OR "NOT""#
        );
    }

    #[test]
    fn an_unbalanced_quote_opens_a_phrase_to_the_end() {
        assert_eq!(
            build_fts_query(r#"foo "bar baz"#).unwrap(),
            r#""foo" OR "bar baz""#
        );
    }

    #[test]
    fn nul_separates_words() {
        assert_eq!(build_fts_query("auth\0bug").unwrap(), r#""auth" OR "bug""#);
        assert_eq!(
            build_fts_query("\"login\" \"a\0b\"").unwrap(),
            r#""login" OR "a b""#
        );
    }

    #[test]
    fn text_without_terms_is_none() {
        for text in ["", "   ", "* - :", r#""""#] {
            assert_eq!(build_fts_query(text), None, "{text:?}");
        }
    }
}
