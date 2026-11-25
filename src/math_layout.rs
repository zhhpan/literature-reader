#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathStyle {
    Italic,
    Monospace,
    Roman,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MathNode {
    Text(String),
    Sequence(Vec<MathNode>),
    Styled {
        style: MathStyle,
        content: Box<MathNode>,
    },
    Scripts {
        base: Box<MathNode>,
        sub: Option<Box<MathNode>>,
        sup: Option<Box<MathNode>>,
    },
    Fraction {
        numerator: Box<MathNode>,
        denominator: Box<MathNode>,
    },
    Radical(Box<MathNode>),
}

pub fn parse(text: &str) -> Option<MathNode> {
    let text = text.trim();
    let delimited = [("$$", "$$"), ("$", "$"), (r"\(", r"\)"), (r"\[", r"\]")]
        .into_iter()
        .find_map(|(start, end)| text.strip_prefix(start)?.strip_suffix(end));
    let text = delimited.unwrap_or(text);
    if text.len() > 4_096 || text.contains('\u{fffd}') {
        return None;
    }
    Parser { input: text.chars().peekable(), tex: delimited.is_some() || text.contains('\\') }.sequence(false, 0)
}

pub fn parse_display(text: &str) -> Option<MathNode> {
    if text.lines().count() != 1 || text.contains(['\u{221a}', '\u{23af}', '\u{25a1}', '\u{25af}']) {
        return None;
    }
    parse(text)
}

struct Parser<'a> {
    input: std::iter::Peekable<std::str::Chars<'a>>,
    tex: bool,
}

impl Parser<'_> {
    fn sequence(&mut self, group: bool, depth: u8) -> Option<MathNode> {
        if depth > 16 {
            return None;
        }
        let mut nodes = Vec::new();
        loop {
            match self.input.peek().copied() {
                None if group => return None,
                None => break,
                Some('}') if group => { self.input.next(); break; }
                Some('}') if self.tex => return None,
                _ => {}
            }
            let node = self.atom(depth)?;
            let node = self.scripts(node, depth)?;
            if let (Some(MathNode::Text(left)), MathNode::Text(right)) = (nodes.last_mut(), &node) {
                left.push_str(right);
            } else {
                nodes.push(node);
            }
        }
        match nodes.len() {
            0 => None,
            1 => nodes.pop(),
            _ => Some(MathNode::Sequence(nodes)),
        }
    }

    fn atom(&mut self, depth: u8) -> Option<MathNode> {
        if depth > 16 {
            return None;
        }
        match self.input.next()? {
            '{' if self.tex => self.sequence(true, depth + 1),
            '\\' => self.command(depth),
            '_' | '^' | '$' | '&' => None,
            character if unicode_script(character).is_some() => None,
            character if character.is_whitespace() => Some(MathNode::Text(" ".into())),
            character if !character.is_control() => Some(MathNode::Text(character.to_string())),
            _ => None,
        }
    }

    fn scripts(&mut self, base: MathNode, depth: u8) -> Option<MathNode> {
        let mut sub = None;
        let mut sup = None;
        while let Some(character) = self.input.peek().copied() {
            if character.is_whitespace() {
                let mut after_spaces = self.input.clone();
                while after_spaces.next_if(|character| character.is_whitespace()).is_some() {}
                if after_spaces.peek().is_some_and(|&next| matches!(next, '_' | '^') || unicode_script(next).is_some()) {
                    self.input = after_spaces;
                    continue;
                }
            }
            let (upper, script) = match character {
                '_' | '^' => {
                    self.input.next();
                    self.skip_spaces();
                    let argument = if self.input.peek() == Some(&'{') {
                        self.group(depth)?
                    } else {
                        self.atom(depth + 1)?
                    };
                    (character == '^', argument)
                }
                _ => {
                    let Some((upper, _)) = unicode_script(character) else { break };
                    let mut script = String::new();
                    while let Some((next_upper, glyph)) = self.input.peek().copied().and_then(unicode_script) {
                        if next_upper != upper { break; }
                        self.input.next();
                        script.push(glyph);
                    }
                    (upper, MathNode::Text(script))
                }
            };
            let slot = if upper { &mut sup } else { &mut sub };
            if slot.replace(Box::new(script)).is_some() {
                return None;
            }
        }
        if sub.is_some() || sup.is_some() {
            if matches!(&base, MathNode::Text(text) if text.trim().is_empty()) {
                return None;
            }
            Some(MathNode::Scripts { base: Box::new(base), sub, sup })
        } else {
            Some(base)
        }
    }

    fn command(&mut self, depth: u8) -> Option<MathNode> {
        let mut name = String::new();
        while let Some(character) = self.input.next_if(char::is_ascii_alphabetic) {
            name.push(character);
        }
        if name.is_empty() {
            let character = self.input.next()?;
            let text = match character {
                ',' | ';' | ':' | ' ' => " ".into(),
                '!' => String::new(),
                '{' | '}' | '_' | '^' | '$' | '&' | '%' | '#' => character.to_string(),
                _ => return None,
            };
            return Some(MathNode::Text(text));
        }
        self.skip_spaces();
        match name.as_str() {
            "frac" | "dfrac" | "tfrac" => Some(MathNode::Fraction {
                numerator: Box::new(self.group(depth)?),
                denominator: Box::new(self.group(depth)?),
            }),
            "sqrt" => Some(MathNode::Radical(Box::new(self.group(depth)?))),
            "mathit" | "mathrm" => Some(MathNode::Styled {
                style: if name == "mathit" { MathStyle::Italic } else { MathStyle::Roman },
                content: Box::new(self.group(depth)?),
            }),
            "texttt" | "text" => Some(MathNode::Styled {
                style: if name == "texttt" { MathStyle::Monospace } else { MathStyle::Roman },
                content: Box::new(MathNode::Text(self.literal_group()?)),
            }),
            "quad" => Some(MathNode::Text("\u{2003}".into())),
            "qquad" => Some(MathNode::Text("\u{2003}\u{2003}".into())),
            // With no \middle or delimiter sizing, retain the delimiters at the current size.
            "left" | "right" => match self.input.peek() {
                Some('(' | ')' | '[' | ']' | '|' | '\\') => self.atom(depth + 1),
                Some('.') => { self.input.next(); Some(MathNode::Text(String::new())) }
                _ => None,
            },
            _ => Some(MathNode::Text(symbol(&name)?.to_owned())),
        }
    }

    fn group(&mut self, depth: u8) -> Option<MathNode> {
        self.skip_spaces();
        (self.input.next()? == '{').then_some(())?;
        self.sequence(true, depth + 1)
    }

    fn literal_group(&mut self) -> Option<String> {
        self.skip_spaces();
        (self.input.next()? == '{').then_some(())?;
        let mut text = String::new();
        let mut depth = 0;
        while let Some(character) = self.input.next() {
            match character {
                '}' if depth == 0 => return Some(text),
                '}' => depth -= 1,
                '{' => depth += 1,
                '\\' => {
                    let escaped = self.input.next()?;
                    if !"\\{}_^$&%#".contains(escaped) {
                        return None;
                    }
                    text.push(escaped);
                    continue;
                }
                character if character.is_control() => return None,
                _ => {}
            }
            text.push(character);
        }
        None
    }

    fn skip_spaces(&mut self) {
        while self.input.next_if(|character| character.is_whitespace()).is_some() {}
    }
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ", "epsilon" => "ϵ",
        "varepsilon" => "ε", "zeta" => "ζ", "eta" => "η", "theta" => "θ", "vartheta" => "ϑ",
        "iota" => "ι", "kappa" => "κ", "lambda" => "λ", "mu" => "μ", "nu" => "ν",
        "xi" => "ξ", "pi" => "π", "rho" => "ρ", "varrho" => "ϱ", "sigma" => "σ",
        "tau" => "τ", "upsilon" => "υ", "phi" => "ϕ", "varphi" => "φ", "chi" => "χ",
        "psi" => "ψ", "omega" => "ω", "Gamma" => "Γ", "Delta" => "Δ", "Theta" => "Θ",
        "Lambda" => "Λ", "Xi" => "Ξ", "Pi" => "Π", "Sigma" => "Σ", "Phi" => "Φ",
        "Psi" => "Ψ", "Omega" => "Ω", "sum" => "∑", "prod" => "∏", "int" => "∫",
        "partial" => "∂", "nabla" => "∇", "infty" => "∞", "cdot" => "·", "times" => "×",
        "div" => "÷", "pm" => "±", "mp" => "∓", "le" | "leq" => "≤", "ge" | "geq" => "≥",
        "ne" | "neq" => "≠", "equiv" => "≡", "approx" => "≈", "sim" => "∼", "propto" => "∝",
        "in" => "∈", "notin" => "∉", "subseteq" => "⊆", "subset" => "⊂", "cup" => "∪", "cap" => "∩",
        "land" | "wedge" => "∧", "lor" | "vee" => "∨", "forall" => "∀", "exists" => "∃",
        "rightarrow" | "to" => "→", "leftarrow" => "←", "mapsto" => "↦", "Rightarrow" => "⇒",
        "leftrightarrow" => "↔", "Leftrightarrow" | "iff" => "⇔", "ldots" => "…", "cdots" => "⋯",
        "langle" => "⟨", "rangle" => "⟩", "vert" => "|", "Vert" => "‖", "prime" => "′",
        "log" => "log", "ln" => "ln", "sin" => "sin", "cos" => "cos", "tan" => "tan", "exp" => "exp",
        "max" => "max", "min" => "min", "lim" => "lim",
        _ => return None,
    })
}

fn unicode_script(character: char) -> Option<(bool, char)> {
    let (upper, from, to) = match character {
        '₀'..='₉' | '₊' | '₋' | '₌' | '₍' | '₎' | 'ₐ' | 'ₑ' | 'ₕ' | 'ᵢ' | 'ⱼ' | 'ₖ'
        | 'ₗ' | 'ₘ' | 'ₙ' | 'ₒ' | 'ₚ' | 'ᵣ' | 'ₛ' | 'ₜ' | 'ᵤ' | 'ᵥ' | 'ₓ' => {
            (false, "₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎ₐₑₕᵢⱼₖₗₘₙₒₚᵣₛₜᵤᵥₓ", "0123456789+−=()aehijklmnoprstuvx")
        }
        _ => (true, "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾ⁱⁿ", "0123456789+−=()in"),
    };
    from.chars().zip(to.chars()).find_map(|(source, normalized)| {
        (source == character).then_some((upper, normalized))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_scripts_and_fraction_structure() {
        let MathNode::Fraction { numerator, denominator } = parse(r"\(\frac{x_i²}{\sqrt{\alpha}}\)").unwrap() else {
            panic!("fraction was not preserved");
        };
        assert!(matches!(*numerator, MathNode::Scripts { sub: Some(_), sup: Some(_), .. }));
        assert_eq!(*denominator, MathNode::Radical(Box::new(MathNode::Text("α".into()))));
    }

    #[test]
    fn rejects_unsupported_layouts_instead_of_flattening_them() {
        assert!(parse(r"\begin{matrix}x&y\end{matrix}").is_none());
        assert!(parse(r"\frac{x}{y").is_none());
        assert!(parse_display("x = y\nz").is_none());
        assert!(parse_display("σ^{′} = σ (1)").is_some());
    }

    #[test]
    fn keeps_set_braces_in_pdf_text_and_parses_braced_scripts() {
        let MathNode::Sequence(parts) = parse_display("S = {x_{ij} | x > 0}").unwrap() else {
            panic!("inline scripts were not preserved");
        };
        assert_eq!(parts[0], MathNode::Text("S = {".into()));
        assert!(matches!(&parts[1], MathNode::Scripts { sub: Some(_), .. }));
        assert_eq!(parts[2], MathNode::Text(" | x > 0}".into()));
        assert!(matches!(parse("x _ i ^ 2"), Some(MathNode::Scripts { sub: Some(_), sup: Some(_), .. })));
    }

    #[test]
    fn retains_code_typography_and_literal_punctuation_in_a_display_equation() {
        let text = r"C_{1} = \texttt{read/summarize},\quad C_{2} = \texttt{work_id = \{value\}}.";
        let MathNode::Sequence(parts) = parse_display(text).unwrap() else { panic!("expected an equation sequence") };
        let code: Vec<_> = parts.iter().filter_map(|part| match part {
            MathNode::Styled { style: MathStyle::Monospace, content } => match &**content {
                MathNode::Text(text) => Some(text.as_str()),
                _ => None,
            },
            _ => None,
        }).collect();
        assert_eq!(code, ["read/summarize", "work_id = {value}"]);
        assert_eq!(parts.iter().filter(|part| matches!(part, MathNode::Scripts { .. })).count(), 2);
        assert!(matches!(parse(r"\mathit{A}_{1}"), Some(MathNode::Scripts { base, .. })
            if matches!(*base, MathNode::Styled { style: MathStyle::Italic, .. })));
    }
}
