use crate::hyphenation::{Hyphenation, LINE_HYPHENS};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockKind {
    Title,
    Author,
    Heading,
    Paragraph,
    Equation,
    Caption,
    ReferenceHeading,
    Reference,
    Figure,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Bounds {
    fn from_size(left: f32, top: f32, width: f32, height: f32) -> Self {
        Self { left, top, right: left + width, bottom: top + height }
    }

    fn width(self) -> f32 {
        self.right - self.left
    }

    fn height(self) -> f32 {
        self.bottom - self.top
    }

    fn center(self) -> f32 {
        (self.left + self.right) * 0.5
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn overlap_x(self, other: Self) -> f32 {
        self.right.min(other.right) - self.left.max(other.left)
    }
}

#[derive(Debug)]
pub struct Block {
    pub kind: BlockKind,
    pub page: usize,
    pub text: String,
    pub bounds: Bounds,
}

#[derive(Debug)]
pub struct Document {
    pub title: String,
    pub pages: usize,
    pub blocks: Vec<Block>,
    /// Asset block index to the caption block index, when the pairing is identifiable.
    pub asset_captions: HashMap<usize, usize>,
    /// First block index to the exclusive end of a same-level list.
    pub list_groups: HashMap<usize, usize>,
    /// Display equation block index to math text with recoverable script positions.
    pub native_equations: HashMap<usize, String>,
    /// Alternate paragraph text for rendering and translation, with isolated inline scripts marked.
    pub native_inline: HashMap<usize, String>,
    /// Source page and bounds of paragraph continuations outside the starting column.
    pub paragraph_continuations: HashMap<usize, Vec<(usize, Bounds)>>,
    pub notices: Vec<String>,
}

struct Page {
    number: usize,
    width: f32,
    height: f32,
    groups: Vec<Vec<Line>>,
}

struct Line {
    bounds: Bounds,
    text: String,
    last_word_right: Option<f32>,
    font_heights: Vec<f32>,
    cell_breaks: Vec<f32>,
    words: Vec<Word>,
}

struct Word {
    bounds: Bounds,
    text: Range<usize>,
}

impl Line {
    fn font_height(&self) -> f32 {
        self.font_heights.get(self.font_heights.len() / 2).copied().unwrap_or(self.bounds.height())
    }
}

pub fn extract(path: &Path) -> Result<Document, String> {
    let path = input_path(path)?;
    let (layout, images) = std::thread::scope(|scope| {
        let images = scope.spawn(|| raster_layout(&path));
        let result = run(command("pdftotext").args(["-enc", "UTF-8", "-tsv"]).arg(&path).arg("-"));
        (result, images.join().unwrap_or_else(|_| Err("读取图片位置失败".into())))
    });
    let pages = parse_layout(&layout?)?;
    let hyphenation = Hyphenation::from_lines(pages.iter().flat_map(|page| page.groups.iter().flatten())
        .map(|line| line.text.as_str()));
    let size = body_size(&pages);
    let repeated_margins = repeated_margin_lines(&pages);
    let main_page = pages.iter().take(2).find(|page| page.groups.iter().flatten().any(|line| starts_abstract(&line.text)))
        .unwrap_or(&pages[0]);
    let title_group = find_title(main_page, size);
    let title = title_group.map(|group| join_lines(&main_page.groups[group], None))
        .or_else(|| metadata_title(&path))
        .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().into_owned());
    let mut notices = Vec::new();
    let images = images.unwrap_or_else(|error| {
        notices.push(format!("图片位置读取失败，仍可查看原始页面。{error}"));
        HashMap::new()
    });

    let mut blocks = Vec::new();
    let mut references = false;
    let mut empty_pages = Vec::new();
    let mut assets_by_page = HashMap::new();
    let columns: Vec<_> = pages.iter().map(page_columns).collect();
    for (page, page_columns) in pages.iter().zip(&columns) {
        if page.groups.is_empty() {
            empty_pages.push(page.number.to_string());
        }
        let title_here = title_group.filter(|_| page.number == main_page.number);
        let mut page_blocks = text_blocks(page, size, &repeated_margins, title_here, &hyphenation);
        join_aligned_fragments(&mut page_blocks);
        join_equation_fragments(&mut page_blocks);
        join_display_assignments(&mut page_blocks, page);
        attach_equation_numbers(&mut page_blocks, page.width);
        let assets = add_assets(page, &mut page_blocks, page_columns, images.get(&page.number));
        if !assets.is_empty() {
            assets_by_page.insert(page.number, assets);
        }
        for mut block in order_page(page, page_blocks, page_columns) {
            if block.kind == BlockKind::Heading && is_reference_heading(&block.text) {
                references = true;
                block.kind = BlockKind::ReferenceHeading;
            } else if block.kind == BlockKind::Heading && is_appendix_boundary(&block.text) {
                references = false;
            } else if references && matches!(block.kind, BlockKind::Paragraph | BlockKind::Heading | BlockKind::Equation) {
                block.kind = BlockKind::Reference;
            }
            blocks.push(block);
        }
    }
    if !empty_pages.is_empty() {
        let preview = empty_pages.iter().take(8).map(String::as_str).collect::<Vec<_>>().join("、");
        let suffix = if empty_pages.len() > 8 { format!("等 {} 页", empty_pages.len()) } else { "页".into() };
        notices.push(format!("第 {preview} {suffix}没有可提取的文字，可查看原始页面。"));
    }
    let blocks = merge_reference_fragments(blocks, &columns);
    let mut paragraph_continuations = HashMap::new();
    let (blocks, native_inline) = merge_paragraph_fragments(blocks, &pages, &columns, size, &hyphenation, &mut paragraph_continuations);
    let asset_captions = link_asset_captions(&blocks, &assets_by_page);
    let list_groups = collect_list_groups(&blocks, size);
    let mut native_equations = HashMap::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.kind == BlockKind::Equation
            && let Some(page) = block.page.checked_sub(1).and_then(|index| pages.get(index))
            && let Some(source) = native_equation(block, page)
        {
            native_equations.insert(index, source);
        }
    }
    Ok(Document { title, pages: pages.len(), blocks, asset_captions, list_groups, native_equations, native_inline, paragraph_continuations, notices })
}

/// Renders the page as binary RGB Portable Pixmap (P6) data.
pub fn render_page(path: &Path, page: usize, dpi: u32) -> Result<Vec<u8>, String> {
    render(path, page, dpi, None)
}

/// Renders the block's original appearance as binary RGB Portable Pixmap (P6) data.
pub fn render_block(path: &Path, block: &Block, dpi: u32) -> Result<Vec<u8>, String> {
    let padding = if matches!(block.kind, BlockKind::Figure | BlockKind::Table) { 1.0 } else { 4.0 };
    render(path, block.page, dpi, Some((block.bounds, padding)))
}

fn render(path: &Path, page: usize, dpi: u32, crop: Option<(Bounds, f32)>) -> Result<Vec<u8>, String> {
    if page == 0 {
        return Err("PDF 页码从 1 开始".into());
    }
    let path = input_path(path)?;
    let page = page.to_string();
    let dpi = dpi.clamp(36, 300);
    let dpi_arg = dpi.to_string();
    let mut process = command("pdftoppm");
    process.args(["-singlefile", "-f", &page, "-l", &page, "-r", &dpi_arg]);
    if let Some((bounds, padding)) = crop {
        let scale = dpi as f32 / 72.0;
        let left = ((bounds.left - padding) * scale).floor().max(0.0) as u32;
        let top = ((bounds.top - padding) * scale).floor().max(0.0) as u32;
        let right = ((bounds.right + padding) * scale).ceil() as u32;
        let bottom = ((bounds.bottom + padding) * scale).ceil() as u32;
        let width = right.saturating_sub(left).max(1).to_string();
        let height = bottom.saturating_sub(top).max(1).to_string();
        process.args(["-x", &left.to_string(), "-y", &top.to_string(), "-W", &width, "-H", &height]);
    }
    let ppm = run(process.arg(path))?;
    if !ppm.starts_with(b"P6\n") && !ppm.starts_with(b"P6\r\n") {
        return Err("Poppler 未输出有效的页面图像".into());
    }
    Ok(ppm)
}

fn input_path(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|error| format!("无法定位 PDF 文件：{error}"))
}

fn command(program: &str) -> Command {
    let mut process = Command::new(crate::runtime::poppler(program));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    process.env("LC_ALL", "C");
    process
}

fn run(process: &mut Command) -> Result<Vec<u8>, String> {
    let name = process.get_program().to_string_lossy().to_string();
    let Output { status, stdout, stderr } = process.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("找不到 {name}，请安装 Poppler 并将它加入 PATH。")
        } else {
            format!("无法启动 {name}：{error}")
        }
    })?;
    if status.success() {
        Ok(stdout)
    } else {
        let message = String::from_utf8_lossy(&stderr);
        let message = message.lines().find(|line| !line.trim().is_empty()).unwrap_or("没有错误详情");
        Err(format!("{name} 读取 PDF 失败：{message}"))
    }
}

fn metadata_title(path: &Path) -> Option<String> {
    let output = run(command("pdfinfo").args(["-enc", "UTF-8"]).arg(path)).ok()?;
    String::from_utf8_lossy(&output).lines()
        .filter_map(|line| line.strip_prefix("Title:"))
        .map(str::trim)
        .find(|title| !title.is_empty())
        .map(str::to_owned)
}

fn positive_bounds(values: [&str; 4]) -> Option<Bounds> {
    let [left, top, width, height] = values.map(|value| value.parse::<f32>().ok());
    let (left, top, width, height) = (left?, top?, width?, height?);
    if ![left, top, width, height].iter().all(|n| n.is_finite()) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(Bounds::from_size(left, top, width, height))
}

fn parse_layout(bytes: &[u8]) -> Result<Vec<Page>, String> {
    let source = String::from_utf8_lossy(bytes);
    let mut pages: Vec<Page> = Vec::new();
    for row in source.lines().skip(1) {
        let mut fields = row.splitn(12, '\t');
        let [level, number, _, _, _, _, left, top, width, height, _, text] = std::array::from_fn(|_| fields.next().unwrap_or_default());
        match level {
            "1" => {
                let (Ok(number), Some(bounds)) = (number.parse(), positive_bounds([left, top, width, height])) else {
                    continue;
                };
                pages.push(Page { number, width: bounds.width(), height: bounds.height(), groups: Vec::new() });
            }
            "3" => {
                if let Some(page) = pages.last_mut() {
                    page.groups.push(Vec::new());
                }
            }
            "4" => {
                if let (Some(group), Some(bounds)) = (
                    pages.last_mut().and_then(|page| page.groups.last_mut()),
                    positive_bounds([left, top, width, height]),
                ) {
                    group.push(Line {
                        bounds, text: String::new(), last_word_right: None, font_heights: Vec::new(),
                        cell_breaks: Vec::new(), words: Vec::new(),
                    });
                }
            }
            "5" => {
                if let (Some(line), Some(word)) = (
                    pages.last_mut().and_then(|page| page.groups.last_mut()).and_then(|group| group.last_mut()),
                    positive_bounds([left, top, width, height]),
                ) {
                    let token = if text.chars().any(char::is_control) {
                        std::borrow::Cow::Owned(text.chars().map(|character| if character.is_control() { ' ' } else { character }).collect::<String>())
                    } else {
                        std::borrow::Cow::Borrowed(text)
                    };
                    let token = token.trim();
                    if token.is_empty() {
                        continue;
                    }
                    let gap = line.last_word_right.map(|right| word.left - right);
                    if gap.is_some_and(|gap| gap >= (word.height() * 0.9).max(9.0)) {
                        line.cell_breaks.push(word.left);
                    }
                    let small_caps = gap.is_some_and(|gap| gap.abs() < 0.8)
                        && line.text.split_whitespace().next_back().is_some_and(|word| word.len() == 1 && word.starts_with(char::is_uppercase))
                        && token.chars().all(|character| character.is_ascii_uppercase());
                    if small_caps {
                        line.text.push_str(token);
                    } else {
                        append_word(&mut line.text, token);
                    }
                    line.words.push(Word { bounds: word, text: line.text.len() - token.len()..line.text.len() });
                    line.last_word_right = Some(word.right);
                    line.font_heights.push(word.height());
                }
            }
            _ => {}
        }
    }
    for page in &mut pages {
        for group in &mut page.groups {
            group.retain(|line| {
                !line.text.is_empty() && !(line.bounds.height() > 1.5 * line.bounds.width() && line.text.chars().count() > 6)
            });
            for line in group {
                line.font_heights.sort_unstable_by(f32::total_cmp);
            }
        }
        join_drop_caps(&mut page.groups);
    }
    if pages.is_empty() {
        Err("Poppler 未返回页面。请确认 PDF 可以打开，并使用支持 -tsv 的 Poppler。".into())
    } else {
        Ok(pages)
    }
}

fn join_drop_caps(groups: &mut Vec<Vec<Line>>) {
    for index in 0..groups.len() {
        let [cap] = groups[index].as_slice() else { continue };
        let [glyph] = cap.words.as_slice() else { continue };
        if cap.text.chars().count() != 1 || !cap.text.starts_with(char::is_uppercase) {
            continue;
        }
        let glyph = glyph.bounds;
        let Some((next, _)) = groups.iter().enumerate().filter_map(|(other, lines)| {
            let [first, second, ..] = lines.as_slice() else { return None };
            let head = first.words.first()?.bounds;
            let height = first.font_height();
            let gap = head.left - glyph.right;
            let offset = head.top - glyph.top;
            // A drop cap sits beside the first word and extends into the next line.
            (first.words.len() >= 3 && first.text.starts_with(char::is_lowercase)
                && glyph.left < head.left && glyph.height() >= height * 1.5
                && (-height * 0.35..=height * 0.6).contains(&gap)
                && (-height * 0.25..=glyph.height() * 0.45).contains(&offset)
                && head.bottom <= glyph.bottom + height * 0.2
                && second.bounds.top > head.top && second.bounds.top <= glyph.bottom + height * 0.2
                && second.bounds.left >= glyph.right - height * 0.35)
                .then_some((other, head.top))
        }).min_by(|(_, left), (_, right)| left.total_cmp(right)) else { continue };

        let cap = groups[index].pop().expect("isolated drop cap");
        // Keep body baselines and font sizes for paragraph and column detection.
        let first = &mut groups[next][0];
        let prefix_length = cap.text.len();
        first.text.insert_str(0, &cap.text);
        first.words[0].bounds = first.words[0].bounds.union(glyph);
        first.words[0].text.end += prefix_length;
        for word in &mut first.words[1..] {
            word.text = word.text.start + prefix_length..word.text.end + prefix_length;
        }
    }
    groups.retain(|group| !group.is_empty());
}

fn append_word(line: &mut String, word: &str) {
    if word.is_empty() {
        return;
    }
    let previous = line.chars().next_back();
    let current = word.chars().next();
    if let (Some(previous), Some(current)) = (previous, current) {
        if !("([{«".contains(previous) || ",.;:!?)]}»，。；：！？、）】".contains(current)
            || is_cjk(previous) && is_cjk(current))
        {
            line.push(' ');
        }
    }
    line.push_str(word);
}

fn is_cjk(character: char) -> bool {
    matches!(character, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}')
}

fn join_lines(lines: &[Line], hyphenation: Option<&Hyphenation>) -> String {
    let mut result = String::new();
    for line in lines {
        append_text_fragment(&mut result, &line.text, hyphenation);
    }
    result
}

fn append_text_fragment(result: &mut String, text: &str, hyphenation: Option<&Hyphenation>) {
    if result.ends_with('\u{00ad}') {
        result.pop();
        result.push_str(text);
    } else if result.ends_with(['-', '\u{2010}', '\u{2011}']) && text.starts_with(char::is_lowercase) {
        if hyphenation.is_some_and(|rules| rules.remove_line_hyphen(result, text)) {
            result.pop();
        }
        result.push_str(text);
    } else {
        append_word(result, text);
    }
}

fn body_size(pages: &[Page]) -> f32 {
    let mut frequencies = BTreeMap::new();
    for line in pages.iter().flat_map(|page| page.groups.iter().flatten()) {
        let height = line.font_height();
        if line.text.chars().count() >= 24 && (3.5..=24.0).contains(&height) {
            *frequencies.entry((height * 2.0).round() as u32).or_insert(0) += 1;
        }
    }
    frequencies.into_iter().max_by_key(|&(_, frequency)| frequency)
        .map_or(10.0, |(bucket, _)| bucket as f32 / 2.0)
}

fn find_title(page: &Page, body: f32) -> Option<usize> {
    let conventional = page.groups.iter().enumerate().filter_map(|(index, group)| {
        let first = group.first()?;
        let height = first.font_height();
        let text = join_lines(group, None);
        let letters = text.chars().filter(|character| character.is_alphabetic()).count();
        (first.bounds.top < page.height * 0.33 && height >= (body * 1.3).max(11.0) && letters >= 8
            && first.bounds.width() > page.width * 0.14
            && !known_heading(&text) && caption_type(&text).is_none()).then_some((index, height))
    }).max_by(|left, right| left.1.total_cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .map(|(index, _)| index);
    conventional.or_else(|| {
        page.groups.iter().enumerate()
            .filter(|(_, group)| cover_byline_band(page, group, body).is_some())
            .max_by(|(_, left), (_, right)| left[0].font_height().total_cmp(&right[0].font_height()))
            .map(|(index, _)| index)
    })
}

fn author_band(page: &Page, title: usize, body_size: f32) -> Option<(f32, f32)> {
    if let Some(band) = cover_byline_band(page, &page.groups[title], body_size) {
        return Some(band);
    }
    let title_end = page.groups[title].iter().map(|line| line.bounds.bottom).max_by(f32::total_cmp)?;
    let lines = || page.groups.iter().flatten().filter(|line| line.bounds.top >= title_end - 2.0);
    let body_start = lines().filter(|line| {
        starts_abstract(&line.text) || matches!(section_name(&line.text).as_str(), "introduction" | "引言")
    }).map(|line| line.bounds.top).min_by(f32::total_cmp)
        .unwrap_or_else(|| (title_end + body_size * 7.0).min(page.height * 0.40));
    let first_author = lines().filter(|line| {
        line.bounds.top < body_start - 1.0 && line.bounds.top <= title_end + body_size.max(9.0) * 5.0
            && is_byline(&line.text)
    }).map(|line| line.bounds.top).min_by(f32::total_cmp)?;
    Some((first_author - 3.0, body_start - 1.0))
}

fn cover_byline_band(page: &Page, headline: &[Line], body: f32) -> Option<(f32, f32)> {
    let [first, _, ..] = headline else { return None };
    let height = first.font_height();
    // Illustrated first pages can put a large headline across the image and the byline above it.
    if page.number != 1 || !(0.33..=0.62).contains(&(first.bounds.top / page.height))
        || height < (body * 2.25).max(22.0) || first.bounds.width() <= page.width * 0.27
        || caption_type(&first.text).is_some() || headline.iter().any(|line| {
            (line.bounds.left - first.bounds.left).abs() > body || line.font_height() < height * 0.8
        })
        || headline.iter().flat_map(|line| line.text.chars()).filter(|c| c.is_alphabetic()).count() < 20
    {
        return None;
    }
    page.groups.iter().filter_map(|group| {
        let start = group.first()?;
        let end = group.last()?;
        (start.bounds.top <= page.height * 0.28 && first.bounds.top - end.bounds.bottom >= body * 4.0
            && (start.bounds.left - first.bounds.left).abs() <= body
            && start.font_height() <= body * 1.25 && is_byline(&start.text)
            && group.iter().any(|line| is_author_affiliation(&line.text))
            && group.iter().all(|line| is_byline(&line.text) || is_author_affiliation(&line.text)))
            .then_some((start.bounds.top - 3.0, end.bounds.bottom + 1.0))
    }).max_by(|left, right| left.0.total_cmp(&right.0))
}

fn is_byline(text: &str) -> bool {
    let text = text.trim();
    if text.chars().count() > 160 || text.contains('@') || text.contains("://")
        || text.contains([':', ';', '=', '?', '!', '。', '；']) || known_heading(text)
        || is_author_affiliation(text)
    {
        return false;
    }
    let mut name_count = 0;
    let mut has_cjk_name = false;
    for token in text.split(|character: char| character.is_whitespace() || [',', '、', '，', '·'].contains(&character)) {
        let token = token.trim_matches(|character: char| !character.is_alphabetic());
        if token.is_empty() || matches!(token, "and" | "de" | "del" | "den" | "der" | "di" | "la" | "van" | "von" | "by" | "By") {
            continue;
        }
        let compact_cjk = token.chars().all(is_cjk) && (2..=6).contains(&token.chars().count());
        if !compact_cjk && (!token.starts_with(char::is_uppercase) || token.chars().any(|c| {
            !c.is_alphabetic() && !matches!(c, '\u{0300}'..='\u{036f}') && !"-.'’".contains(c)
        })) {
            return false;
        }
        has_cjk_name |= compact_cjk;
        name_count += 1;
    }
    (2..=24).contains(&name_count) || name_count == 1 && has_cjk_name
}

fn is_author_affiliation(text: &str) -> bool {
    if text.chars().count() > 260 {
        return false;
    }
    let lower = text.to_lowercase();
    if lower.contains('@') || lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("orcid") {
        return true;
    }
    let words = lower.split(|character: char| !character.is_alphabetic());
    words.into_iter().any(|word| matches!(word, "university" | "université" | "universität" | "department" | "institute"
        | "laboratory" | "laboratories" | "college" | "academy" | "inc" | "corporation" | "ltd" | "affiliation"))
        || ["大学", "学院", "研究院", "研究所", "实验室"].iter().any(|part| lower.contains(part))
}

fn normalized_margin(text: &str) -> String {
    text.chars().filter(|character| !character.is_ascii_digit()).collect::<String>().trim().to_lowercase()
}

fn is_margin(line: &Line, page: &Page) -> bool {
    line.bounds.top < page.height * 0.08 || line.bounds.bottom > page.height * 0.92
}

fn repeated_margin_lines(pages: &[Page]) -> HashSet<String> {
    let mut seen = HashMap::new();
    for page in pages {
        let margins = page.groups.iter().filter(|group| group.len() == 1)
            .map(|group| &group[0]).filter(|line| is_margin(line, page))
            .map(|line| normalized_margin(&line.text)).collect::<HashSet<_>>();
        for text in margins {
            *seen.entry(text).or_insert(0) += 1;
        }
    }
    seen.into_iter().filter(|(_, count)| *count >= 2).map(|(text, _)| text).collect()
}

#[derive(Clone, Copy)]
enum ListMarker {
    Bullet(char),
    Number(u16, char, bool),
    Letter(u8, bool),
}

impl ListMarker {
    fn from_text(text: &str) -> Option<Self> {
        let marker = text.split_whitespace().next()?;
        if marker.starts_with(['•', '◦', '▪', '‣', '●', '○']) || matches!(marker, "-" | "–" | "—") {
            return marker.chars().next().map(Self::Bullet);
        }
        let ending = marker.chars().next_back()?;
        let label = marker.strip_suffix(['.', ')'])?;
        let parenthesized = label.starts_with('(');
        let label = label.strip_prefix('(').unwrap_or(label);
        if (1..=3).contains(&label.len()) && label.bytes().all(|byte| byte.is_ascii_digit()) {
            return label.parse().ok().map(|number| Self::Number(number, ending, parenthesized));
        }
        (ending == ')' && label.len() == 1 && label.starts_with(char::is_lowercase))
            .then(|| Self::Letter(label.as_bytes()[0], parenthesized))
    }

    fn follows(self, previous: Self) -> bool {
        match (previous, self) {
            (Self::Bullet(a), Self::Bullet(b)) => a == b,
            (Self::Number(a, ending_a, open_a), Self::Number(b, ending_b, open_b)) => {
                ending_a == ending_b && open_a == open_b && a + 1 == b
            }
            (Self::Letter(a, open_a), Self::Letter(b, open_b)) => open_a == open_b && a + 1 == b,
            _ => false,
        }
    }
}

fn starts_list_item(text: &str) -> bool {
    ListMarker::from_text(text).is_some()
}

fn collect_list_groups(blocks: &[Block], body: f32) -> HashMap<usize, usize> {
    let mut groups = HashMap::new();
    let mut start = 0;
    let size = body.max(8.0);
    for end in 1..=blocks.len() {
        let previous = &blocks[end - 1];
        let continues = blocks.get(end).is_some_and(|next| {
            previous.kind == BlockKind::Paragraph && next.kind == BlockKind::Paragraph
                && previous.page == next.page
                && (previous.bounds.left - next.bounds.left).abs() <= size * 0.5
                && (-1.0..=size * 1.8).contains(&(next.bounds.top - previous.bounds.bottom))
                && ListMarker::from_text(previous.text.as_str()).zip(ListMarker::from_text(&next.text))
                    .is_some_and(|(previous, next)| next.follows(previous))
        });
        if !continues {
            if end - start > 1 {
                groups.insert(start, end);
            }
            start = end;
        }
    }
    groups
}

fn text_blocks(page: &Page, body: f32, margins: &HashSet<String>, title: Option<usize>, hyphenation: &Hyphenation) -> Vec<Block> {
    let mut blocks = Vec::new();
    let leading = line_spacing(page).unwrap_or(body.max(9.0) * 1.12);
    let authors = title.and_then(|title| author_band(page, title, body));
    let classify = |line: &Line| {
        if authors.is_some_and(|(start, end)| (start..end).contains(&line.bounds.top))
            && (is_byline(&line.text) || is_author_affiliation(&line.text))
        {
            BlockKind::Author
        } else {
            line_kind(line, body)
        }
    };
    for (index, group) in page.groups.iter().enumerate() {
        if title == Some(index) {
            blocks.push(make_block(page.number, BlockKind::Title, group, hyphenation));
            continue;
        }
        if group.len() == 1 && is_margin(&group[0], page) {
            let text = &group[0].text;
            if text.chars().all(|character| character.is_ascii_digit()) || margins.contains(&normalized_margin(text)) {
                continue;
            }
        }
        let mut start = 0;
        let mut kind = classify(&group[0]);
        for index in 1..group.len() {
            let line = &group[index];
            let last = &group[index - 1];
            let next_kind = classify(line);
            let gap = line.bounds.top - last.bounds.bottom;
            let baseline_gap = line.bounds.top - last.bounds.top;
            let new_indent = line.bounds.left - last.bounds.left > body.max(8.0) * 0.8;
            let starts_item = next_kind == BlockKind::Paragraph && starts_list_item(&line.text);
            let item = (kind == BlockKind::Paragraph).then(|| ListMarker::from_text(&group[start].text)).flatten();
            let continues_item = item.is_some() && next_kind == BlockKind::Paragraph && !starts_item
                && group[start].words.get(1).is_some_and(|word| (line.bounds.left - word.bounds.left).abs() <= body * 0.55);
            let starts_indented_paragraph = new_indent && kind == BlockKind::Paragraph
                && (!continues_item && gap >= 0.0 || gap >= -body * 0.25
                    && line.text.starts_with(char::is_uppercase)
                    && last.text.trim_end_matches(['”', '’', '"', '\'', ')', ']']).ends_with(['.', '!', '?'])
                    && group[start..index].iter().any(|earlier| earlier.bounds.right - last.bounds.right >= body * 1.7));
            let introduces_list = next_kind == BlockKind::Paragraph && !starts_item
                && line.text.ends_with([':', '：']) && last.text.ends_with(['.', '?', '!', '。', '？', '！'])
                && item.is_some_and(|item| group.get(index + 1).is_some_and(|next| {
                    (next.bounds.left - group[start].bounds.left).abs() <= body * 0.5
                        && ListMarker::from_text(&next.text).is_some_and(|next| next.follows(item))
                }));
            if kind == BlockKind::Caption && next_kind != BlockKind::Caption && baseline_gap < leading * 1.42 {
                let label = &group[start];
                let initial = caption_label(&label.text).filter(|(_, tail)| tail.is_empty())
                    .map_or(0.0, |_| label.font_height());
                if line.font_height() <= last.font_height().max(initial) * 1.16 {
                    continue;
                }
            }
            if kind == BlockKind::Heading && next_kind == BlockKind::Paragraph
                && last.text.ends_with('-') && line.text.starts_with(char::is_lowercase)
                && (0.0..=leading * 1.32).contains(&baseline_gap)
                && (line.bounds.left - group[start].bounds.left).abs() <= body * 3.0
            {
                continue;
            }
            let starts_section = matches!(next_kind, BlockKind::Caption | BlockKind::Heading);
            let changes_equation = (kind == BlockKind::Equation) != (next_kind == BlockKind::Equation);
            let displaced_equation = kind == BlockKind::Equation && next_kind == BlockKind::Equation
                && (line.bounds.left - last.bounds.left).abs() > body && baseline_gap < body * 1.15;
            let changes_authorship = (kind == BlockKind::Author) != (next_kind == BlockKind::Author);
            let leaves_caption = kind == BlockKind::Caption;
            if starts_section || changes_equation || displaced_equation || changes_authorship || leaves_caption || kind == BlockKind::Heading
                || baseline_gap > leading * 1.42 || starts_item || introduces_list
                || starts_indented_paragraph
                || starts_reference_entry(&line.text)
            {
                blocks.push(make_block(page.number, kind, &group[start..index], hyphenation));
                start = index;
                kind = next_kind;
            }
        }
        blocks.push(make_block(page.number, kind, &group[start..], hyphenation));
    }
    blocks
}

fn line_spacing(page: &Page) -> Option<f32> {
    let mut frequencies = BTreeMap::new();
    for pair in page.groups.iter().flat_map(|group| group.windows(2)) {
        let first = &pair[0];
        let second = &pair[1];
        let gap = second.bounds.top - first.bounds.top;
        if (5.0..=26.0).contains(&gap) && first.text.chars().count() >= 24 && second.text.chars().count() >= 24 {
            *frequencies.entry((gap * 2.0).round() as u32).or_insert(0) += 1;
        }
    }
    frequencies.into_iter().max_by_key(|&(_, frequency)| frequency).map(|(bucket, _)| bucket as f32 * 0.5)
}

fn make_block(page: usize, kind: BlockKind, lines: &[Line], hyphenation: &Hyphenation) -> Block {
    let bounds = lines.iter().fold(lines[0].bounds, |bounds, line| bounds.union(line.bounds));
    let text = if kind == BlockKind::Author {
        lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join("\n")
    } else if kind == BlockKind::Caption && lines.len() > 1
        && caption_label(&lines[0].text).is_some_and(|(_, tail)| tail.is_empty())
    {
        format!("{}\n{}", lines[0].text, join_lines(&lines[1..], None))
    } else {
        join_lines(lines, (kind == BlockKind::Paragraph).then_some(hyphenation))
    };
    Block { kind, page, text, bounds }
}

fn line_kind(line: &Line, body: f32) -> BlockKind {
    if caption_type(&line.text).is_some() {
        BlockKind::Caption
    } else if looks_like_equation(&line.text) {
        BlockKind::Equation
    } else if is_heading(line, body) {
        BlockKind::Heading
    } else {
        BlockKind::Paragraph
    }
}

fn caption_type(text: &str) -> Option<bool> {
    let (figure, tail) = caption_label(text.lines().next()?)?;
    (tail.is_empty() || tail.starts_with(['.', ':', '：', '．', '。', '–', '—'])).then_some(figure)
}

fn caption_label(text: &str) -> Option<(bool, &str)> {
    let lower = text.to_lowercase();
    let prefixes = [("figure ", true), ("fig. ", true), ("fig ", true), ("图", true), ("table ", false), ("表", false)];
    prefixes.into_iter().find_map(|(prefix, figure)| {
        lower.strip_prefix(prefix)?;
        let remainder = text[prefix.len()..].trim_start();
        let candidate = remainder.strip_prefix(['S', 's']).unwrap_or(remainder);
        let digits = candidate.find(|character: char| !character.is_ascii_digit()).unwrap_or(candidate.len());
        let roman = candidate.find(|character: char| !"IVXLCDM".contains(character)).unwrap_or(candidate.len());
        let end = if digits != 0 { digits } else { roman };
        (1..=12).contains(&end).then(|| (figure, candidate[end..].trim_start()))
    })
}

fn strip_section_number(text: &str) -> &str {
    let Some((label, name)) = text.split_once(' ') else { return text; };
    if is_section_number(label) { name.trim_start() } else { text }
}

fn is_section_number(label: &str) -> bool {
    let number = label.trim_end_matches(['.', ')']);
    if number.is_empty() || number.len() > 8 {
        return false;
    }
    let arabic = number.split('.').all(|part| !part.is_empty() && part.len() <= 2 && part.chars().all(|c| c.is_ascii_digit()));
    let letter = label.ends_with('.') && (number.chars().all(|character| "IVXLCDM".contains(character))
        || number.len() == 1 && number.starts_with(char::is_uppercase));
    arabic || letter
}

fn section_name(text: &str) -> String {
    strip_section_number(text).trim_end_matches(['.', ':', '：']).to_lowercase()
}

fn is_reference_heading(text: &str) -> bool {
    matches!(section_name(text).as_str(), "references" | "bibliography" | "works cited" | "参考文献")
}

fn starts_appendix(text: &str) -> bool {
    let name = section_name(text);
    name == "appendix" || name.starts_with("appendix ") || name.starts_with("附录")
}

fn is_appendix_boundary(text: &str) -> bool {
    if starts_appendix(text) {
        return true;
    }
    let Some((label, title)) = text.split_once(' ') else { return false; };
    // Major lettered appendices often omit the word "Appendix" (for example, "A Data Collection").
    label.len() == 1 && label.starts_with(|character: char| character.is_ascii_uppercase())
        && title.chars().count() >= 6 && title.starts_with(char::is_uppercase)
        && !title.contains([',', '.', ':', '@'])
}

fn starts_abstract(text: &str) -> bool {
    let name = section_name(text);
    name == "abstract" || name.starts_with("abstract—") || name.starts_with("abstract:") || name.starts_with("摘要")
}

fn known_heading(text: &str) -> bool {
    is_reference_heading(text) || starts_appendix(text) || matches!(section_name(text).as_str(),
        "abstract" | "introduction" | "background" | "method" | "methods" | "methodology" | "related work"
        | "results" | "evaluation" | "discussion" | "conclusion" | "conclusions" | "acknowledgment" | "acknowledgments"
        | "acknowledgement" | "acknowledgements" | "摘要" | "引言" | "背景" | "方法" | "结果" | "结论" | "致谢")
}

fn is_heading(line: &Line, body: f32) -> bool {
    let text = &line.text;
    if known_heading(text) {
        return true;
    }
    let name = strip_section_number(text);
    text.chars().count() <= 105 && name.chars().filter(|c| c.is_alphabetic()).count() >= 6
        && text.split_whitespace().count() <= 12
        && !text.ends_with(['.', ',', ';', '?', '!', '。', '；'])
        && name.chars().next().is_some_and(|character| character.is_uppercase() || is_cjk(character))
        && (name.len() < text.len() && line.font_height() >= body.max(8.0) * 0.92
            || !text.contains(',') && line.font_height() >= (body * 1.17).max(9.0))
}

fn looks_like_equation(text: &str) -> bool {
    if text.contains("://") || text.contains("&") && text.contains(".php?") {
        return false;
    }
    let operators = "=<>≤≥≠≡≈∈∉∑∏∫√⊂⊆⊢⇒⇔↦∀∃×±∧∨→←∂∞∥⇥∪∩∝⇝";
    let operator_count = text.chars().filter(|character| operators.contains(*character)).count();
    let words = || text.split(|character: char| !character.is_ascii_alphabetic());
    let prose = words().filter(|word| word.len() >= 3 && !word.bytes().all(|byte| byte.is_ascii_uppercase())).count();
    operator_count > 0 && prose <= 2 + operator_count.min(2) && text.chars().count() < 155
        && !words().any(|word| matches!(word.to_ascii_lowercase().as_str(), "each" | "the" | "by" | "for"
            | "from" | "using" | "with" | "while" | "that" | "these" | "those" | "respectively" | "pixels"))
}

fn join_aligned_fragments(blocks: &mut Vec<Block>) {
    for index in 0..blocks.len() {
        let block = &blocks[index];
        if !matches!(block.kind, BlockKind::Paragraph | BlockKind::Heading | BlockKind::Caption) {
            continue;
        }
        let text = block.text.as_str();
        let section_prefix = is_section_number(text);
        let appendix_prefix = text.len() == 1 && text.starts_with(|character: char| character.is_ascii_uppercase());
        let short_caption = block.kind == BlockKind::Caption && text.split_whitespace().count() <= 2;
        if !section_prefix && !appendix_prefix && !short_caption {
            continue;
        }
        let start = block.bounds;
        let adjacent = blocks.iter().enumerate().filter(|(other, block)| {
            let bounds = block.bounds;
            *other != index && !block.text.is_empty() && (bounds.top - start.top).abs() < 2.0
                && (0.0..24.0).contains(&(bounds.left - start.right))
                && (block.kind == BlockKind::Heading || block.kind == BlockKind::Paragraph)
                && (!appendix_prefix || block.kind == BlockKind::Heading)
                && (!section_prefix || text.ends_with('.') || block.kind == BlockKind::Heading)
        }).min_by(|(_, left), (_, right)| left.bounds.left.total_cmp(&right.bounds.left)).map(|(other, _)| other);
        if let Some(other) = adjacent {
            let mut title = std::mem::take(&mut blocks[index].text);
            append_word(&mut title, &blocks[other].text);
            blocks[index].text = title;
            blocks[index].bounds = start.union(blocks[other].bounds);
            if section_prefix || appendix_prefix {
                blocks[index].kind = BlockKind::Heading;
            }
            blocks[other].text.clear();
        }
    }
    blocks.retain(|block| !block.text.is_empty());
}

fn starts_reference_entry(text: &str) -> bool {
    text.strip_prefix('[').and_then(|rest| rest.split_once(']'))
        .is_some_and(|(label, rest)| !rest.trim().is_empty() && !label.is_empty() && label.chars().all(|character| character.is_ascii_digit()))
}

fn numbered_reference(text: &str) -> bool {
    if let Some((label, _)) = text.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
        return (1..=4).contains(&label.len()) && label.bytes().all(|byte| byte.is_ascii_digit());
    }
    text.split_whitespace().next().and_then(|label| label.strip_suffix('.'))
        .is_some_and(|label| (1..=3).contains(&label.len()) && label.bytes().all(|byte| byte.is_ascii_digit()))
}

fn merge_reference_fragments(blocks: Vec<Block>, columns: &[PageColumns]) -> Vec<Block> {
    let column_key = |block: &Block| {
        (block.page, columns.get(block.page - 1).and_then(|layout| layout.lane(block.bounds)))
    };
    let mut edges = HashMap::new();
    for block in blocks.iter().filter(|block| block.kind == BlockKind::Reference) {
        let edge = edges.entry(column_key(block)).or_insert(block.bounds.left);
        *edge = edge.min(block.bounds.left);
    }

    let mut output: Vec<Block> = Vec::with_capacity(blocks.len());
    let mut numbered_section = None;
    for block in blocks {
        if block.kind != BlockKind::Reference {
            if block.kind == BlockKind::ReferenceHeading {
                numbered_section = None;
            }
            output.push(block);
            continue;
        }
        let numbered = numbered_reference(&block.text);
        let uses_numbers = *numbered_section.get_or_insert(numbered);
        let column = column_key(&block);
        let starts_entry = if uses_numbers { numbered } else { block.bounds.left <= edges[&column] + 2.8 };
        if !starts_entry {
            if let Some(last) = output.last_mut().filter(|last| last.kind == BlockKind::Reference) {
                append_text_fragment(&mut last.text, &block.text, None);
                if column_key(last) == column {
                    last.bounds = last.bounds.union(block.bounds);
                }
                continue;
            }
        }
        output.push(block);
    }
    output
}

#[derive(Clone, Copy)]
struct ParagraphLocation {
    page: usize,
    page_height: f32,
    column: Option<usize>,
    column_count: usize,
    bounds: Bounds,
    first_line_indent: f32,
}

fn merge_paragraph_fragments(
    blocks: Vec<Block>, pages: &[Page], columns: &[PageColumns], body: f32,
    hyphenation: &Hyphenation,
    continuations: &mut HashMap<usize, Vec<(usize, Bounds)>>,
) -> (Vec<Block>, HashMap<usize, String>) {
    let mut output: Vec<Block> = Vec::with_capacity(blocks.len());
    let mut native_inline = HashMap::new();
    let mut previous: Option<ParagraphLocation> = None;
    for block in blocks {
        let page = block.page.checked_sub(1).and_then(|index| pages.get(index))
            .filter(|_| block.kind == BlockKind::Paragraph);
        let inline = page.and_then(|page| inline_equation_text(&block, page));
        let location = page.map(|page| paragraph_location(&block, page, &columns[block.page - 1]));
        let continuation = output.last().zip(previous).zip(location).is_some_and(|((last, previous), next)| {
            paragraph_continues(last, &block, previous, next, body)
        });
        if continuation {
            let index = output.len() - 1;
            let last = &mut output[index];
            if let Some(source) = native_inline.get_mut(&index) {
                append_text_fragment(source, inline.as_deref().unwrap_or(&block.text), Some(hyphenation));
            } else if let Some(inline) = inline {
                let mut source = last.text.clone();
                append_text_fragment(&mut source, &inline, Some(hyphenation));
                native_inline.insert(index, source);
            }
            append_text_fragment(&mut last.text, &block.text, Some(hyphenation));
            // A block's source coordinates refer to the lane where the paragraph starts.
            let same_column = location.is_some_and(|location| {
                columns[block.page - 1].lane(last.bounds) == location.column
            });
            if last.page == block.page && same_column {
                last.bounds = last.bounds.union(block.bounds);
            } else {
                continuations.entry(index).or_default().push((block.page, block.bounds));
            }
        } else {
            if let Some(inline) = inline {
                native_inline.insert(output.len(), inline);
            }
            output.push(block);
        }
        previous = location;
    }
    (output, native_inline)
}

fn paragraph_location(block: &Block, page: &Page, columns: &PageColumns) -> ParagraphLocation {
    let bounds = block.bounds;
    let first_line_indent = page.groups.iter().flatten().find(|line| {
        (line.bounds.top - bounds.top).abs() < 0.5 && contains_bounds(bounds, line.bounds)
    }).map_or(0.0, |line| line.bounds.left - bounds.left);
    ParagraphLocation {
        page: block.page, page_height: page.height, column: columns.lane(bounds), column_count: columns.lane_count(),
        bounds, first_line_indent,
    }
}

fn paragraph_continues(
    left: &Block, right: &Block, previous: ParagraphLocation, next: ParagraphLocation, body: f32,
) -> bool {
    let end = left.text.trim_end_matches(['”', '’', '"', '\'', ')', ']']);
    if end.ends_with(['.', '!', '?', ':', '。', '！', '？', '：'])
        || !right.text.starts_with(char::is_lowercase) || starts_list_item(&right.text)
        || next.first_line_indent > body * 0.55
    {
        return false;
    }

    if previous.page == next.page {
        let gap = next.bounds.top - previous.bounds.bottom;
        let indent = next.bounds.left - previous.bounds.left;
        return match (previous.column, next.column) {
            // A narrow column can pick up the paragraph that started across the page above it.
            (None, Some(0)) => {
                (-1.0..=body * 1.15).contains(&gap) && indent.abs() <= body * 0.55
            }
            (left, right) if left == right => {
                (-1.0..=body * 1.15).contains(&gap) && (-body * 1.7..=body * 0.55).contains(&indent)
            }
            // In a mixed layout the top of the next column can be well below the page top.
            (Some(left), Some(right)) => right == left + 1 && gap <= 0.0,
            _ => false,
        };
    }

    next.page.checked_sub(previous.page) == Some(1)
        && previous.column.is_none_or(|column| column + 1 == previous.column_count)
        && next.column.is_none_or(|column| column == 0)
        && previous.bounds.bottom >= previous.page_height * 0.58 && next.bounds.top <= next.page_height * 0.34
}

fn equation_number(text: &str) -> bool {
    text.strip_prefix('(').and_then(|value| value.strip_suffix(')')).is_some_and(|value| {
        value.chars().any(|c| c.is_ascii_digit()) && value.len() <= 10 && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
    })
}

fn join_equation_fragments(blocks: &mut Vec<Block>) {
    const JOIN_OPERATORS: &str = "=≡≈∼≃∩∪∈≤≥→⇒⇝";
    for index in 0..blocks.len() {
        if blocks[index].kind != BlockKind::Equation || blocks[index].text.is_empty() {
            continue;
        }
        if blocks[index].text.ends_with('=') {
            join_piecewise_equation(blocks, index);
        }
        // Poppler can split an aligned equation at the wide space around a large operator.
        while blocks[index].text.ends_with(|ch| JOIN_OPERATORS.contains(ch) || ch == 'Ø') {
            let bounds = blocks[index].bounds;
            let next = blocks.iter().enumerate().filter(|(other, block)| {
                *other != index && !block.text.is_empty() && (block.bounds.top - bounds.top).abs() <= 5.0
                    && (-1.0..=25.0).contains(&(block.bounds.left - bounds.right))
                    && (block.kind == BlockKind::Equation || block.text.trim() == "Ø")
            }).min_by(|(_, a), (_, b)| a.bounds.left.total_cmp(&b.bounds.left)).map(|(other, _)| other);
            let Some(next) = next else { break; };
            let text = std::mem::take(&mut blocks[next].text);
            blocks[index].bounds = bounds.union(blocks[next].bounds);
            append_word(&mut blocks[index].text, &text);
        }

        if blocks[index].text.contains(['Ø', '∑', '∏', '∫', '⋂', '⋃']) {
            let bounds = blocks[index].bounds;
            let limit = blocks.iter().enumerate().filter(|(other, block)| {
                *other != index && !block.text.is_empty() && block.kind == BlockKind::Equation
                    && block.bounds.left > bounds.left + 5.0 && block.bounds.right < bounds.right - 5.0
                    && block.bounds.height() < bounds.height() * 0.85
                    && (-1.0..=7.0).contains(&(block.bounds.top - bounds.bottom))
            }).min_by(|(_, a), (_, b)| a.bounds.top.total_cmp(&b.bounds.top)).map(|(other, _)| other);
            if let Some(limit) = limit {
                let text = std::mem::take(&mut blocks[limit].text);
                blocks[index].bounds = bounds.union(blocks[limit].bounds);
                append_word(&mut blocks[index].text, &text);
            }
        }
    }
    blocks.retain(|block| !block.text.is_empty());
}

fn join_piecewise_equation(blocks: &mut [Block], equation: usize) {
    let bounds = blocks[equation].bounds;
    let branches = |below| blocks.iter().enumerate().filter(move |(index, block)| {
        let offset = block.bounds.top - bounds.top;
        *index != equation && block.kind == BlockKind::Equation && (2.0..=11.0).contains(&offset.abs())
            && (offset > 0.0) == below && (0.0..=15.0).contains(&(block.bounds.left - bounds.right))
            && block.text.split_whitespace().any(|word| word == "if" || word == "otherwise")
    }).min_by(|(_, a), (_, b)| a.bounds.left.total_cmp(&b.bounds.left)).map(|(index, _)| index);
    let (Some(upper), Some(lower)) = (branches(false), branches(true)) else { return; };
    if (blocks[upper].bounds.left - blocks[lower].bounds.left).abs() > 3.0 {
        return;
    }
    for branch in [upper, lower] {
        let text = std::mem::take(&mut blocks[branch].text);
        blocks[equation].bounds = blocks[equation].bounds.union(blocks[branch].bounds);
        append_word(&mut blocks[equation].text, &text);
    }
    let edge = blocks[upper].bounds.left;
    let equation_bounds = blocks[equation].bounds;
    for block in blocks.iter_mut().filter(|block| {
        block.kind == BlockKind::Paragraph && matches!(block.text.as_str(), "(" | ")" | "{" | "}")
            && (0.0..=8.0).contains(&(edge - block.bounds.right))
            && block.bounds.top > equation_bounds.top - 6.0 && block.bounds.bottom < equation_bounds.bottom + 6.0
    }) {
        block.text.clear();
    }
}

fn attach_equation_numbers(blocks: &mut Vec<Block>, page_width: f32) {
    let mut labels = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.kind == BlockKind::Paragraph && equation_number(&block.text) {
            let label = block.bounds;
            let target = blocks.iter().enumerate().filter(|(_, other)| {
                other.kind == BlockKind::Equation && (label.top - other.bounds.top).abs() < 8.0
                    && (0.0..page_width * 0.36).contains(&(label.left - other.bounds.right))
            }).min_by(|(_, left), (_, right)| right.bounds.right.total_cmp(&left.bounds.right));
            if let Some((target, _)) = target {
                labels.push((index, target));
            }
        }
    }
    for &(label, target) in &labels {
        let bounds = blocks[label].bounds;
        let text = std::mem::take(&mut blocks[label].text);
        blocks[target].bounds = blocks[target].bounds.union(bounds);
        append_word(&mut blocks[target].text, &text);
    }
    blocks.retain(|block| !block.text.is_empty());
}

fn join_display_assignments(blocks: &mut Vec<Block>, page: &Page) {
    let exact_line = |bounds| page.groups.iter().flatten().find(|line| line.bounds == bounds);
    let mut equations: Vec<_> = blocks.iter().enumerate().filter_map(|(index, block)| {
        (block.kind == BlockKind::Equation).then_some(index)
    }).collect();
    equations.sort_unstable_by(|&a, &b| blocks[a].bounds.left.total_cmp(&blocks[b].bounds.left));
    for index in equations {
        if blocks[index].text.is_empty() {
            continue;
        }
        let Some(mut line) = exact_line(blocks[index].bounds) else { continue; };
        while line.text.ends_with([',', ';']) && numbered_assignment(line).is_some() {
            let next = blocks.iter().enumerate().filter_map(|(candidate, block)| {
                if candidate == index || block.text.is_empty()
                    || !matches!(block.kind, BlockKind::Paragraph | BlockKind::Equation)
                    || !aligned_display_neighbors(line.bounds, block.bounds)
                {
                    return None;
                }
                let next = exact_line(block.bounds)?;
                display_assignment_continues(line, next).then_some((candidate, next))
            }).min_by(|(_, left), (_, right)| left.bounds.left.total_cmp(&right.bounds.left));
            let Some((candidate, next)) = next else { break; };
            let text = std::mem::take(&mut blocks[candidate].text);
            blocks[index].bounds = blocks[index].bounds.union(blocks[candidate].bounds);
            append_word(&mut blocks[index].text, &text);
            line = next;
        }
    }
    blocks.retain(|block| !block.text.is_empty());
}

fn aligned_display_neighbors(left: Bounds, right: Bounds) -> bool {
    (left.top - right.top).abs() <= 1.25 && (left.bottom - right.bottom).abs() <= 2.0
        && (0.0..=left.height() * 1.65).contains(&(right.left - left.right))
}

fn display_assignment_continues(left: &Line, right: &Line) -> bool {
    left.text.ends_with([',', ';']) && aligned_display_neighbors(left.bounds, right.bounds)
        && numbered_assignment(left).zip(numbered_assignment(right)).is_some_and(|((left, number, _), (right, next, _))| {
            left == right && number.parse::<u32>().ok().zip(next.parse::<u32>().ok())
                .is_some_and(|(number, next)| number + 1 == next)
        })
}

fn numbered_assignment(line: &Line) -> Option<(&str, &str, &[Word])> {
    let [base, script, equals, rest @ ..] = line.words.as_slice() else { return None; };
    let base_text = &line.text[base.text.clone()];
    let number = &line.text[script.text.clone()];
    if rest.is_empty() || base_text.chars().count() != 1 || !base_text.starts_with(char::is_alphabetic)
        || !number.bytes().all(|digit| digit.is_ascii_digit()) || number.len() > 4
        || &line.text[equals.text.clone()] != "="
        || isolated_script(base_text, base.bounds, number, script.bounds, script.bounds.left - base.bounds.right) != Some('_')
    {
        return None;
    }
    Some((base_text, number, rest))
}

fn monospaced_assignment(line: &Line) -> Option<String> {
    let (base, number, words) = numbered_assignment(line)?;
    let first = words.first()?;
    let last = words.last()?;
    let value = &line.text[first.text.start..last.text.end];
    let literal = value.trim_end_matches([',', '.', ';']);
    if !literal.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b" /-".contains(&byte)) {
        return None;
    }

    // Code text is set on an even grid. For a single token, a slash-separated
    // identifier and a wide, fixed-pitch-like cell provide the remaining cues.
    let cell = first.bounds.width() / (first.text.end - first.text.start) as f32;
    if !(0.67..=0.9).contains(&(cell / first.bounds.height())) {
        return None;
    }
    if words.len() == 1 {
        if !literal.contains('/') {
            return None;
        }
    } else if !words.windows(2).all(|pair| {
        let length = pair[0].text.end - pair[0].text.start;
        (pair[0].bounds.width() / length as f32 - cell).abs() <= cell * 0.08
            && (pair[1].bounds.left - pair[0].bounds.right - cell).abs() <= cell * 0.12
    }) {
        return None;
    }
    Some(format!("{base}_{{{number}}} = \\texttt{{{literal}}}{}", &value[literal.len()..]))
}

fn native_equation(block: &Block, page: &Page) -> Option<String> {
    let mut lines: Vec<_> = page.groups.iter().flatten().filter(|line| contains_bounds(block.bounds, line.bounds)).collect();
    lines.sort_unstable_by(|left, right| left.bounds.left.total_cmp(&right.bounds.left));
    let (first, rest) = lines.split_first()?;
    if !looks_like_equation(&first.text) {
        return None;
    }
    let mut text = native_equation_line(first)?;
    let mut previous = *first;
    for (index, next) in rest.iter().enumerate() {
        if display_assignment_continues(previous, next) {
            text.push_str("\\quad ");
            text.push_str(&native_equation_line(next)?);
            previous = next;
        } else if index + 1 == rest.len() && equation_number(&next.text) && next.bounds.left >= previous.bounds.right {
            append_word(&mut text, &next.text);
        } else {
            return None;
        }
    }
    Some(text)
}

fn native_equation_line(line: &Line) -> Option<String> {
    if line.text.contains(['Ø', 'ø', '�', '\\', '_', '^']) || line.text.chars().any(|ch| ch.is_control()
        || matches!(ch, '\u{e000}'..='\u{f8ff}' | '\u{239b}'..='\u{23b3}' | '∑' | '∏' | '∫' | '√' | '⋂' | '⋃'))
        || line.words.windows(2).any(|pair| pair[1].bounds.left < pair[0].bounds.left - 1.5)
    {
        return None;
    }
    if let Some(source) = monospaced_assignment(line) {
        return Some(source);
    }

    let mut text = String::new();
    let mut base: Option<&Word> = None;
    let mut script: Option<(char, Bounds)> = None;
    for word in &line.words {
        let token = &line.text[word.text.clone()];
        let previous = script.map(|(_, bounds)| bounds).or_else(|| base.map(|base| base.bounds));
        let gap = previous.map_or(0.0, |bounds| word.bounds.left - bounds.right);
        let marker = base.and_then(|base| {
            let symbol = &line.text[base.text.clone()];
            let base_gap = word.bounds.left - base.bounds.right;
            isolated_script(symbol, base.bounds, token, word.bounds, base_gap)
                .or_else(|| isolated_script(symbol, base.bounds, token, word.bounds, gap))
        });
        if let Some(marker) = marker {
            if script.is_some_and(|(previous, _)| marker == previous) {
                text.pop();
            } else {
                text.push(marker);
                text.push('{');
            }
            text.push_str(token);
            text.push('}');
            script = Some((marker, word.bounds));
        } else {
            if gap <= 1.25 || token.chars().all(|ch| "′″‴".contains(ch)) {
                text.push_str(token);
            } else {
                append_word(&mut text, token);
            }
            base = Some(word);
            script = None;
        }
    }
    Some(text)
}

fn isolated_script(base: &str, anchor: Bounds, token: &str, bounds: Bounds, gap: f32) -> Option<char> {
    if !base.ends_with(|ch: char| ch.is_alphanumeric() || ")]}".contains(ch))
        || !(-1.5..=3.25).contains(&gap) || token.chars().count() > 5
        || !token.chars().all(|ch| ch.is_alphanumeric() || "+−-".contains(ch))
    {
        return None;
    }
    let shift = (bounds.top + bounds.bottom - anchor.top - anchor.bottom) * 0.5;
    let small = bounds.height() <= anchor.height() * 0.90;
    let single_digit = token.len() == 1 && token.starts_with(|ch: char| ch.is_ascii_digit());
    if shift >= (anchor.height() * 0.15).max(1.1) && bounds.bottom > anchor.bottom - 1.5
        && (small || single_digit && bounds.top > anchor.top + 0.65)
    {
        Some('_')
    } else if shift <= -(anchor.height() * 0.13).max(1.0) && bounds.bottom < anchor.bottom - 1.1 && small {
        Some('^')
    } else {
        None
    }
}

fn inline_equation_text(block: &Block, page: &Page) -> Option<String> {
    let mut source = String::new();
    let mut search_from = 0;
    let mut copied_through = 0;
    for line in page.groups.iter().flatten().filter(|line| contains_bounds(block.bounds, line.bounds)) {
        // A discretionary hyphen may be absent from the reflowed block. The
        // unchanged line prefix still identifies where its word offsets start.
        let prefix = line.text.strip_suffix(LINE_HYPHENS).unwrap_or(&line.text);
        if prefix.is_empty() { continue; }
        let Some(offset) = block.text[search_from..].find(prefix) else { continue; };
        let line_start = search_from + offset;
        let mark = &line.text[prefix.len()..];
        let prefix_end = line_start + prefix.len();
        let line_length = if block.text[prefix_end..].starts_with(mark) { line.text.len() } else { prefix.len() };
        search_from = line_start + line_length;
        for (index, pair) in line.words.windows(2).enumerate() {
            let [base, script] = pair else { continue; };
            let before = index.checked_sub(1).and_then(|offset| line.words.get(offset));
            let after = line.words.get(index + 2);
            if !before.into_iter().chain(after).any(|word| {
                let operator = &line.text[word.text.clone()];
                operator.chars().count() <= 2 && operator.chars().any(|ch| "=<>≤≥≠≡≈∈∉∧∨→⇔+×".contains(ch))
            }) {
                continue;
            }
            let symbol = &line.text[base.text.clone()];
            let script_text = &line.text[script.text.clone()];
            let mut characters = symbol.chars();
            let Some(variable) = characters.next() else { continue; };
            if characters.next().is_some() || !variable.is_alphabetic() || is_cjk(variable)
                || script_text.chars().count() != 1 || !script_text.starts_with(char::is_alphanumeric)
            {
                continue;
            }
            let gap = script.bounds.left - base.bounds.right;
            if gap > 1.8 {
                continue;
            }
            let Some(marker) = isolated_script(symbol, base.bounds, script_text, script.bounds, gap) else { continue; };
            let start = line_start + base.text.start;
            if start < copied_through || script.text.end > line_length {
                continue;
            }
            source.push_str(&block.text[copied_through..start]);
            source.push_str("\\(");
            source.push_str(symbol);
            source.push(marker);
            source.push('{');
            source.push_str(script_text);
            source.push_str("}\\)");
            copied_through = line_start + script.text.end;
        }
    }
    if copied_through == 0 {
        return None;
    }
    source.push_str(&block.text[copied_through..]);
    Some(source)
}

struct PageColumns {
    width: f32,
    gutters: Vec<f32>,
}

impl PageColumns {
    fn lane_count(&self) -> usize {
        self.gutters.len() + 1
    }

    fn lane(&self, bounds: Bounds) -> Option<usize> {
        self.lane_with_margin(bounds, self.width * 0.025)
    }

    fn lane_with_margin(&self, bounds: Bounds, margin: f32) -> Option<usize> {
        if self.gutters.iter().any(|&gutter| bounds.left < gutter - margin && bounds.right > gutter + margin) {
            return None;
        }
        Some(self.gutters.partition_point(|&gutter| gutter <= bounds.center()))
    }
}

fn page_columns(page: &Page) -> PageColumns {
    let gutters = three_column_gutters(page).map(Vec::from)
        .unwrap_or_else(|| two_column_gutter(page).into_iter().collect());
    PageColumns { width: page.width, gutters }
}

fn two_column_gutter(page: &Page) -> Option<f32> {
    let midpoint = page.width * 0.5;
    let mut left = 0;
    let mut right = 0;
    for line in page.groups.iter().flatten() {
        let bounds = line.bounds;
        if bounds.width() < page.width * 0.14 || line.text.chars().count() < 24 {
            continue;
        }
        if bounds.left < midpoint - page.width * 0.16 && bounds.right < midpoint + page.width * 0.025 {
            left += 1;
        } else if bounds.left >= midpoint - page.width * 0.025 && bounds.left < midpoint + page.width * 0.13 {
            right += 1;
        }
    }
    (left >= 2 && right >= 2).then_some(midpoint)
}

fn three_column_gutters(page: &Page) -> Option<[f32; 2]> {
    let width = page.width;
    let mut lines: Vec<_> = page.groups.iter().flatten().filter_map(|line| {
        let bounds = line.bounds;
        ((width * 0.14..=width * 0.34).contains(&bounds.width())
            && line.text.chars().count() >= 24).then_some(bounds)
    }).collect();
    lines.sort_by(|left, right| left.left.total_cmp(&right.left));

    let mut alignments: Vec<Vec<Bounds>> = Vec::new();
    for line in lines {
        if let Some(aligned) = alignments.last_mut().filter(|aligned| line.left - aligned[0].left <= width * 0.012) {
            aligned.push(line);
        } else {
            alignments.push(vec![line]);
        }
    }
    alignments.retain(|lines| lines.len() >= 3);

    for (left_index, left) in alignments.iter().enumerate() {
        for (middle_index, middle) in alignments.iter().enumerate().skip(left_index + 1) {
            let stride = middle[0].left - left[0].left;
            if !(width * 0.20..=width * 0.34).contains(&stride) {
                continue;
            }
            let expected = middle[0].left + stride;
            for right in alignments.iter().skip(middle_index + 1) {
                if (right[0].left - expected).abs() > width * 0.04 {
                    continue;
                }
                if let Some(gutters) = parallel_text_gutters([left, middle, right], width) {
                    return Some(gutters);
                }
            }
        }
    }
    None
}

fn parallel_text_gutters(columns: [&[Bounds]; 3], page_width: f32) -> Option<[f32; 2]> {
    let top = columns.iter().filter_map(|col| col.iter().map(|line| line.top).min_by(f32::total_cmp))
        .max_by(f32::total_cmp)?;
    let bottom = columns.iter().filter_map(|col| col.iter().map(|line| line.bottom).max_by(f32::total_cmp))
        .min_by(f32::total_cmp)?;
    let right_edges = columns.map(|col| {
        let mut edges: Vec<_> = col.iter().filter(|line| line.bottom > top && line.top < bottom)
            .map(|line| line.right).collect();
        if edges.len() < 3 { return None; }
        edges.sort_unstable_by(f32::total_cmp);
        Some(edges[edges.len() * 3 / 4])
    });
    let [Some(first), Some(second), Some(_)] = right_edges else { return None; };
    let edges = [(first, columns[1][0].left), (second, columns[2][0].left)];
    if !edges.iter().all(|&(end, start)| (page_width * 0.008..=page_width * 0.09).contains(&(start - end))) {
        return None;
    }
    Some(edges.map(|(end, start)| (end + start) * 0.5))
}

fn order_page(page: &Page, mut blocks: Vec<Block>, columns: &PageColumns) -> Vec<Block> {
    blocks.sort_by(|left, right| left.bounds.top.total_cmp(&right.bounds.top)
        .then_with(|| left.bounds.left.total_cmp(&right.bounds.left)));
    if columns.lane_count() == 1 {
        return blocks;
    }
    let front_end = if blocks.iter().any(|block| block.kind == BlockKind::Title) {
        blocks.iter().filter(|block| block.bounds.top < page.height * 0.53).find(|block| {
            starts_abstract(&block.text) || section_name(&block.text) == "introduction"
        }).map_or(0.0, |block| block.bounds.top - 2.0)
    } else {
        0.0
    };
    let mut ordered = Vec::with_capacity(blocks.len());
    let mut lanes = (0..columns.lane_count()).map(|_| Vec::new()).collect::<Vec<_>>();
    let margin = page.width * if columns.lane_count() == 2 { 0.08 } else { 0.025 };
    for block in blocks {
        let bounds = block.bounds;
        let lane = columns.lane_with_margin(bounds, margin)
            .filter(|_| block.kind != BlockKind::Title && bounds.top >= front_end);
        if let Some(lane) = lane {
            lanes[lane].push(block);
        } else {
            for lane in &mut lanes {
                ordered.append(lane);
            }
            ordered.push(block);
        }
    }
    ordered.extend(lanes.into_iter().flatten());
    ordered
}

fn raster_layout(path: &Path) -> Result<HashMap<usize, Vec<Bounds>>, String> {
    let listed = run(command("pdfimages").arg("-list").arg(path))?;
    if !String::from_utf8_lossy(&listed).lines().skip(2).any(|line| !line.trim().is_empty()) {
        return Ok(HashMap::new());
    }
    let temporary = tempfile::tempdir().map_err(|error| format!("无法创建图片临时目录：{error}"))?;
    let output = run(command("pdftohtml").args(["-q", "-xml", "-stdout", "-zoom", "1", "-noroundcoord"])
        .arg(path).arg(temporary.path().join("image")))?;
    let mut result: HashMap<usize, Vec<Bounds>> = HashMap::new();
    let mut page = None;
    for line in String::from_utf8_lossy(&output).lines().map(str::trim_start) {
        if let Some(element) = line.strip_prefix("<page ") {
            page = xml_attr(element, "number").and_then(|number| number.parse().ok());
        } else if let (Some(page), Some(element)) = (page, line.strip_prefix("<image ")) {
            let attrs = ["left", "top", "width", "height"].map(|name| xml_attr(element, name).unwrap_or_default());
            if let Some(bounds) = positive_bounds(attrs) {
                result.entry(page).or_default().push(bounds);
            }
        }
    }
    Ok(result)
}

// Poppler emits the numeric attributes before the image path, one element per line.
fn xml_attr<'a>(element: &'a str, attribute: &str) -> Option<&'a str> {
    let marker = format!("{attribute}=\"");
    let token = element.split_ascii_whitespace().find_map(|token| token.strip_prefix(&marker))?;
    token.split_once('"').map(|(value, _)| value)
}

struct Asset {
    kind: BlockKind,
    bounds: Bounds,
    caption: Option<Bounds>,
}

fn add_assets(page: &Page, blocks: &mut Vec<Block>, columns: &PageColumns, images: Option<&Vec<Bounds>>) -> Vec<Asset> {
    join_detached_caption_titles(blocks);
    let mut rasters: Vec<Bounds> = Vec::new();
    for &bounds in images.into_iter().flatten() {
        if bounds.width() < 35.0 || bounds.height() < 22.0
            || bounds.width() * bounds.height() > page.width * page.height * 0.82
        {
            continue;
        }
        let duplicate = rasters.iter().any(|other| (other.left - bounds.left).abs() < 1.0
            && (other.top - bounds.top).abs() < 1.0 && (other.right - bounds.right).abs() < 1.0
            && (other.bottom - bounds.bottom).abs() < 1.0);
        if !duplicate {
            rasters.push(bounds);
        }
    }

    let mut captions: Vec<_> = blocks.iter().filter(|block| block.kind == BlockKind::Caption).collect();
    captions.sort_by(|left, right| left.bounds.top.total_cmp(&right.bounds.top));
    let mut assets = Vec::<Asset>::new();
    let mut used = vec![false; rasters.len()];
    for caption in captions {
        let Some(is_figure) = caption_type(&caption.text) else { continue; };
        let mark = caption.bounds;
        let region = if is_figure {
            vector_figure(page, blocks, mark, columns)
        } else {
            table_region(page, mark, columns, &assets)
        };
        let mut raster_bounds: Option<Bounds> = None;
        for (index, &raster) in rasters.iter().enumerate() {
            if used[index] { continue; }
            let near_above = mark.overlap_x(raster) > 20.0 && (-3.0..65.0).contains(&(mark.top - raster.bottom));
            let near_below = !is_figure && mark.overlap_x(raster) > 20.0 && (-3.0..40.0).contains(&(raster.top - mark.bottom));
            if region.is_some_and(|region| contains_bounds(region, raster)) || region.is_none() && (near_above || near_below)
                || is_figure && near_above
            {
                raster_bounds = Some(raster_bounds.map_or(raster, |current| current.union(raster)));
                used[index] = true;
            }
        }
        let bounds = match (region, raster_bounds) {
            // A wide raster directly above the caption describes the figure more precisely
            // than the space inferred from surrounding text.
            (Some(region), Some(raster)) if is_figure && raster.width() >= region.width() * 0.82
                && (-3.0..=24.0).contains(&(mark.top - raster.bottom)) => raster,
            (Some(region), Some(raster)) => region.union(raster),
            (Some(bounds), None) | (None, Some(bounds)) => bounds,
            (None, None) => continue,
        };
        assets.push(Asset {
            kind: if is_figure { BlockKind::Figure } else { BlockKind::Table },
            bounds, caption: Some(mark),
        });
    }
    assets.extend(rasters.into_iter().zip(used).filter_map(|(bounds, used)| {
        (!used).then_some(Asset { kind: BlockKind::Figure, bounds, caption: None })
    }));

    blocks.retain(|block| {
        matches!(block.kind, BlockKind::Title | BlockKind::Author | BlockKind::Caption)
            || !assets.iter().any(|asset| contains_bounds(asset.bounds, block.bounds))
    });
    blocks.extend(assets.iter().map(|asset| Block {
        kind: asset.kind, page: page.number, text: String::new(), bounds: asset.bounds,
    }));
    assets
}

fn join_detached_caption_titles(blocks: &mut Vec<Block>) {
    for index in 0..blocks.len() {
        let block = &blocks[index];
        if block.kind != BlockKind::Caption
            || !block.text.lines().next().and_then(caption_label).is_some_and(|(_, tail)| tail.is_empty())
        {
            continue;
        }
        let label = block.bounds;
        loop {
            let last = blocks[index].bounds.bottom;
            let title = blocks.iter().enumerate().filter(|(_, candidate)| {
                let bounds = candidate.bounds;
                matches!(candidate.kind, BlockKind::Paragraph | BlockKind::Heading)
                    && (-0.5..=6.0).contains(&(bounds.top - last))
                    && (bounds.center() - label.center()).abs() <= 7.0
                    && bounds.bottom - label.top <= 40.0
                    && candidate.text.chars().filter(|c| c.is_uppercase()).count() >= 4
                    && !candidate.text.chars().any(char::is_lowercase)
            }).min_by(|(_, a), (_, b)| a.bounds.top.total_cmp(&b.bounds.top));
            let Some((other, title)) = title else { break; };
            let title_bounds = title.bounds;
            let title = std::mem::take(&mut blocks[other].text);
            blocks[index].text.push('\n');
            blocks[index].text.push_str(&title);
            blocks[index].bounds = blocks[index].bounds.union(title_bounds);
        }
    }
    blocks.retain(|block| !block.text.is_empty());
}

fn contains_bounds(region: Bounds, content: Bounds) -> bool {
    content.top >= region.top - 2.0 && content.bottom <= region.bottom + 3.0
        && region.overlap_x(content) >= content.width() * 0.96
}

fn link_asset_captions(blocks: &[Block], assets: &HashMap<usize, Vec<Asset>>) -> HashMap<usize, usize> {
    let mut positioned = HashMap::<usize, Vec<(usize, &Block)>>::new();
    for (index, block) in blocks.iter().enumerate().filter(|(_, block)| {
        matches!(block.kind, BlockKind::Figure | BlockKind::Table | BlockKind::Caption)
    }) {
        positioned.entry(block.page).or_default().push((index, block));
    }
    let mut links = HashMap::new();
    for (page, assets) in assets {
        let Some(blocks) = positioned.get(page) else { continue; };
        for asset in assets {
            let Some(caption) = asset.caption else { continue; };
            let index = blocks.iter().find(|(_, block)| block.kind == asset.kind && block.bounds == asset.bounds);
            let caption_index = blocks.iter().find(|(_, block)| block.kind == BlockKind::Caption && block.bounds == caption);
            if let (Some((index, _)), Some((caption_index, _))) = (index, caption_index) {
                links.insert(*index, *caption_index);
            }
        }
    }
    links
}

fn asset_columns(page: &Page, caption: Bounds, columns: &PageColumns) -> (f32, f32) {
    let margin = page.width * 0.065;
    let Some(lane) = columns.lane_with_margin(caption, 8.0) else { return (margin, page.width - margin); };
    let left = lane.checked_sub(1).and_then(|index| columns.gutters.get(index)).map_or(margin, |gutter| gutter + 4.0);
    let right = columns.gutters.get(lane).map_or(page.width - margin, |gutter| gutter - 4.0);
    (left, right)
}

fn vector_figure(page: &Page, blocks: &[Block], caption: Bounds, columns: &PageColumns) -> Option<Bounds> {
    let (left, right) = asset_columns(page, caption, columns);
    let bottom = caption.top - 2.0;
    let column = Bounds { left, top: 0.0, right, bottom };
    let text_width = if columns.lane_with_margin(caption, 8.0).is_none() {
        column.width() / columns.lane_count() as f32
    } else {
        column.width()
    };
    let above = blocks.iter().filter(|block| {
        let bounds = block.bounds;
        let overlap = bounds.overlap_x(column);
        bounds.bottom <= bottom + 2.0 && match block.kind {
            BlockKind::Caption | BlockKind::Heading | BlockKind::Title | BlockKind::Author => {
                overlap > column.width() * 0.25
            }
            BlockKind::Paragraph => overlap > text_width * 0.72 && block.text.chars().count() > 58
                && block.text.split_whitespace().count() > 9 && prose_word_ratio(&block.text) > 0.58,
            _ => false,
        }
    }).map(|block| block.bounds.bottom).max_by(f32::total_cmp);
    let page_origin = (page.height * 0.06).max(bottom - page.height * 0.61);
    let top = above.map_or(page_origin, |y| y + 6.0).max(page_origin);
    let top = if bottom - top < 28.0 && bottom < page.height * 0.30 { page_origin } else { top };
    (bottom - top >= 28.0).then_some(Bounds { top, ..column })
}

struct TableRow {
    bounds: Bounds,
    cells: Vec<f32>,
    boundary: bool,
}

fn table_region(page: &Page, caption: Bounds, columns: &PageColumns, assets: &[Asset]) -> Option<Bounds> {
    let (left, right) = asset_columns(page, caption, columns);
    let column = Bounds { left, top: 0.0, right, bottom: page.height };
    let search_height = page.height * 0.48;
    let mut lines: Vec<_> = page.groups.iter().flatten().filter(|line| {
        let bounds = line.bounds;
        (bounds.bottom <= caption.top + 0.5 || bounds.top >= caption.bottom - 0.5)
            && bounds.top >= (caption.top - search_height).max(0.0) && bounds.bottom <= (caption.bottom + search_height).min(page.height)
            && bounds.overlap_x(column) >= bounds.width() * 0.88
    }).collect();
    lines.sort_by(|a, b| a.bounds.top.total_cmp(&b.bounds.top).then_with(|| a.bounds.left.total_cmp(&b.bounds.left)));
    let mut rows: Vec<TableRow> = Vec::new();
    for line in lines {
        let bounds = line.bounds;
        // A full-width caption can sit beside prose columns whose left edges repeat like a grid.
        let prose_column = columns.lane(bounds).is_some_and(|lane| {
            columns.lane_count() > 1 && column.width() > page.width * 0.7
                && bounds.width() > page.width * if columns.lane_count() == 2 { 0.26 } else { 0.17 }
                && if lane == 0 { bounds.left < page.width * 0.17 }
                    else { (bounds.left - columns.gutters[lane - 1]).abs() < page.width * 0.045 }
        }) && line.text.split_whitespace().count() >= 6 && prose_word_ratio(&line.text) > 0.60;
        let prose = line.cell_breaks.is_empty() && (prose_column || bounds.width() > column.width() * 0.65
            && line.text.split_whitespace().count() >= 12);
        let boundary = caption_type(&line.text).is_some() || known_heading(&line.text) || prose;
        if let Some(last) = rows.last_mut().filter(|row| (row.bounds.top - bounds.top).abs() <= 3.2) {
            last.bounds = last.bounds.union(bounds);
            last.cells.push(bounds.left);
            last.cells.extend(line.cell_breaks.iter().copied());
            last.boundary |= boundary;
        } else {
            let cells = std::iter::once(bounds.left).chain(line.cell_breaks.iter().copied()).collect();
            rows.push(TableRow { bounds, cells, boundary });
        }
    }
    for row in &mut rows {
        row.cells.sort_by(f32::total_cmp);
        row.cells.dedup_by(|a, b| (*a - *b).abs() < 12.0);
    }
    let start_after = rows.partition_point(|row| row.bounds.top < caption.top);
    let above = table_grid(rows[..start_after].iter().rev(), caption.top, false);
    let below = table_grid(rows[start_after..].iter(), caption.bottom, true);
    [above, below].into_iter().flatten().filter(|(bounds, _)| {
        assets.iter().all(|asset| asset.kind != BlockKind::Table
            || !contains_bounds(asset.bounds, *bounds) && !contains_bounds(*bounds, asset.bounds))
    }).max_by(|left, right| left.1.total_cmp(&right.1)).map(|(bounds, _)| bounds)
}

fn table_grid<'a>(rows: impl Iterator<Item = &'a TableRow>, edge: f32, below: bool) -> Option<(Bounds, f32)> {
    let mut selected: Vec<&TableRow> = Vec::new();
    let mut near_tiers = Vec::new();
    let mut tiers = Vec::new();
    let mut consecutive_tiers = 0;
    let mut previous = edge;
    for row in rows {
        let near = if below { row.bounds.top } else { row.bounds.bottom };
        let gap = if below { near - previous } else { previous - near };
        if row.boundary || gap > if selected.is_empty() { 36.0 } else { 18.0 } {
            break;
        }
        if row.cells.len() < 2 {
            if !selected.is_empty() {
                if gap > 7.0 || consecutive_tiers == 2 { break; }
                tiers.push(row);
                consecutive_tiers += 1;
                previous = if below { row.bounds.bottom } else { row.bounds.top };
                continue;
            }
            if gap > if near_tiers.is_empty() { 20.0 } else { 7.0 } { break; }
            near_tiers.push(row);
            previous = if below { row.bounds.bottom } else { row.bounds.top };
            continue;
        }
        consecutive_tiers = 0;
        if selected.is_empty() && gap > 7.0 {
            near_tiers.clear();
        }
        previous = if below { row.bounds.bottom } else { row.bounds.top };
        selected.push(row);
    }

    let first = selected.first()?;
    let last = selected.last()?;
    if selected.len() < 2 || !selected.iter().enumerate().any(|(index, row)| {
        selected.iter().skip(index + 1).take(3).any(|next| {
            row.cells.iter().filter(|x| next.cells.iter().any(|y| (*x - y).abs() <= 5.0)).count() >= 2
        })
    }) {
        return None;
    }
    let mut bounds = selected.iter().fold(first.bounds.union(last.bounds), |bounds, row| bounds.union(row.bounds));
    // Adjacent single-cell rows can be merged column headers or table notes.
    for tier in near_tiers.into_iter().chain(tiers) {
        if bounds.overlap_x(tier.bounds) >= tier.bounds.width() * 0.94 {
            bounds = bounds.union(tier.bounds);
        }
    }
    let first_gap = if below { bounds.top - edge } else { edge - bounds.bottom }.max(0.0);
    let top = if below { (bounds.top - 3.5).max(edge + 1.0) } else { (bounds.top - 3.5).max(0.0) };
    let bottom = if below { bounds.bottom + 4.5 } else { (bounds.bottom + 4.5).min(edge - 1.0) };
    let bounds = Bounds { left: (bounds.left - 5.5).max(0.0), right: bounds.right + 5.5, top, bottom };
    Some((bounds, selected.len().min(4) as f32 - first_gap / 6.0))
}

fn prose_word_ratio(text: &str) -> f32 {
    let (words, prose) = text.split_whitespace().fold((0, 0), |(words, prose), token| {
        let token = token.trim_matches(['.', ',', ':', ';', '(', ')', '“', '”', '-']);
        let ordinary = token.chars().count() >= 3 && token.chars().all(char::is_alphabetic);
        (words + 1, prose + usize::from(ordinary))
    });
    prose as f32 / words.max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_a_detached_drop_cap_without_moving_body_lines() {
        let layout = concat!(
            "header\n",
            "1\t1\t0\t0\t0\t0\t0\t0\t684\t864\t-1\t###PAGE###\n",
            "3\t1\t0\t0\t0\t0\t78\t504\t38\t95\t-1\t###FLOW###\n",
            "4\t1\t0\t0\t0\t0\t78\t504\t38\t95\t-1\t###LINE###\n",
            "5\t1\t0\t0\t0\t0\t78\t504\t38\t95\t100\tT\n",
            "3\t1\t0\t0\t0\t0\t120\t522\t220\t31\t-1\t###FLOW###\n",
            "4\t1\t0\t0\t0\t0\t120\t522\t220\t16\t-1\t###LINE###\n",
            "5\t1\t0\t0\t0\t0\t120\t522\t11\t16\t100\the\n",
            "5\t1\t0\t0\t0\t1\t134\t522\t45\t16\t100\temergent\n",
            "5\t1\t0\t0\t0\t2\t183\t522\t49\t16\t100\tnetworks\n",
            "4\t1\t0\t0\t1\t0\t120\t537\t154\t16\t-1\t###LINE###\n",
            "5\t1\t0\t0\t1\t0\t120\t537\t154\t16\t100\tcontinue across several regions.\n",
            "3\t1\t1\t0\t0\t0\t98\t522\t17\t16\t-1\t###FLOW###\n",
            "4\t1\t1\t0\t0\t0\t98\t522\t17\t16\t-1\t###LINE###\n",
            "5\t1\t1\t0\t0\t0\t98\t522\t17\t16\t100\tA\n",
        );
        let pages = parse_layout(layout.as_bytes()).unwrap();
        let groups = &pages[0].groups;
        assert_eq!(groups.len(), 2);
        let first = &groups[0][0];
        assert_eq!(first.text, "The emergent networks");
        assert_eq!(first.bounds, Bounds::from_size(120.0, 522.0, 220.0, 16.0));
        assert_eq!(first.font_height(), 16.0);
        assert_eq!(&first.text[first.words[0].text.clone()], "The");
        assert_eq!(&first.text[first.words[1].text.clone()], "emergent");
        assert_eq!(groups[1][0].text, "A");
    }

    #[test]
    fn reconstructs_the_aligned_numbered_assignments_as_one_display() {
        let line = |left, number, value: &str| {
            let text = format!("C {number} = {value}");
            let mut words = vec![
                Word { bounds: Bounds::from_size(left, 600.0, 7.0, 8.9), text: 0..1 },
                Word { bounds: Bounds::from_size(left + 7.0, 603.5, 4.0, 6.2), text: 2..3 },
                Word { bounds: Bounds::from_size(left + 14.0, 600.0, 8.0, 8.9), text: 4..5 },
            ];
            let mut x = left + 25.0;
            let mut start = 6;
            for word in value.split_whitespace() {
                let end = start + word.len();
                let width = word.len() as f32 * 5.9;
                words.push(Word { bounds: Bounds::from_size(x, 600.7, width, 7.7), text: start..end });
                x += width + 5.9;
                start = end + 1;
            }
            Line {
                bounds: Bounds { left, top: 600.0, right: x - 5.9, bottom: 609.7 },
                text, last_word_right: None, font_heights: Vec::new(), cell_breaks: Vec::new(), words,
            }
        };
        let first = line(140.0, 1, "show/inspect,");
        let left = first.bounds.right + 11.0;
        let next = line(left, 2, "compare the selected entries,");
        let unrelated = line(next.bounds.right + 11.0, 4, "other text.");
        let block = |line: &Line, kind| Block { kind, page: 1, text: line.text.clone(), bounds: line.bounds };
        let mut blocks = vec![block(&next, BlockKind::Paragraph), block(&unrelated, BlockKind::Paragraph),
            block(&first, BlockKind::Equation)];
        let page = Page { number: 1, width: 612.0, height: 792.0, groups: vec![vec![first], vec![next], vec![unrelated]] };

        join_display_assignments(&mut blocks, &page);
        assert_eq!(blocks.len(), 2);
        let combined = blocks.iter().find(|block| block.kind == BlockKind::Equation).unwrap();
        assert_eq!(native_equation(combined, &page).as_deref(),
            Some(r"C_{1} = \texttt{show/inspect},\quad C_{2} = \texttt{compare the selected entries},"));
        assert!(blocks.iter().any(|block| block.text == "C 4 = other text."));
    }

    #[test]
    fn groups_adjacent_list_items_around_a_separate_introduction() {
        let items = ["• Check the given scope.", "• Check the given payload?", "A second group asks:",
            "• What is the next request?", "• What is the next bound?"];
        let page = Page { number: 1, width: 612.0, height: 792.0, groups: vec![items.into_iter().enumerate()
            .map(|(index, text)| Line {
                bounds: Bounds::from_size(74.0, 550.0 + index as f32 * 12.0, 200.0, 9.0),
                text: text.to_owned(), last_word_right: None, font_heights: Vec::new(), cell_breaks: Vec::new(), words: Vec::new(),
            }).collect()] };
        let hyphenation = Hyphenation::from_lines(std::iter::empty());
        let blocks = text_blocks(&page, 9.0, &HashSet::new(), None, &hyphenation);
        assert_eq!(blocks.iter().map(|block| block.text.as_str()).collect::<Vec<_>>(), items);
        assert_eq!(collect_list_groups(&blocks, 9.0), HashMap::from([(0, 2), (3, 5)]));
    }

    #[test]
    fn keeps_inline_scripts_when_rejoining_a_wrapped_word() {
        let make_line = |top, text: &str, words| Line {
            bounds: Bounds::from_size(15.0, top, 150.0, 10.0),
            text: text.to_owned(), last_word_right: None, font_heights: Vec::new(), cell_breaks: Vec::new(), words,
        };
        let words = vec![
            Word { bounds: Bounds::from_size(25.0, 8.0, 4.0, 8.0), text: 4..5 },
            Word { bounds: Bounds::from_size(29.0, 12.0, 2.0, 5.0), text: 6..7 },
            Word { bounds: Bounds::from_size(35.0, 8.0, 4.0, 8.0), text: 8..9 },
        ];
        let page = Page { number: 1, width: 500.0, height: 800.0, groups: vec![vec![
            make_line(8.0, "Let x i = combina-", words),
            make_line(22.0, "tion of values.", Vec::new()),
        ]] };
        let hyphenation = Hyphenation::from_lines(std::iter::empty());
        let block = make_block(1, BlockKind::Paragraph, &page.groups[0], &hyphenation);
        assert_eq!(block.text, "Let x i = combination of values.");
        assert_eq!(inline_equation_text(&block, &page).as_deref(), Some(r"Let \(x_{i}\) = combination of values."));
    }

    #[test]
    fn preserves_unfinished_sentences_at_column_and_page_breaks() {
        let location = |page, column, left, top| ParagraphLocation {
            page, page_height: 840.0, column: Some(column), column_count: 2,
            bounds: Bounds::from_size(left, top, 251.0, 9.0), first_line_indent: 0.0,
        };
        let block = |position: ParagraphLocation, text: &str| Block {
            kind: BlockKind::Paragraph, page: position.page, text: text.into(), bounds: position.bounds,
        };
        let left = location(2, 0, 38.0, 754.0);
        let right = location(2, 1, 307.0, 84.0);
        let next_page = location(3, 0, 38.0, 84.0);
        let start = block(left, "However, sandboxing alone cannot solve");
        let continued = block(right, "the problem described above.");
        assert!(paragraph_continues(&start, &continued, left, right, 9.0));
        assert!(!paragraph_continues(&block(left, "The sandbox is limited."), &continued, left, right, 9.0));
        assert!(!paragraph_continues(&start, &block(right, "Fourth, consider the next example."), left, right, 9.0));
        assert!(!paragraph_continues(&start, &continued, left, ParagraphLocation { first_line_indent: 9.0, ..right }, 9.0));
        let right_bottom = location(2, 1, 307.0, 754.0);
        assert!(paragraph_continues(&block(right_bottom, "The argument continues"),
            &block(next_page, "on the next page."), right_bottom, next_page, 9.0));
    }

    #[test]
    fn connects_a_spanning_paragraph_to_a_lower_three_column_section() {
        let location = |column, left, top, width, height| ParagraphLocation {
            page: 1, page_height: 864.0, column, column_count: 3,
            bounds: Bounds::from_size(left, top, width, height), first_line_indent: 0.0,
        };
        let block = |position: ParagraphLocation, text: &str| Block {
            kind: BlockKind::Paragraph, page: position.page, text: text.into(), bounds: position.bounds,
        };
        let full = location(None, 78.0, 522.0, 514.0, 151.0);
        let left = location(Some(0), 78.0, 679.0, 161.0, 97.0);
        let middle = location(Some(1), 259.0, 679.0, 159.0, 97.0);
        let right = location(Some(2), 440.0, 679.0, 166.0, 97.0);
        let start = block(full, "The sample text continues across");
        let first = block(left, "the first column, then wraps");
        let second = block(middle, "to the second column.");
        assert!(paragraph_continues(&start, &first, full, left, 12.5));
        assert!(paragraph_continues(&first, &second, left, middle, 12.5));
        assert!(!paragraph_continues(&start, &second, full, middle, 12.5));
        assert!(!paragraph_continues(&first, &block(right, "into the last column."), left, right, 12.5));
        assert!(!paragraph_continues(&second, &block(right, "A separate paragraph starts here."), middle, right, 12.5));
    }
}
