use std::borrow::Cow;
use std::ops::Range;

const MARKER_START: &str = "[[LR_MATH_";
const MARKER_END: &str = "]]";

pub struct ProtectedText<'a> {
    masked: Cow<'a, str>,
    formulas: Vec<&'a str>,
    prefix: String,
}

impl ProtectedText<'_> {
    pub fn as_str(&self) -> &str {
        &self.masked
    }

    pub fn has_formulas(&self) -> bool {
        !self.formulas.is_empty()
    }

    /// A response must contain every marker exactly once, without adding or changing one.
    pub fn restore_with_ranges(&self, translated: &str) -> Option<(String, Vec<Range<usize>>)> {
        self.restore_markers(translated, false)
    }

    pub fn preview_with_ranges(&self, translated: &str) -> Option<(String, Vec<Range<usize>>)> {
        self.restore_markers(translated, true)
    }

    fn restore_markers(&self, translated: &str, partial: bool) -> Option<(String, Vec<Range<usize>>)> {
        if !self.has_formulas() {
            return Some((translated.to_owned(), Vec::new()));
        }
        let mut seen = vec![false; self.formulas.len()];
        let mut text = String::with_capacity(translated.len());
        let mut ranges = Vec::with_capacity(self.formulas.len());
        let mut tail = translated;
        while let Some((plain, marker)) = tail.split_once(&self.prefix) {
            text.push_str(plain);
            let Some((id, rest)) = marker.split_once(MARKER_END) else {
                return partial.then_some((text, ranges));
            };
            let ordinal = id.parse::<usize>().ok()?;
            if ordinal.to_string() != id {
                return None;
            }
            let index = ordinal.checked_sub(1)?;
            let duplicate = seen.get_mut(index)?;
            if std::mem::replace(duplicate, true) {
                return None;
            }
            let start = text.len();
            text.push_str(self.formulas[index]);
            ranges.push(start..text.len());
            tail = rest;
        }
        if partial {
            // A placeholder can start in one stream update and finish in another.
            if let Some(length) = (1..self.prefix.len()).rev()
                .find(|&length| tail.ends_with(&self.prefix[..length]))
            {
                tail = &tail[..tail.len() - length];
            }
        } else if seen.contains(&false) {
            return None;
        }
        text.push_str(tail);
        Some((text, ranges))
    }

    #[cfg(test)]
    fn restore(&self, translated: &str) -> Option<String> {
        self.restore_with_ranges(translated).map(|(text, _)| text)
    }
}

#[cfg(test)]
fn has_inline_formula(text: &str) -> bool {
    !formula_ranges(text).is_empty()
}

pub fn protect(text: &str) -> ProtectedText<'_> {
    let ranges = formula_ranges(text);
    if ranges.is_empty() {
        return ProtectedText {
            masked: Cow::Borrowed(text),
            formulas: Vec::new(),
            prefix: String::new(),
        };
    }
    let mut prefix = MARKER_START.to_owned();
    while text.contains(&prefix) {
        prefix.push('_');
    }
    let mut masked = String::with_capacity(text.len());
    let mut formulas = Vec::with_capacity(ranges.len());
    let mut offset = 0;
    for range in ranges {
        masked.push_str(&text[offset..range.start]);
        formulas.push(&text[range.clone()]);
        masked.push_str(&prefix);
        masked.push_str(&formulas.len().to_string());
        masked.push_str(MARKER_END);
        offset = range.end;
    }
    masked.push_str(&text[offset..]);
    ProtectedText {
        masked: Cow::Owned(masked),
        formulas,
        prefix,
    }
}

/// Text slices stay in source order. A true flag marks a math run.
pub fn split_inline(text: &str) -> Vec<(&str, bool)> {
    span_runs(text, &formula_ranges(text))
}

/// Split into known math spans using sorted UTF-8 byte ranges.
pub fn span_runs<'a>(text: &'a str, ranges: &[Range<usize>]) -> Vec<(&'a str, bool)> {
    let mut result = Vec::new();
    let mut offset = 0;
    for range in ranges {
        let (Some(plain), Some(formula)) = (
            text.get(offset..range.start),
            text.get(range.clone()),
        ) else {
            return vec![(text, false)];
        };
        if !plain.is_empty() {
            result.push((plain, false));
        }
        if !formula.is_empty() {
            result.push((formula, true));
        }
        offset = range.end;
    }
    if offset < text.len() {
        result.push((&text[offset..], false));
    }
    result
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Atom,
    StyledAtom,
    Strong,
    Operator,
    Open,
    Close,
    Separator,
    Prose,
}

impl Kind {
    fn atom(self) -> bool {
        matches!(self, Self::Atom | Self::StyledAtom)
    }
}

struct Token<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    kind: Kind,
}

fn formula_ranges(text: &str) -> Vec<Range<usize>> {
    let tokens = tokenize(text);
    let mut ranges = explicit_formulas(text);
    let mut start = 0;
    while start < tokens.len() {
        if tokens[start].kind == Kind::Prose {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < tokens.len() && joins(&tokens[end - 1], &tokens[end], text) {
            end += 1;
        }
        let candidate = trim_formula(&tokens[start..end]);
        if candidate
            .iter()
            .enumerate()
            .any(|(i, token)| is_seed(token, i, candidate))
        {
            ranges.push(candidate[0].start..candidate[candidate.len() - 1].end);
        }
        start = end;
    }
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut().filter(|last| range.start < last.end) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut chars = text.char_indices().peekable();
    let mut tokens = Vec::new();
    while let Some((start, first)) = chars.next() {
        if first.is_whitespace() {
            continue;
        }
        let mut end = start + first.len_utf8();
        let leading_decimal =
            first == '.' && text[end..].starts_with(|ch: char| ch.is_ascii_digit());
        let kind = if is_word_char(first) || leading_decimal {
            while let Some(&(index, next)) = chars.peek() {
                let decimal = next == '.'
                    && text[start..index].bytes().all(|byte| byte.is_ascii_digit())
                    && text[index + 1..].starts_with(|ch: char| ch.is_ascii_digit());
                if !(is_word_char(next) || decimal) {
                    break;
                }
                chars.next();
                end = index + next.len_utf8();
            }
            let word = &text[start..end];
            let mut script_end = end;
            if is_script_base(word) {
                while let Some(length) = braced_script_length(&text[script_end..]) {
                    script_end += length;
                }
            }
            if script_end > end {
                while chars.peek().is_some_and(|&(index, _)| index < script_end) {
                    chars.next();
                }
                end = script_end;
                Kind::StyledAtom
            } else {
                word_kind(word)
            }
        } else if "≤≥≠≈≃≅≡≪≫∈∉⊂⊆⊃⊇∪∩∧∨→↔⇒⇔↦∝±−×÷∂∇∑∏√∫∞∀∃".contains(first) {
            Kind::Strong
        } else {
            match first {
                '(' | '[' | '{' | '⟨' => Kind::Open,
                ')' | ']' | '}' | '⟩' => Kind::Close,
                '=' | '<' | '>' | '+' | '-' | '/' | '*' | '·' | '⋅' | '^' | '_' | '|' => {
                    Kind::Operator
                }
                ',' | ';' | ':' => Kind::Separator,
                _ => Kind::Prose,
            }
        };
        tokens.push(Token {
            text: &text[start..end],
            start,
            end,
            kind,
        });
    }
    tokens
}

fn is_script_base(word: &str) -> bool {
    let base = word.split(is_script).next().unwrap_or_default();
    let mut letters = base.chars();
    letters.next().is_some_and(|ch| {
        (ch.is_ascii_alphabetic() || is_math_letter(ch)) && letters.next().is_none()
    }) || matches!(base, "lim" | "max" | "min" | "arg")
}

fn braced_script_length(text: &str) -> Option<usize> {
    let marker = text.trim_start_matches(is_horizontal_space);
    let argument = marker.strip_prefix('_').or_else(|| marker.strip_prefix('^'))?;
    let content = argument.trim_start_matches(is_horizontal_space).strip_prefix('{')?;
    let content_start = text.len() - content.len();
    let mut depth = 1;
    let mut has_atom = false;
    for (offset, ch) in content.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return has_atom.then_some(content_start + offset + 1);
                }
            }
            '\r' | '\n' => return None,
            ch if is_word_char(ch) => has_atom = true,
            ch if ch.is_whitespace() || "+-*/·⋅×−÷±=<>≤≥≠≈∈→^_()[]|.,:′'\\".contains(ch) => {}
            _ => return None,
        }
    }
    None
}

fn is_horizontal_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\u{00a0}' | '\u{202f}')
}

fn is_word_char(ch: char) -> bool {
    (ch.is_alphanumeric() && !is_cjk(ch)) || is_script(ch)
}

fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{3040}'..='\u{30ff}' | '\u{3100}'..='\u{312f}' | '\u{31a0}'..='\u{31bf}'
        | '\u{3400}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}' | '\u{f900}'..='\u{faff}'
        | '\u{20000}'..='\u{323af}')
}

fn word_kind(word: &str) -> Kind {
    let mut chars = word.chars();
    let first = chars.next().unwrap();
    if is_script(first) || is_math_letter(first) && chars.next().is_none() {
        return Kind::StyledAtom;
    }
    if word.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
        return Kind::Atom;
    }
    let base = word.split(is_script).next().unwrap_or_default();
    let count = base.chars().count();
    let likely_variable = count == 1 && first.is_alphabetic()
        || count == 2 && base.bytes().all(|byte| byte.is_ascii_uppercase());
    if likely_variable && base.len() != word.len() && word[base.len()..].chars().all(is_script) {
        return Kind::StyledAtom;
    }
    if likely_variable
        || matches!(
            word,
            "sin" | "cos" | "tan" | "log" | "ln" | "exp" | "lim" | "max" | "min" | "det"
                | "Pr" | "arg" | "sqrt"
        )
    {
        return Kind::Atom;
    }
    Kind::Prose
}

fn is_math_letter(ch: char) -> bool {
    matches!(ch, '\u{0370}'..='\u{03ff}' | '\u{1d400}'..='\u{1d7ff}' | 'ℕ' | 'ℤ' | 'ℚ' | 'ℝ' | 'ℂ' | 'ℓ' | 'ℏ' | 'ℵ')
}

fn is_script(ch: char) -> bool {
    matches!(ch, '¹' | '²' | '³' | '′' | '″' | '‴' | '⁗' | '\u{2070}'..='\u{209f}' | '\u{1d2c}'..='\u{1d6a}')
}

fn joins(left: &Token<'_>, right: &Token<'_>, source: &str) -> bool {
    if right.kind == Kind::Prose || source[left.end..right.start].contains(['\r', '\n']) {
        return false;
    }
    if left.kind.atom() && right.kind.atom() {
        return false;
    }
    if left.kind == Kind::Close && right.kind.atom() && left.end != right.start {
        return false;
    }
    if left.kind == Kind::Separator && right.kind == Kind::Open {
        return false;
    }
    true
}

fn trim_formula<'tokens, 'text>(mut tokens: &'tokens [Token<'text>]) -> &'tokens [Token<'text>] {
    loop {
        let Some(first) = tokens.first() else {
            return tokens;
        };
        let last = tokens.last().unwrap();
        let unmatched_open = first.kind == Kind::Open && !edge_balanced(tokens, true);
        let unmatched_close = last.kind == Kind::Close && !edge_balanced(tokens, false);
        if matches!(first.kind, Kind::Separator | Kind::Close) || unmatched_open {
            tokens = &tokens[1..];
        } else if matches!(last.kind, Kind::Separator | Kind::Open | Kind::Operator)
            || unmatched_close
        {
            tokens = &tokens[..tokens.len() - 1];
        } else {
            return tokens;
        }
    }
}

fn edge_balanced(tokens: &[Token<'_>], forward: bool) -> bool {
    let mut depth = 0;
    let mut edge = tokens.iter();
    while let Some(token) = if forward { edge.next() } else { edge.next_back() } {
        match token.kind {
            Kind::Open => depth += 1,
            Kind::Close => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return true;
        }
    }
    false
}

fn is_seed(token: &Token<'_>, index: usize, tokens: &[Token<'_>]) -> bool {
    if matches!(token.kind, Kind::StyledAtom | Kind::Strong) {
        return true;
    }
    let next = tokens.get(index + 1);
    if token.kind.atom()
        && token.text.starts_with(char::is_alphabetic)
        && next.is_some_and(|next| next.text == "(" && token.end == next.start)
    {
        return tokens[index + 2..].iter().any(|part| part.text == ")");
    }
    if token.kind != Kind::Operator || !matches!(token.text, "=" | "<" | ">" | "+" | "^" | "_") {
        return false;
    }
    let previous = index.checked_sub(1).and_then(|i| tokens.get(i));
    previous.is_some_and(|previous| previous.kind.atom() || previous.kind == Kind::Close)
        && next.is_some_and(|next| {
            next.kind.atom() || next.kind == Kind::Strong
                || (next.kind == Kind::Open && !matches!(token.text, "^" | "_"))
        })
}

fn explicit_formulas(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    for (start, _) in text.match_indices("\\(") {
        if let Some((content, _)) = text[start + 2..].split_once("\\)") {
            if !content.trim().is_empty() && !content.contains(['\r', '\n']) {
                ranges.push(start..start + 4 + content.len());
            }
        }
    }
    let mut dollars = text
        .match_indices('$')
        .map(|(start, _)| start)
        .filter(|&start| !text[..start].ends_with('\\'))
        .peekable();
    while let Some(start) = dollars.next() {
        let Some(&end) = dollars.peek() else { break };
        let content = &text[start + 1..end];
        if !content.is_empty()
            && content.trim().len() == content.len()
            && !content.contains(['\r', '\n'])
            && !text[end + 1..].starts_with(|ch: char| ch.is_ascii_digit())
            && (content.contains(['^', '_', '\\'])
                || tokenize(content).iter().all(|part| part.kind != Kind::Prose))
        {
            ranges.push(start..end + 1);
            dollars.next();
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_inline_unicode_expressions_and_scripts() {
        let original = "If x_i² ≤ y and f(x) = x ^ 2, then the rule holds.";
        let parts = split_inline(original);
        assert_eq!(parts, vec![("If ", false), ("x_i² ≤ y", true), (" and ", false),
            ("f(x) = x ^ 2", true), (", then the rule holds.", false)]);
        let protected = protect(original);
        assert_eq!(protected.as_str(), "If [[LR_MATH_1]] and [[LR_MATH_2]], then the rule holds.");
        assert_eq!(split_inline("Let σ′ = σ."), vec![("Let ", false), ("σ′ = σ", true), (".", false)]);
        assert_eq!(protected.restore("若 [[LR_MATH_1]] 且 [[LR_MATH_2]]，规则成立。"),
            Some("若 x_i² ≤ y 且 f(x) = x ^ 2，规则成立。".to_string()));
        let (reordered, ranges) = protected
            .restore_with_ranges("取[[LR_MATH_2]]，利用[[LR_MATH_1]]。")
            .unwrap();
        assert_eq!(
            span_runs(&reordered, &ranges),
            vec![
                ("取", false),
                ("f(x) = x ^ 2", true),
                ("，利用", false),
                ("x_i² ≤ y", true),
                ("。", false),
            ],
        );
        assert_eq!(
            split_inline("若p < .05则取θ。"),
            vec![("若", false), ("p < .05", true), ("则取", false), ("θ", true), ("。", false)],
        );
    }

    #[test]
    fn rejects_missing_unknown_and_duplicate_markers() {
        let protected = protect("The variables α and β are sampled.");
        assert_eq!(protected.restore("[[LR_MATH_1]]"), None);
        assert_eq!(protected.restore("[[LR_MATH_1]] [[LR_MATH_2]] [[LR_MATH_1]]"), None);
        assert_eq!(protected.restore("[[LR_MATH_1]] [[LR_MATH_2]] [[LR_MATH_3]]"), None);
        assert_eq!(protected.restore("[[LR_MATH_01]] [[LR_MATH_2]]"), None);
    }

    #[test]
    fn avoids_common_nonmath_text_and_accepts_explicit_delimiters() {
        let prose = "The user_id passed the A/B test, costing $5 in cash and $10 in credit [12].";
        assert!(!has_inline_formula(prose));
        assert_eq!(protect(prose).as_str(), prose);
        assert_eq!(split_inline("Use $x$ in \\(q(a)\\)."), vec![
            ("Use ", false), ("$x$", true), (" in ", false), ("\\(q(a)\\)", true), (".", false),
        ]);
        assert_eq!(
            split_inline("At $5, use $\\frac{x}{2}$."),
            vec![("At $5, use ", false), ("$\\frac{x}{2}$", true), (".", false)],
        );
    }

    #[test]
    fn picks_a_prefix_that_does_not_collide_with_source_text() {
        let original = "Literal [[LR_MATH_1]] and x = 1.";
        let protected = protect(original);
        assert!(protected.has_formulas());
        assert_eq!(protected.restore(protected.as_str()), Some(original.to_string()));
    }

    #[test]
    fn preserves_complete_braced_scripts_on_math_variables() {
        for formula in ["x_{ij}", "x_{train}", "x^{x+y}", "x _ {ij}", "β_{t}^{2}", r"x^{\frac{a}{b}}"] {
            assert_eq!(split_inline(formula), vec![(formula, true)]);
            let protected = protect(formula);
            assert_eq!(protected.as_str(), "[[LR_MATH_1]]");
            assert_eq!(protected.restore(protected.as_str()).as_deref(), Some(formula));
        }
        assert_eq!(
            split_inline("Use x_{train} = x^{x+y} for training."),
            vec![("Use ", false), ("x_{train} = x^{x+y}", true), (" for training.", false)],
        );
        for plain in ["filename_{train}", "x_{unclosed", "x_{nested{brace}", "x_{}", "x_{中文}"] {
            assert!(!has_inline_formula(plain), "{plain}");
        }
    }
}
