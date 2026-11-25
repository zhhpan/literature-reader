use std::collections::HashSet;
use std::sync::OnceLock;

pub const LINE_HYPHENS: [char; 4] = ['-', '\u{2010}', '\u{2011}', '\u{00ad}'];

pub struct Hyphenation {
    // Full words and compounds observed away from a physical line break.
    document_words: HashSet<String>,
}

impl Hyphenation {
    pub fn from_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Self {
        let document_words = lines.flat_map(str::split_whitespace)
            .filter_map(normalize_word).collect();
        Self { document_words }
    }

    pub fn remove_line_hyphen(&self, before: &str, after: &str) -> bool {
        let Some((left, right)) = break_words(before, after) else { return false };
        let compound = format!("{left}-{right}");
        if self.document_words.contains(&compound) || dictionary_contains(&compound) {
            return false;
        }
        // Parts of longer compounds may themselves be split by the PDF layout.
        let tail = left.rsplit('-').next().unwrap_or(&left);
        let head = right.split('-').next().unwrap_or(&right);
        if dictionary_contains(tail) && dictionary_contains(head) {
            return false;
        }
        self.document_words.contains(&format!("{left}{right}"))
            || dictionary_contains(&format!("{tail}{head}"))
    }
}

fn break_words(before: &str, after: &str) -> Option<(String, String)> {
    let prefix = before.strip_suffix(['-', '\u{2010}'])?;
    let suffix = after.split_whitespace().next()?;
    if prefix.ends_with(char::is_whitespace) || !suffix.starts_with(|ch: char| ch.is_ascii_lowercase()) {
        return None;
    }
    Some((normalize_word(prefix.split_whitespace().next_back()?)?, normalize_word(suffix)?))
}

fn normalize_word(text: &str) -> Option<String> {
    let word = text.trim_matches(['(', ')', '[', ']', '{', '}', '\'', '"', '“', '”', '‘', '’', '.', ',', ';', ':', '?', '!']);
    if !(2..=96).contains(&word.len())
        || !word.split(['-', '\u{2010}', '\u{2011}'])
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
    {
        return None;
    }
    Some(word.to_ascii_lowercase().replace(['\u{2010}', '\u{2011}'], "-"))
}

fn dictionary_contains(word: &str) -> bool {
    static WORDS: OnceLock<Vec<&str>> = OnceLock::new();
    let words = WORDS.get_or_init(|| include_str!("data/english-words.txt").lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty()).collect());
    words.binary_search(&word).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_wrapped_words_from_genuine_compounds() {
        let hyphenation = Hyphenation::from_lines(std::iter::empty());
        assert!(hyphenation.remove_line_hyphen("the combina-", "tion.” For instance"));
        assert!(hyphenation.remove_line_hyphen("an inter-", "nal API"));
        assert!(!hyphenation.remove_line_hyphen("language-", "level restrictions"));
        assert!(!hyphenation.remove_line_hyphen("the re-", "sign process"));
        assert!(!hyphenation.remove_line_hyphen("an https://ex/combina-", "tion."));
        assert!(!hyphenation.remove_line_hyphen("the combina\u{2011}", "tion is intentional"));
    }

    #[test]
    fn uses_whole_words_in_the_document_for_unlisted_terminology() {
        let hyphenation = Hyphenation::from_lines(["In this system, zephyrkernel and re-sign are used.",
            "Someone else may resign."].into_iter());
        assert!(hyphenation.remove_line_hyphen("the zephyrker-", "nel platform"));
        assert!(!hyphenation.remove_line_hyphen("the re-", "sign process"));
    }
}
