use super::{Painter, UI_FONT_COUNT, rc, scale};
use super::selection::TextLine;
use crate::math_layout::{MathNode, MathStyle, parse, parse_display};
use std::mem::zeroed;
use std::ptr::null;
use std::sync::OnceLock;
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*};

pub(super) struct NativeMath {
    width: i32,
    ascent: i32,
    descent: i32,
    marks: Vec<Mark>,
}

enum Mark {
    Text { content: Vec<u16>, font: usize, origin: POINT },
    Rule(RECT),
    Stroke([POINT; 5]),
}

const MATH_SIZES: [i32; 4] = [18, 16, 13, 10];
pub(super) const FONT_COUNT: usize = 3 * MATH_SIZES.len();

pub(super) fn font_style(index: usize) -> (&'static str, i32, bool) {
    const FILES: [&[u8]; 3] = [
        include_bytes!("../data/latex-fonts/latinmodern-math.otf"),
        include_bytes!("../data/latex-fonts/lmroman10-italic.otf"),
        include_bytes!("../data/latex-fonts/lmmono10-regular.otf"),
    ];
    const FAMILIES: [(&str, &str, bool); 3] = [
        ("Latin Modern Math", "Cambria Math", false),
        ("LM Roman 10", "Times New Roman", true),
        ("LM Mono 10", "Courier New", false),
    ];
    static REGISTERED: OnceLock<[bool; 3]> = OnceLock::new();
    let available = REGISTERED.get_or_init(|| FILES.map(|bytes| {
        // Keep the embedded faces private to the reader process.
        let mut count = 0;
        let resource = unsafe {
            AddFontMemResourceEx(bytes.as_ptr().cast(), bytes.len() as u32, null(), &raw mut count)
        };
        !resource.is_null() && count > 0
    }));
    let family = index / MATH_SIZES.len();
    let (embedded, fallback, italic) = FAMILIES[family];
    let face = if available[family] { embedded } else { fallback };
    (face, MATH_SIZES[index % MATH_SIZES.len()], italic)
}

fn font_index(style: MathStyle, size: usize) -> usize {
    let family = match style {
        MathStyle::Roman => 0,
        MathStyle::Italic => 1,
        MathStyle::Monospace => 2,
    };
    UI_FONT_COUNT + family * MATH_SIZES.len() + size
}

impl NativeMath {
    fn empty() -> Self {
        Self { width: 0, ascent: 0, descent: 0, marks: Vec::new() }
    }

    pub(super) fn display(p: &Painter, text: &str, width: i32, dpi: u32) -> Option<Self> {
        let node = parse_display(text)?;
        (0..3).map(|size| Self::layout(p, &node, size, dpi, None)).find(|layout| layout.width <= width)
    }

    fn inline(p: &Painter, text: &str, width: i32, dpi: u32) -> Option<Self> {
        if let Some(node) = parse(text) {
            return (1..3).map(|size| Self::layout(p, &node, size, dpi, None))
                .find(|layout| layout.width <= width);
        }
        let layout = Self::math_text(p, text, 1, dpi, MathStyle::Roman);
        (layout.width <= width).then_some(layout)
    }

    fn layout(p: &Painter, node: &MathNode, size: usize, dpi: u32, style: Option<MathStyle>) -> Self {
        let child_size = (size + 1).min(MATH_SIZES.len() - 1);
        let em = scale(MATH_SIZES[size], dpi);
        let gap = scale(2, dpi).max(1);
        match node {
            MathNode::Text(text) => match style {
                Some(style) => Self::math_text(p, text, size, dpi, style),
                None => Self::default_math_text(p, text, size, dpi),
            },
            MathNode::Styled { style, content } => Self::layout(p, content, size, dpi, Some(*style)),
            MathNode::Sequence(nodes) => {
                let mut result = Self::empty();
                for node in nodes {
                    let child = Self::layout(p, node, size, dpi, style);
                    result.place(child, result.width, 0);
                }
                result
            }
            MathNode::Scripts { base, sub, sup } => {
                let mut result = Self::layout(p, base, size, dpi, style);
                let (ascent, descent) = (result.ascent, result.descent);
                let x = result.width + gap / 2;
                let script_size = child_size.max(2);
                let superscript_bottom = sup.as_ref().map(|node| {
                    let child = Self::layout(p, node, script_size, dpi, style);
                    let shift = (ascent * 3 / 5).max(child.descent + em / 5);
                    let bottom = child.descent - shift;
                    result.place(child, x, -shift);
                    bottom
                });
                if let Some(node) = sub {
                    let child = Self::layout(p, node, script_size, dpi, style);
                    let mut shift = descent.max(child.ascent - em * 2 / 5);
                    if let Some(bottom) = superscript_bottom {
                        shift = shift.max(bottom + gap + child.ascent);
                    }
                    result.place(child, x, shift);
                }
                result
            }
            MathNode::Fraction { numerator, denominator } => {
                let numerator = Self::layout(p, numerator, child_size, dpi, style);
                let denominator = Self::layout(p, denominator, child_size, dpi, style);
                let width = numerator.width.max(denominator.width) + 2 * gap;
                let rule_y = -em / 4;
                let thickness = scale(1, dpi).max(1);
                let nx = (width - numerator.width) / 2;
                let ny = rule_y - gap - numerator.descent;
                let dx = (width - denominator.width) / 2;
                let dy = rule_y + thickness + gap + denominator.ascent;
                let mut result = Self::empty();
                result.place(numerator, nx, ny);
                result.place(denominator, dx, dy);
                result.width = width;
                result.marks.push(Mark::Rule(rc(0, rule_y, width, rule_y + thickness)));
                result
            }
            MathNode::Radical(node) => {
                let child = Self::layout(p, node, size, dpi, style);
                let inset = (em / 2).max(5);
                let top = -child.ascent - gap;
                let bottom = (child.descent - gap).max(0);
                let right = inset + child.width + gap;
                let mut result = Self::empty();
                result.place(child, inset, 0);
                result.width = right;
                result.ascent = -top;
                result.marks.push(Mark::Stroke([
                    POINT { x: 0, y: -em / 5 },
                    POINT { x: inset / 4, y: -em / 4 },
                    POINT { x: inset / 2, y: bottom },
                    POINT { x: inset - 1, y: top },
                    POINT { x: right, y: top },
                ]));
                result
            }
        }
    }

    fn default_math_text(p: &Painter, text: &str, size: usize, dpi: u32) -> Self {
        let mut result = Self::empty();
        let mut start = 0;
        for (index, character) in text.char_indices() {
            let end = index + character.len_utf8();
            let letter = character.is_ascii_alphabetic() || matches!(character, 'α'..='ω' | 'ϑ' | 'ϕ' | 'ϱ' | 'ϵ');
            if letter && !text[..index].ends_with(char::is_alphabetic) && !text[end..].starts_with(char::is_alphabetic) {
                let prefix = Self::math_text(p, &text[start..index], size, dpi, MathStyle::Roman);
                result.place(prefix, result.width, 0);
                let variable = Self::math_text(p, &text[index..end], size, dpi, MathStyle::Italic);
                result.place(variable, result.width, 0);
                start = end;
            }
        }
        let suffix = Self::math_text(p, &text[start..], size, dpi, MathStyle::Roman);
        result.place(suffix, result.width, 0);
        result
    }

    fn math_text(p: &Painter, text: &str, size: usize, dpi: u32, style: MathStyle) -> Self {
        if text.is_empty() {
            return Self::empty();
        }
        let font = font_index(style, size);
        let em = scale(MATH_SIZES[size], dpi);
        let tall_operator = text.contains(['∫', '∮', '∑', '∏', '⋂', '⋃']);
        let ascent = if tall_operator { em } else { em * 4 / 5 };
        let descent = if tall_operator { em * 2 / 5 } else { em / 4 };
        Self::text(p, text, font, ascent, descent)
    }

    fn text(p: &Painter, text: &str, font: usize, ascent: i32, descent: i32) -> Self {
        let content: Vec<_> = text.encode_utf16().collect();
        let width = p.run_width(&content, font);
        Self::text_run(content, font, width, ascent, descent)
    }

    fn text_run(content: Vec<u16>, font: usize, width: i32, ascent: i32, descent: i32) -> Self {
        let marks = vec![Mark::Text { content, font, origin: POINT { x: 0, y: 0 } }];
        Self { width, ascent, descent, marks }
    }

    fn place(&mut self, part: Self, x: i32, baseline: i32) {
        self.width = self.width.max(x + part.width);
        self.ascent = self.ascent.max(part.ascent - baseline);
        self.descent = self.descent.max(part.descent + baseline);
        self.marks.extend(part.marks.into_iter().map(|mut mark| {
            match &mut mark {
                Mark::Text { origin, .. } => {
                    origin.x += x;
                    origin.y += baseline;
                }
                Mark::Rule(bounds) => {
                    bounds.left += x;
                    bounds.right += x;
                    bounds.top += baseline;
                    bounds.bottom += baseline;
                }
                Mark::Stroke(points) => {
                    for point in points {
                        point.x += x;
                        point.y += baseline;
                    }
                }
            }
            mark
        }));
    }

    pub(super) fn height(&self) -> i32 {
        self.ascent + self.descent
    }

    pub(super) fn draw_centered(&self, p: &Painter, bounds: RECT, ink: u32) {
        let x = bounds.left + (bounds.right - bounds.left - self.width).max(0) / 2;
        let y = bounds.top + (bounds.bottom - bounds.top - self.height()).max(0) / 2;
        self.draw(p, x, y + self.ascent, ink);
    }

    fn draw(&self, p: &Painter, x: i32, baseline: i32, ink: u32) {
        for mark in &self.marks {
            match mark {
                Mark::Text { content, font, origin } => {
                    p.draw_run(content, *font, x + origin.x, baseline + origin.y, ink);
                }
                Mark::Rule(bounds) => {
                    p.fill(rc(x + bounds.left, baseline + bounds.top, x + bounds.right, baseline + bounds.bottom), ink);
                }
                Mark::Stroke(points) => unsafe {
                    let pen = SelectObject(p.dc, GetStockObject(DC_PEN));
                    let previous_color = SetDCPenColor(p.dc, ink);
                    let points = points.map(|point| POINT { x: x + point.x, y: baseline + point.y });
                    Polyline(p.dc, points.as_ptr(), points.len() as i32);
                    SetDCPenColor(p.dc, previous_color);
                    SelectObject(p.dc, pen);
                },
            }
        }
    }
}

pub(super) struct NativeText {
    lines: Vec<FlowLine>,
    gap: i32,
    height: i32,
}

impl NativeText {
    pub(super) fn layout(p: &Painter, runs: Vec<(&str, bool)>, font: usize, width: i32, dpi: u32) -> Option<Self> {
        let mut flow = Flow::new(p, font, width);
        let mut offset = 0;
        for (text, math) in runs {
            let end = offset + text.len();
            if let Some(formula) = math.then(|| NativeMath::inline(p, text, width, dpi)).flatten() {
                let stops = vec![(0, offset), (formula.width, end)];
                flow.push(FlowLine { drawing: formula, stops });
            } else {
                flow.prose(text, offset);
            }
            offset = end;
        }
        if !flow.line.drawing.marks.is_empty() {
            flow.finish_line(offset);
        }
        if flow.lines.iter().all(|line| line.drawing.marks.is_empty()) {
            return None;
        }
        let gap = scale(3, dpi);
        let height = flow.lines.iter().map(|line| line.drawing.height()).sum::<i32>()
            + flow.lines.len().saturating_sub(1) as i32 * gap;
        Some(Self { lines: flow.lines, gap, height })
    }

    pub(super) fn height(&self) -> i32 {
        self.height
    }

    pub(super) fn draw(&self, p: &Painter, bounds: RECT, align: u32, ink: u32) {
        let saved = unsafe { SaveDC(p.dc) };
        let mut visible = bounds;
        if saved != 0 {
            unsafe {
                IntersectClipRect(p.dc, bounds.left, bounds.top, bounds.right, bounds.bottom);
                GetClipBox(p.dc, &mut visible);
            }
        }
        for (line, rect) in self.visible_lines(bounds, align, visible) {
            line.drawing.draw(p, rect.left, rect.top + line.drawing.ascent, ink);
        }
        if saved != 0 {
            unsafe { RestoreDC(p.dc, saved) };
        }
    }

    pub(super) fn selection_lines(&self, bounds: RECT, align: u32, clip: RECT) -> Vec<TextLine> {
        self.visible_lines(bounds, align, clip).map(|(line, bounds)| TextLine {
            bounds,
            stops: line.stops.iter().map(|&(x, offset)| (bounds.left + x, offset)).collect(),
        }).collect()
    }

    fn visible_lines(&self, bounds: RECT, align: u32, clip: RECT) -> impl Iterator<Item = (&FlowLine, RECT)> {
        let top = bounds.top.max(clip.top);
        let bottom = bounds.bottom.min(clip.bottom);
        self.lines.iter().scan(bounds.top, move |y, line| {
            if *y >= bottom {
                return None;
            }
            let width = line.drawing.width;
            let left = if align == DT_CENTER {
                bounds.left + (bounds.right - bounds.left - width).max(0) / 2
            } else {
                bounds.left
            };
            let rect = rc(left, *y, left + width, *y + line.drawing.height());
            *y = rect.bottom + self.gap;
            Some((line, rect))
        }).filter(move |(_, rect)| rect.bottom > top)
    }
}

struct FlowLine {
    drawing: NativeMath,
    stops: Vec<(i32, usize)>,
}

impl FlowLine {
    fn empty() -> Self {
        Self { drawing: NativeMath::empty(), stops: Vec::new() }
    }
}

struct Flow<'a> {
    p: &'a Painter,
    font: usize,
    width: i32,
    ascent: i32,
    descent: i32,
    space: i32,
    pending_space: bool,
    lines: Vec<FlowLine>,
    line: FlowLine,
}

impl<'a> Flow<'a> {
    fn new(p: &'a Painter, font: usize, width: i32) -> Self {
        let (ascent, descent) = p.run_metrics(font);
        let space = p.run_width(&[32], font);
        Self {
            p, font, width, ascent, descent, space, pending_space: false,
            lines: Vec::new(), line: FlowLine::empty(),
        }
    }

    fn push(&mut self, part: FlowLine) {
        let has_content = !self.line.drawing.marks.is_empty();
        let space = if self.pending_space && has_content { self.space } else { 0 };
        let x = if has_content && self.line.drawing.width + space + part.drawing.width > self.width {
            self.finish_line(0);
            0
        } else {
            self.line.drawing.width + space
        };
        for (advance, offset) in part.stops {
            let stop = (x + advance, offset);
            if self.line.stops.last() != Some(&stop) {
                self.line.stops.push(stop);
            }
        }
        self.line.drawing.place(part.drawing, x, 0);
        self.pending_space = false;
    }

    fn prose(&mut self, text: &str, offset: usize) {
        let mut word_start = 0;
        let mut after_cjk = false;
        for (index, ch) in text.char_indices() {
            if ch.is_whitespace() && ch != '\u{00a0}' {
                self.word(&text[word_start..index], offset + word_start);
                if ch == '\n' {
                    self.finish_line(offset + index);
                } else if ch != '\r' {
                    self.pending_space = true;
                }
                word_start = index + ch.len_utf8();
                after_cjk = false;
            } else if !closing_punctuation(ch) {
                let cjk = is_cjk(ch);
                if after_cjk || cjk {
                    self.word(&text[word_start..index], offset + word_start);
                    word_start = index;
                }
                after_cjk = cjk;
            }
        }
        self.word(&text[word_start..], offset + word_start);
    }

    fn word(&mut self, text: &str, offset: usize) {
        if text.is_empty() {
            return;
        }
        let part = self.word_part(text, offset);
        if part.drawing.width <= self.width {
            self.push(part);
            return;
        }
        let mut start = 0;
        let mut length = 0;
        for (index, ch) in text.char_indices() {
            let glyph_width = self.p.run_width(ch.encode_utf16(&mut [0; 2]), self.font);
            if index > start && length + glyph_width > self.width {
                self.push(self.word_part(&text[start..index], offset + start));
                start = index;
                length = 0;
            }
            length += glyph_width;
        }
        self.push(self.word_part(&text[start..], offset + start));
    }

    fn word_part(&self, text: &str, offset: usize) -> FlowLine {
        let content: Vec<_> = text.encode_utf16().collect();
        let (width, stops) = self.p.run_stops(&content, text, self.font, offset);
        let drawing = NativeMath::text_run(content, self.font, width, self.ascent, self.descent);
        FlowLine { drawing, stops }
    }

    fn finish_line(&mut self, offset: usize) {
        let mut line = std::mem::replace(&mut self.line, FlowLine::empty());
        line.drawing.ascent = line.drawing.ascent.max(self.ascent);
        line.drawing.descent = line.drawing.descent.max(self.descent);
        if line.stops.is_empty() {
            line.stops.push((0, offset));
        }
        self.lines.push(line);
        self.pending_space = false;
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(ch, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}' | '\u{20000}'..='\u{323af}')
}

fn closing_punctuation(ch: char) -> bool {
    matches!(ch, '，' | '。' | '、' | '；' | '：' | '？' | '！' | '）' | '］' | '】' | '》' | '”' | '’' | '」' | '』')
}

impl Painter {
    fn run_stops(&self, content: &[u16], text: &str, font: usize, offset: usize) -> (i32, Vec<(i32, usize)>) {
        let mut extents = vec![0; content.len()];
        let mut fit = 0;
        let mut size = SIZE { cx: 0, cy: 0 };
        let measured = unsafe {
            let previous = SelectObject(self.dc, self.fonts[font]);
            let measured = GetTextExtentExPointW(self.dc, content.as_ptr(), content.len() as i32,
                i32::MAX, &mut fit, extents.as_mut_ptr(), &mut size);
            SelectObject(self.dc, previous);
            measured != 0 && fit as usize == content.len()
        };
        let width = if measured { size.cx } else { self.run_width(content, font) }.max(0);
        let mut stops = Vec::with_capacity(content.len() + 1);
        stops.push((0, offset));
        let mut units = 0;
        let mut previous = 0;
        for (index, ch) in text.char_indices() {
            units += ch.len_utf16();
            let advance = if units == content.len() {
                width
            } else if measured {
                extents[units - 1].clamp(previous, width)
            } else {
                (i64::from(width) * units as i64 / content.len() as i64) as i32
            };
            previous = advance;
            stops.push((advance, offset + index + ch.len_utf8()));
        }
        (width, stops)
    }

    pub(super) fn run_width(&self, text: &[u16], font: usize) -> i32 {
        unsafe {
            let previous = SelectObject(self.dc, self.fonts[font]);
            let mut size: SIZE = zeroed();
            GetTextExtentPoint32W(self.dc, text.as_ptr(), text.len() as i32, &mut size);
            SelectObject(self.dc, previous);
            size.cx
        }
    }

    fn run_metrics(&self, font: usize) -> (i32, i32) {
        unsafe {
            let previous = SelectObject(self.dc, self.fonts[font]);
            let mut metrics: TEXTMETRICW = zeroed();
            GetTextMetricsW(self.dc, &mut metrics);
            SelectObject(self.dc, previous);
            (metrics.tmAscent, metrics.tmDescent)
        }
    }

    fn draw_run(&self, text: &[u16], font: usize, x: i32, baseline: i32, ink: u32) {
        unsafe {
            let previous = SelectObject(self.dc, self.fonts[font]);
            let align = SetTextAlign(self.dc, TA_LEFT | TA_BASELINE);
            SetBkMode(self.dc, TRANSPARENT as i32);
            SetTextColor(self.dc, ink);
            TextOutW(self.dc, x, baseline, text.as_ptr(), text.len() as i32);
            SetTextAlign(self.dc, align);
            SelectObject(self.dc, previous);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_body_sized_display_math_with_italic_and_typewriter_faces() {
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        assert!(!dc.is_null());
        let fonts = super::super::make_fonts(96);
        let painter = Painter { dc, fonts };
        let formula = r"C_{1} = \texttt{read/summarize},\quad C_{2} = \texttt{delete with explicit approval}.";
        let math = NativeMath::display(&painter, formula, 640, 96).unwrap();
        assert!(math.height() <= 24);
        let has_font = |style, size| {
            let expected = font_index(style, size);
            math.marks.iter().any(|mark| {
                matches!(mark, Mark::Text { font, .. } if *font == expected)
            })
        };
        assert!(has_font(MathStyle::Italic, 0));
        assert!(has_font(MathStyle::Monospace, 0));
        assert!(has_font(MathStyle::Roman, 2));
        unsafe { DeleteDC(dc) };
        for font in fonts {
            unsafe { DeleteObject(font) };
        }
    }

    #[test]
    fn selection_uses_utf8_boundaries_and_keeps_formula_together() {
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        assert!(!dc.is_null());
        let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        let painter = Painter { dc, fonts: [font; super::super::FONT_COUNT] };
        let layout = NativeText::layout(&painter, vec![("甲𝑥", false), ("x_i", true), ("乙", false)], 0, 640, 96);
        unsafe { DeleteDC(dc) };

        let bounds = rc(20, 40, 660, 140);
        let lines = layout.unwrap().selection_lines(bounds, DT_CENTER, bounds);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(line.bounds.left > bounds.left);
        assert_eq!(line.stops.iter().map(|&(_, byte)| byte).collect::<Vec<_>>(), [0, 3, 7, 10, 13]);
        assert!(line.stops.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    }
}
