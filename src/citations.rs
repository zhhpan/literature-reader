use crate::pdf::{BlockKind, Document};
use std::collections::{BTreeMap, HashMap, HashSet};
use url::{Host, Url};

#[derive(Debug, PartialEq, Eq)]
pub struct CitationLink {
    pub label: String,
    pub target: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SourceLink {
    pub label: String,
    pub url: String,
}

type Numbered = BTreeMap<u32, Option<usize>>;
type AuthorYears = HashMap<(String, String), Option<usize>>;

/// Resolves citations to bibliography blocks. Ambiguous or missing entries never become links.
pub fn build(document: &Document) -> Vec<Vec<CitationLink>> {
    resolve(document.blocks.iter().map(|block| (block.kind, block.text.as_str())))
}

fn resolve<'a>(blocks: impl Iterator<Item = (BlockKind, &'a str)> + Clone) -> Vec<Vec<CitationLink>> {
    let mut numbers = Numbered::new();
    let mut authors = AuthorYears::new();
    let mut numbered_parentheses = false;
    for (index, (kind, text)) in blocks.clone().enumerate() {
        if kind != BlockKind::Reference {
            continue;
        }
        let text = text.trim();
        let entry = if let Some((number, rest)) = reference_number(text) {
            numbered_parentheses |= text.starts_with('(');
            numbers.entry(number).and_modify(|other| *other = None).or_insert(Some(index));
            rest
        } else {
            text
        };
        if let Some(key) = reference_author_year(entry) {
            authors.entry(key).and_modify(|other| *other = None).or_insert(Some(index));
        }
    }

    // A group such as (4; 7; 12) is strong evidence of a parenthesized citation style.
    // A lone (4) in a bracket-numbered paper is otherwise likely an equation or a list item.
    numbered_parentheses |= !numbers.is_empty() && blocks.clone().any(|(kind, text)| {
        has_inline_citations(kind) && delimited(text, '(', ')').any(|(start, group)| {
            !is_numbered_object(&text[..start])
                && number_ranges(group).is_some_and(|ranges| {
                    let mut matched = ranges.into_iter().flat_map(|(first, last)| numbers.range(first..=last))
                        .filter_map(|(number, target)| target.is_some().then_some(number));
                    matched.next().is_some_and(|first| matched.any(|number| number != first))
                })
        })
    });

    blocks.map(|(kind, text)| {
        if !has_inline_citations(kind) {
            return Vec::new();
        }
        let mut found = Vec::new();
        if !numbers.is_empty() {
            find_numbered(text, '[', ']', &numbers, &mut found);
            if numbered_parentheses {
                find_numbered(text, '(', ')', &numbers, &mut found);
            }
        }
        if !authors.is_empty() {
            find_author_year(text, &authors, &mut found);
        }
        found.sort_by_key(|(start, _)| *start);
        let mut targets = HashSet::new();
        found.into_iter().map(|(_, link)| link).filter(|link| targets.insert(link.target)).collect()
    }).collect()
}

fn has_inline_citations(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Paragraph | BlockKind::Caption | BlockKind::Heading | BlockKind::Title)
}

fn delimited(text: &str, open: char, close: char) -> impl Iterator<Item = (usize, &str)> {
    text.match_indices(open).filter_map(move |(start, _)| {
        let (inside, _) = text[start + open.len_utf8()..].split_once(close)?;
        (!inside.contains(open)).then_some((start, inside))
    })
}

fn reference_number(text: &str) -> Option<(u32, &str)> {
    let (value, remainder) = if let Some(rest) = text.strip_prefix('[') {
        rest.split_once(']')?
    } else if let Some(rest) = text.strip_prefix('(') {
        rest.split_once(')')?
    } else {
        let end = text.bytes().take_while(u8::is_ascii_digit).count();
        let (value, rest) = text.split_at(end);
        let remainder = rest.strip_prefix(['.', ')'])?;
        if value.len() > 3 || !remainder.starts_with(char::is_whitespace) {
            return None;
        }
        (value, remainder)
    };
    let value = number(value)?;
    let remainder = remainder.trim_start();
    (!remainder.is_empty()).then_some((value, remainder))
}

fn number(value: &str) -> Option<u32> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|&value| value > 0)
}

fn number_ranges(group: &str) -> Option<Vec<(u32, u32)>> {
    group.split([',', ';', '，', '；']).map(|part| {
        if let Some((first, last)) = part.split_once(['-', '‐', '‑', '–', '—', '−']) {
            let first = number(first)?;
            let last = number(last)?;
            (first <= last).then_some((first, last))
        } else {
            number(part).map(|value| (value, value))
        }
    }).collect()
}

fn find_numbered(
    text: &str,
    open: char,
    close: char,
    numbers: &Numbered,
    found: &mut Vec<(usize, CitationLink)>,
) {
    for (start, group) in delimited(text, open, close) {
        if open == '(' && is_numbered_object(&text[..start]) {
            continue;
        }
        let Some(ranges) = number_ranges(group) else { continue };
        for (first, last) in ranges {
            for (&number, &target) in numbers.range(first..=last) {
                if let Some(target) = target {
                    found.push((start, CitationLink { label: format!("{open}{number}{close}"), target }));
                }
            }
        }
    }
}

fn is_numbered_object(prefix: &str) -> bool {
    let prefix = prefix.trim_end();
    if prefix.is_empty() || prefix.ends_with(['=', '+', '−', '×', '÷', '/', '→']) {
        return true;
    }
    let last = prefix.trim_end_matches(['.', ':']).rsplit(|ch: char| !ch.is_alphabetic()).next().unwrap_or_default();
    matches!(last.to_ascii_lowercase().as_str(),
        "eq" | "eqs" | "eqn" | "equation" | "equations" | "formula" | "fig" | "figs" | "figure" | "figures"
        | "table" | "tables" | "section" | "sections" | "appendix" | "theorem" | "theorems" | "lemma"
        | "lemmas" | "definition" | "definitions" | "algorithm" | "algorithms" | "step" | "steps" | "case" | "cases"
    )
}

fn words(text: &str) -> impl DoubleEndedIterator<Item = &str> {
    text.split(|ch: char| !ch.is_alphanumeric() && !matches!(ch, '-' | '‐' | '‑' | '\'' | '’' | '\u{300}'..='\u{36f}'))
        .filter(|word| !word.is_empty())
}

fn year(word: &str) -> Option<&str> {
    let bytes = word.as_bytes();
    match bytes {
        [b'1', b'7'..=b'9', a, b] | [b'2', b'0', a, b] if a.is_ascii_digit() && b.is_ascii_digit() => Some(word),
        [b'1', b'7'..=b'9', a, b, b'a'..=b'z'] | [b'2', b'0', a, b, b'a'..=b'z']
            if a.is_ascii_digit() && b.is_ascii_digit() => Some(word),
        _ => None,
    }
}

fn surname_key(text: &str) -> Option<String> {
    let first = text.chars().next()?;
    if !(matches!(first, '\u{3400}'..='\u{9fff}') || first.is_uppercase() && text.chars().nth(1).is_some()) {
        return None;
    }
    Some(text.chars().filter(|ch| !matches!(ch, '\u{300}'..='\u{36f}')).flat_map(char::to_lowercase).collect())
}

fn reference_author_year(text: &str) -> Option<(String, String)> {
    // The author-date formats place a year after the author list and before the title.
    for word in words(text) {
        let Some(year) = year(word) else { continue };
        let start = word.as_ptr() as usize - text.as_ptr() as usize;
        let prefix = text[..start].trim_end();
        let after = text[start + word.len()..].trim_start();
        if !prefix.ends_with(['.', '(', ',']) || !after.starts_with(['.', ')', ','])
            || prefix.contains(['“', '”', '"'])
        {
            // A year embedded in an IEEE-style quoted article title is not a date.
            continue;
        }
        let first = prefix.split([',', '&']).next()?;
        let first = first.split_once(" and ").map_or(first, |(author, _)| author);
        return Some((surname_key(cited_surname(first)?)?, year.to_owned()));
    }
    None
}

fn find_author_year(text: &str, authors: &AuthorYears, found: &mut Vec<(usize, CitationLink)>) {
    for word in words(text) {
        let Some(cited_year) = year(word) else { continue };
        let start = word.as_ptr() as usize - text.as_ptr() as usize;
        let prefix = text[..start].trim_end();
        let literal_year = prefix.strip_suffix('(').is_some();
        let mut context = if let Some(before) = prefix.strip_suffix('(') {
            before.trim_end()
        } else if let Some((_, inside)) = prefix.rsplit_once('(').filter(|(_, inside)| !inside.contains(')')) {
            inside.rsplit(';').next().unwrap_or(inside)
        } else if let Some(before) = prefix.strip_suffix(',') {
            before
        } else {
            continue;
        };

        let after = text[start + word.len()..].trim_start();
        if literal_year && !after.starts_with(')') {
            continue;
        }
        // Several years can share one author list, as in (Li et al., 2020a, 2020b).
        context = context.trim_end_matches(|ch: char| ch == ',' || ch.is_whitespace());
        while let Some((before, preceding_year)) = context.rsplit_once(',') {
            if year(preceding_year.trim()).is_some() {
                context = before;
            } else {
                break;
            }
        }
        let Some(surname) = cited_surname(context) else { continue };
        let Some(normalized) = surname_key(surname) else { continue };
        if let Some(Some(target)) = authors.get(&(normalized, cited_year.to_owned())) {
            found.push((start, CitationLink { label: format!("{surname} {cited_year}"), target: *target }));
        }
    }
}

fn cited_surname(context: &str) -> Option<&str> {
    let context = context.trim_end_matches(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '.'));
    let mut components = words(context).rev();
    let last = components.next()?;
    if last.eq_ignore_ascii_case("al") && components.next()?.eq_ignore_ascii_case("et") {
        return components.next();
    }
    // In "Smith and Jones" or "Smith & Jones" the cited work is keyed by the first surname.
    let before_last = &context[..last.as_ptr() as usize - context.as_ptr() as usize];
    if let Some(before) = before_last.trim_end().strip_suffix('&') {
        return words(before).next_back();
    }
    if let Some(before) = before_last.trim_end().strip_suffix(" and") {
        return words(before).next_back();
    }
    Some(last)
}

/// Converts explicit web addresses and publication identifiers into browser-safe source links.
pub fn source_links(text: &str) -> Vec<SourceLink> {
    let mut links = Vec::new();
    let mut urls = HashSet::new();
    let mut after_arxiv = false;
    for word in text.split_whitespace() {
        let token = trim_source_token(word.trim_start_matches(['(', '[', '{', '<', '"', '“', '‘']));
        let lower = token.to_ascii_lowercase();
        let link = if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("www.") {
            web_link(token, lower.starts_with("www."))
        } else if lower.starts_with("arxiv:") || after_arxiv {
            arxiv_link(token.get(if lower.starts_with("arxiv:") { 6 } else { 0 }..).unwrap_or_default())
        } else {
            let token = if lower.starts_with("doi:") { &token[4..] } else { token };
            doi_link(token)
        };
        after_arxiv = token.eq_ignore_ascii_case("arxiv:");
        if let Some(link) = link.filter(|link| urls.insert(link.url.clone())) {
            links.push(link);
        }
    }
    links
}

fn trim_source_token(value: &str) -> &str {
    let mut value = value.trim_end_matches(['.', ',', ';', '"', '”', '’', '>', '。', '，', '；']);
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        while value.ends_with(close) && value.matches(close).count() > value.matches(open).count() {
            value = &value[..value.len() - close.len_utf8()];
        }
    }
    value.trim_end_matches(['.', ',', ';', '。', '，', '；'])
}

fn web_link(token: &str, bare: bool) -> Option<SourceLink> {
    let mut parsed = if bare { Url::parse(&format!("https://{token}")) } else { Url::parse(token) }.ok()?;
    if !parsed.username().is_empty() || parsed.password().is_some() || !public_web_host(parsed.host()?) {
        return None;
    }
    if matches!(parsed.host_str(), Some("doi.org" | "dx.doi.org")) {
        if let Some(link) = doi_link(parsed.path().trim_start_matches('/')) {
            return Some(link);
        }
    }
    if matches!(parsed.host_str(), Some("arxiv.org" | "www.arxiv.org")) {
        if let Some(link) = parsed.path().strip_prefix("/abs/").and_then(arxiv_link) {
            return Some(link);
        }
    }
    if parsed.scheme() == "http" {
        parsed.set_scheme("https").ok()?;
    }
    let label = format!("{}{}", parsed.host_str()?, parsed.path().trim_end_matches('/'));
    Some(SourceLink { label, url: parsed.into() })
}

fn public_web_host(host: Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => domain.contains('.') && !domain.trim_end_matches('.').ends_with(".localhost")
            && !domain.trim_end_matches('.').ends_with(".local") && domain.trim_end_matches('.') != "localhost",
        Host::Ipv4(address) => !address.is_loopback() && !address.is_private() && !address.is_link_local()
            && !address.is_unspecified() && !address.is_broadcast() && !address.is_multicast(),
        Host::Ipv6(address) => !address.is_loopback() && !address.is_unique_local() && !address.is_unicast_link_local()
            && !address.is_unspecified() && !address.is_multicast()
            && address.to_ipv4_mapped().is_none_or(|v4| public_web_host(Host::Ipv4(v4))),
    }
}

fn doi_link(id: &str) -> Option<SourceLink> {
    let (prefix, suffix) = id.strip_prefix("10.")?.split_once('/')?;
    if !(4..=9).contains(&prefix.len()) || !prefix.bytes().all(|byte| byte.is_ascii_digit())
        || suffix.is_empty() || suffix.chars().any(|ch| ch.is_control() || ch.is_whitespace() || ch == '\\')
    {
        return None;
    }
    let mut url = Url::parse("https://doi.org").ok()?;
    url.set_path(&id.to_ascii_lowercase());
    Some(SourceLink { label: format!("DOI {id}"), url: url.into() })
}

fn arxiv_link(id: &str) -> Option<SourceLink> {
    let (identifier, version) = id.split_once('v').map_or((id, None), |(base, version)| (base, Some(version)));
    if version.is_some_and(|digits| digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())) {
        return None;
    }
    let (date, serial) = identifier.split_once('.')?;
    if date.len() != 4 || !date.bytes().all(|byte| byte.is_ascii_digit())
        || !date[2..].parse::<u8>().ok().is_some_and(|month| (1..=12).contains(&month))
        || !(4..=5).contains(&serial.len()) || !serial.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some(SourceLink { label: format!("arXiv {id}"), url: format!("https://arxiv.org/abs/{id}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(body: &str, bibliography: &[&str]) -> Vec<Vec<CitationLink>> {
        resolve(std::iter::once((BlockKind::Paragraph, body))
            .chain(bibliography.iter().map(|&text| (BlockKind::Reference, text))))
    }

    #[test]
    fn resolves_explicit_bracket_groups_and_parenthesized_style() {
        let links = links("Prior work [1, 3–4] and (1; 2). Eq. (5) and (99) are here.", &[
            "[1] Alpha", "[2] Beta", "[3] Gamma", "[4] Delta", "[5] Epsilon",
        ]);
        assert_eq!(links[0].iter().map(|entry| entry.target).collect::<Vec<_>>(), [1, 3, 4, 2]);
        assert!(links[1].is_empty());
    }

    #[test]
    fn resolves_author_year_and_year_suffix_without_guessing_ambiguities() {
        let links = links("(Li et al., 2020b, 2020a; Chen and Smith, 2024). Smith (2022) and Li (2020).", &[
            "Toby Jia-Jun Li, Tom Mitchell, and Brad Myers. 2020a. Learning from instructions.",
            "Yang Li, Jiacong He, and Xin Zhou. 2020b. Mapping instructions.",
            "Chen, Y. and Smith, J. (2024). An Example.",
            "Jane Smith. 2022. Old paper.",
            "John Smith. 2022. Other paper.",
        ]);
        assert_eq!(links[0].iter().map(|entry| entry.target).collect::<Vec<_>>(), [2, 1, 3]);
    }

    #[test]
    fn normalizes_identifiers_and_rejects_nonpublic_web_destinations() {
        let links = source_links("doi:10.1000/Example. (https://dx.doi.org/10.1000/example), arXiv: 2402.04615v2. https://example.com/paper; http://127.0.0.1/x https://user@example.com/private file:///secret");
        assert_eq!(links.iter().map(|entry| entry.url.as_str()).collect::<Vec<_>>(), [
            "https://doi.org/10.1000/example", "https://arxiv.org/abs/2402.04615v2", "https://example.com/paper",
        ]);
    }
}
