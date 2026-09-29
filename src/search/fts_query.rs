//! Turns free text into an FTS5 query that cannot be misread as FTS5 syntax.

/// Every word becomes a quoted term and double-quoted segments stay phrases;
/// terms are OR-ed so BM25 ranks memories matching more of them higher.
/// Pieces without letters or digits are dropped: the tokenizer would index
/// nothing for them. Returns `None` when no term remains.
pub fn build_fts_query(text: &str) -> Option<String> {
    let mut terms = Vec::new();
    for (index, segment) in text.split('"').enumerate() {
        if index % 2 == 1 {
            let phrase = segment.split_whitespace().collect::<Vec<_>>().join(" ");
            push_term(&mut terms, &phrase);
        } else {
            for word in segment.split_whitespace() {
                push_term(&mut terms, word);
            }
        }
    }
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
            r#""auth-bug:" OR "NEAR(x)" OR "AND" OR "col:val""#
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
    fn text_without_terms_is_none() {
        for text in ["", "   ", "* - :", r#""""#] {
            assert_eq!(build_fts_query(text), None, "{text:?}");
        }
    }
}
