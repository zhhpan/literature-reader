use crate::math::{span_runs, split_inline};
use crate::pdf::{Block, BlockKind, Document};
use crate::reader::{ImageKey, Raster, Reader, is_translatable};
use math::{NativeMath, NativeText};
use std::collections::HashSet;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Controls::{SetScrollInfo, WM_MOUSELEAVE};
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

mod math;
mod platform;
mod position;
mod selection;
mod updates;
use position::PositionState;
use selection::{Selection, TextPart};
use updates::UpdateControl;
pub use platform::run;

const CANVAS: u32 = color(244, 247, 246);
const WHITE: u32 = color(255, 255, 255);
const TEXT: u32 = color(24, 39, 36);
const MUTED: u32 = color(83, 103, 98);
const LINE: u32 = color(217, 228, 224);
const ACCENT: u32 = color(23, 104, 86);
const ACCENT_DARK: u32 = color(13, 83, 68);
const TINT: u32 = color(233, 246, 241);
const ERROR: u32 = color(155, 52, 40);

const BODY: usize = 0;
const SMALL: usize = 1;
const MEDIUM: usize = 2;
const TITLE: usize = 3;
const BRAND: usize = 4;
const PAPER_TITLE: usize = 5;
const PAPER_TITLE_ZH: usize = 6;
const PAPER_HEADING: usize = 7;
const PAPER_AUTHOR: usize = 8;
const PAPER_BODY: usize = 9;
const PAPER_BODY_ZH: usize = 10;
const PAPER_SMALL: usize = 11;
const PAPER_SMALL_ZH: usize = 12;
const PAPER_LINK: usize = 13;
const UI_FONT_COUNT: usize = 14;
const FONT_COUNT: usize = UI_FONT_COUNT + math::FONT_COUNT;
const PAPER_INSET: i32 = 33;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Login,
    CopyLoginLink,
    Open,
    OpenRecent(usize),
    RecentMenu,
    Models,
    Account,
    PageMenu,
    GoTo(usize),
    Previous,
    Next,
    Bilingual,
    Pdf,
    TranslateAll,
    Stop,
    Details,
    Update,
    JumpCitation(usize),
    OpenSource { block: usize, link: usize },
}

struct Hit {
    rect: RECT,
    action: Action,
}

struct CopyTarget {
    rect: RECT,
    index: usize,
    translation: bool,
}

#[derive(Clone, Copy)]
enum RowKind {
    Text,
    Media { caption: Option<usize> },
    ListSource { first: bool },
    ListTranslation { first: bool, last: bool },
    ReferenceHeading,
    ReferenceSource,
    ReferenceTranslationHeading,
    ReferenceTranslation,
}

#[derive(Clone, Copy)]
struct RowContent {
    block: usize,
    kind: RowKind,
}

struct PaperLink {
    label: String,
    action: Action,
    x: i32,
    y: i32,
    width: i32,
}

struct Row {
    content: RowContent,
    y: i32,
    height: i32,
    original_height: i32,
    translated_height: i32,
    image_height: i32,
    original_layout: Option<NativeText>,
    translated_layout: Option<NativeText>,
    equation: Option<NativeMath>,
    links: Vec<PaperLink>,
    links_height: i32,
    top: i32,
    gap: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct LayoutKey {
    width: i32,
    dpi: u32,
    generation: usize,
}

struct View {
    reader: Reader,
    updates: UpdateControl,
    pending_open: Option<PathBuf>,
    login_link: Option<String>,
    pdf_view: bool,
    dpi: u32,
    fonts: [HFONT; FONT_COUNT],
    scroll: i32,
    bilingual_scroll: i32,
    original_scroll: i32,
    max_scroll: i32,
    body_height: i32,
    viewport_height: i32,
    hover: Option<Action>,
    hits: Vec<Hit>,
    copy_targets: Vec<CopyTarget>,
    layout_key: Option<LayoutKey>,
    translation_layout_reset: u64,
    rows: Vec<Row>,
    source_rows: Vec<Option<usize>>,
    translation_rows: Vec<Option<usize>>,
    bibliography: Option<(usize, usize)>,
    focused_reference: Option<usize>,
    reading_position: PositionState,
    selection: Selection,
    mouse_press: Option<platform::MousePress>,
}

impl View {
    fn new(dpi: u32) -> Self {
        Self {
            reader: Reader::new(),
            updates: UpdateControl::new(),
            pending_open: None,
            login_link: None,
            pdf_view: false,
            dpi,
            fonts: make_fonts(dpi),
            scroll: 0,
            bilingual_scroll: 0,
            original_scroll: 0,
            max_scroll: 0,
            body_height: 0,
            viewport_height: 0,
            hover: None,
            hits: Vec::new(),
            copy_targets: Vec::new(),
            layout_key: None,
            translation_layout_reset: 0,
            rows: Vec::new(),
            source_rows: Vec::new(),
            translation_rows: Vec::new(),
            bibliography: None,
            focused_reference: None,
            reading_position: PositionState::default(),
            selection: Selection::default(),
            mouse_press: None,
        }
    }

    fn open(&mut self, path: PathBuf) {
        if !self.reader.signed_in {
            self.pending_open = Some(path);
            return;
        }
        self.pending_open = None;
        self.flush_reading_position();
        self.reading_position = PositionState::default();
        self.selection = Selection::default();
        self.mouse_press = None;
        self.scroll = 0;
        self.bilingual_scroll = 0;
        self.original_scroll = 0;
        self.pdf_view = false;
        self.max_scroll = 0;
        self.layout_key = None;
        self.rows.clear();
        self.source_rows.clear();
        self.translation_rows.clear();
        self.bibliography = None;
        self.hits.clear();
        self.copy_targets.clear();
        self.focused_reference = None;
        self.reader.open(path);
    }

    fn open_pending_document(&mut self) -> bool {
        if self.reader.signed_in && let Some(path) = self.pending_open.take() {
            self.open(path);
            return true;
        }
        false
    }

    fn start_login(&mut self) {
        self.login_link = None;
        self.reader.start_login();
    }

    fn cancel_login(&mut self) {
        if self.login_link.take().is_some() {
            self.reader.models_error = None;
        }
        self.reader.cancel_login();
    }

    fn goto(&mut self, page: usize) {
        if let Some(document) = &self.reader.document {
            let page = page.clamp(1, document.pages.max(1));
            if page != self.reader.page {
                self.reader.page = page;
                self.scroll = 0;
                self.mark_reading_position_changed();
            }
        }
    }

    fn switch_pdf_view(&mut self, pdf_view: bool) {
        if self.pdf_view == pdf_view {
            return;
        }
        if pdf_view {
            self.bilingual_scroll = self.scroll;
            if let Some(block) = self.rows.get(self.first_visible_row())
                .and_then(|row| self.reader.document.as_ref()?.blocks.get(row.content.block))
            {
                if block.page != self.reader.page {
                    self.reader.page = block.page;
                    self.original_scroll = 0;
                }
            }
            self.scroll = self.original_scroll;
        } else {
            self.original_scroll = self.scroll;
            self.scroll = self.bilingual_scroll;
        }
        self.pdf_view = pdf_view;
        self.clear_text_selection();
        self.mark_reading_position_changed();
    }

    fn jump_to_block(&mut self, block: usize) {
        if let Some(row) = self.source_rows.get(block).and_then(|index| *index)
            .and_then(|index| self.rows.get(index))
        {
            let position = row.y - self.s(18);
            self.focused_reference = Some(block);
            self.scroll(position);
        }
    }

    fn change_dpi(&mut self, dpi: u32) {
        if self.pdf_view {
            self.scroll = rescale(self.scroll, self.dpi, dpi);
        }
        self.original_scroll = rescale(self.original_scroll, self.dpi, dpi);
        self.dpi = dpi;
        let old = std::mem::replace(&mut self.fonts, make_fonts(dpi));
        for font in old {
            unsafe { DeleteObject(font) };
        }
    }

    fn s(&self, px: i32) -> i32 {
        scale(px, self.dpi)
    }

    fn hit(&self, point: POINT) -> Option<Action> {
        self.hits.iter().find(|hit| contains(hit.rect, point) && self.can_activate(hit.action))
            .map(|hit| hit.action)
    }

    fn can_activate(&self, action: Action) -> bool {
        self.reader.signed_in || matches!(action,
            Action::Login | Action::CopyLoginLink | Action::Update | Action::Details)
    }

    fn scroll(&mut self, value: i32) {
        let position = value.clamp(0, self.max_scroll);
        if position != self.scroll {
            self.scroll = position;
            self.mark_reading_position_changed();
        }
    }

    fn status(&self) -> (String, u32) {
        if self.login_link.is_some() && let Some(error) = &self.reader.models_error {
            return (error.clone(), ERROR);
        }
        if !self.reader.signed_in {
            if self.reader.signing_in() {
                return ("请在登录窗口完成 CodeBuddy 授权…".into(), ACCENT);
            }
            if self.reader.checking_models {
                return ("正在恢复 CodeBuddy 登录…".into(), MUTED);
            }
            if let Some(error) = &self.reader.models_error {
                return (error.clone(), ERROR);
            }
            let status = if self.pending_open.is_some() { "登录后打开已选 PDF" } else { "" };
            return (status.into(), MUTED);
        }
        if let Some(error) = &self.reader.load_error {
            return (error.clone(), ERROR);
        }
        if self.reader.loading {
            return ("正在提取 PDF 版面和文字…".into(), ACCENT);
        }
        if let Some(job) = &self.reader.translation {
            let state = if job.stopping() { "正在停止" } else { "正在翻译" };
            return (format!("{state}  ·  {} / {} 段", job.done, job.total), ACCENT);
        }
        if self.reader.signing_in() {
            return ("请在登录窗口完成 CodeBuddy 授权…".into(), ACCENT);
        }
        if let Some(error) = &self.reader.translation_error {
            return (error.clone(), ERROR);
        }
        if self.reader.checking_models {
            return ("正在读取账号可用模型…".into(), MUTED);
        }
        if let Some(error) = &self.reader.models_error {
            return (error.clone(), ERROR);
        }
        if self.reader.models.is_empty() {
            return ("当前账号没有可用的翻译模型".into(), MUTED);
        }
        if self.reader.model_save_failed {
            return ("无法保存上次选用的模型。".into(), ERROR);
        }
        if let Some(document) = &self.reader.document {
            let translated = self.reader.translated_count;
            if translated > 0 {
                let id = self.reader.translation_model.as_deref().unwrap_or_default();
                let model = self.reader.model_name(id).unwrap_or(id);
                return (format!("已翻译 {translated} 段  ·  {model}"), ACCENT);
            }
            if self.reader.model.is_none() {
                return ("请选择翻译模型".into(), MUTED);
            }
            return (format!("{} 页", document.pages), MUTED);
        }
        ("CodeBuddy 已连接".into(), ACCENT)
    }

    fn paint(&mut self, hwnd: HWND, dc: HDC, area: RECT) {
        let signed_in = self.reader.signed_in;
        let header = self.s(if signed_in { 108 } else { 60 });
        let footer = area.bottom - self.s(34);
        let body = rc(0, header, area.right, footer.max(header));
        self.viewport_height = body.bottom - body.top;
        if signed_in {
            self.load_reading_position();
        }
        self.selection.begin_frame(body);
        self.hits.clear();
        self.copy_targets.clear();
        let painter = Painter { dc, fonts: self.fonts };
        painter.fill(area, CANVAS);
        self.paint_toolbar(&painter, area.right);
        let clip = unsafe { SaveDC(dc) };
        unsafe { IntersectClipRect(dc, body.left, body.top, body.right, body.bottom) };
        if !signed_in {
            self.paint_login(&painter, body);
        } else if self.reader.document.is_some() {
            if self.pdf_view {
                self.paint_original(&painter, body);
            } else {
                self.paint_bilingual(&painter, body);
            }
        } else if !self.reader.loading && !self.reader.recent_documents.entries().is_empty() {
            self.paint_recent(&painter, body);
        } else {
            self.body_height = 0;
            self.paint_empty(&painter, body);
        }
        if clip != 0 {
            unsafe { RestoreDC(dc, clip) };
        }
        if signed_in {
            self.max_scroll = (self.body_height - self.viewport_height).max(0);
            self.scroll = self.scroll.clamp(0, self.max_scroll);
        }
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS | SIF_DISABLENOSCROLL,
            nMin: 0,
            nMax: if signed_in { self.body_height.saturating_sub(1).max(0) } else { 0 },
            nPage: self.viewport_height.max(0) as u32,
            nPos: if signed_in { self.scroll } else { 0 },
            nTrackPos: 0,
        };
        unsafe { SetScrollInfo(hwnd, SB_VERT, &info, 1) };
        self.paint_footer(&painter, area.right, footer, area.bottom);
    }

    fn paint_toolbar(&mut self, p: &Painter, width: i32) {
        let x = self.s(22);
        p.fill(rc(0, 0, width, self.s(if self.reader.signed_in { 108 } else { 60 })), WHITE);
        p.round(rc(x, self.s(14), x + self.s(31), self.s(45)), self.s(8), ACCENT, ACCENT);
        p.fill(rc(x + self.s(7), self.s(23), x + self.s(14), self.s(35)), WHITE);
        p.fill(rc(x + self.s(17), self.s(23), x + self.s(24), self.s(35)), color(164, 222, 205));
        p.line_text("文献阅读器", BRAND, TEXT, rc(x + self.s(42), 0, x + self.s(155), self.s(59)), DT_LEFT);
        p.fill(rc(0, self.s(59), width, self.s(60)), LINE);
        if !self.reader.signed_in {
            return;
        }
        let open_left = x + self.s(if width >= self.s(920) { 171 } else { 145 });
        let open_rect = rc(open_left, self.s(14), open_left + self.s(98), self.s(45));
        self.button(p, open_rect, "打开 PDF", Some(Action::Open), false);

        let account = rc(width - x - self.s(34), self.s(13), width - x, self.s(47));
        self.paint_account(p, account);
        let model_width = self.s(if width > self.s(930) { 228 } else { 156 });
        let model_right = account.left - self.s(12);
        let model_rect = rc(model_right - model_width, self.s(14), model_right, self.s(45));
        let model_label = self.reader.model.as_deref()
            .map(|id| format!("{}  ▾", self.reader.model_name(id).unwrap_or(id)))
            .unwrap_or_else(|| "翻译模型  ▾".into());
        self.button(p, model_rect, &model_label, Some(Action::Models), false);
        let mut title_right = model_rect.left - self.s(12);
        if let Some(job) = &self.reader.translation {
            let (label, ink) = if job.stopping() {
                ("正在停止…".into(), MUTED)
            } else if let Some(rate) = job.output_rate() {
                (format!("≈ {rate:.1} token/s"), ACCENT_DARK)
            } else {
                ("等待模型输出…".into(), MUTED)
            };
            let bounds = rc(title_right - self.s(112), self.s(17), title_right, self.s(43));
            p.round(bounds, self.s(13), TINT, LINE);
            p.line_text(&label, SMALL, ink, bounds, DT_CENTER);
            title_right = bounds.left - self.s(8);
        }
        if let Some(path) = &self.reader.path {
            let file = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
            let title = self.reader.document.as_ref().map_or(file.as_ref(), |document| document.title.as_str());
            p.line_text(title, SMALL, MUTED,
                rc(open_rect.right + self.s(16), 0, title_right, self.s(59)), DT_LEFT);
        }

        p.fill(rc(0, self.s(107), width, self.s(108)), LINE);
        let Some(pages) = self.reader.document.as_ref().map(|document| document.pages) else { return };
        let top = self.s(68);
        let bottom = self.s(99);
        self.button(p, rc(x, top, x + self.s(82), bottom), "双语阅读", Some(Action::Bilingual), !self.pdf_view);
        self.button(p, rc(x + self.s(88), top, x + self.s(170), bottom), "PDF 原页", Some(Action::Pdf), self.pdf_view);
        if self.pdf_view {
            let page = self.reader.page;
            let navigation = x + self.s(190);
            self.button(p, rc(navigation, top, navigation + self.s(36), bottom), "‹", (page > 1).then_some(Action::Previous), false);
            self.button(p, rc(navigation + self.s(42), top, navigation + self.s(130), bottom), &format!("{page} / {pages}  ▾"), Some(Action::PageMenu), false);
            self.button(p, rc(navigation + self.s(136), top, navigation + self.s(172), bottom), "›", (page < pages).then_some(Action::Next), false);
        }
        if !self.reader.recent_documents.entries().is_empty() {
            let left = x + self.s(if self.pdf_view { 376 } else { 190 });
            self.button(p, rc(left, top, left + self.s(102), bottom), "最近阅读  ▾", Some(Action::RecentMenu), false);
        }
        let right = width - x;
        if let Some(job) = &self.reader.translation {
            let stopping = job.stopping();
            let label = if stopping { "正在停止" } else { "停止翻译" };
            self.button(p, rc(right - self.s(92), top, right, bottom), label, (!stopping).then_some(Action::Stop), false);
        } else if self.reader.can_translate() && self.reader.has_pending_translation() {
            let label = if self.reader.translation_error.is_some() { "重试翻译" } else { "继续翻译" };
            self.button(p, rc(right - self.s(92), top, right, bottom), label, Some(Action::TranslateAll), false);
        }
    }

    fn paint_account(&mut self, p: &Painter, bounds: RECT) {
        let hovered = self.hover == Some(Action::Account);
        let border = if hovered { ACCENT } else { LINE };
        p.round(bounds, self.s(34), TINT, border);
        let initial = self.reader.account.as_ref()
            .and_then(|account| account.display_name.as_deref())
            .and_then(|name| name.chars().find(|character| character.is_alphanumeric()));
        if let Some(initial) = initial {
            let text: String = initial.to_uppercase().collect();
            p.line_text(&text, MEDIUM, ACCENT_DARK, bounds, DT_CENTER);
        } else {
            let x = (bounds.left + bounds.right) / 2;
            let y = bounds.top;
            p.round(rc(x - self.s(4), y + self.s(7), x + self.s(4), y + self.s(15)), self.s(8), ACCENT, ACCENT);
            p.round(rc(x - self.s(8), y + self.s(17), x + self.s(8), y + self.s(26)), self.s(9), ACCENT, ACCENT);
        }
        p.round(rc(bounds.right - self.s(10), bounds.bottom - self.s(10), bounds.right, bounds.bottom),
            self.s(10), ACCENT, WHITE);
        self.hits.push(Hit { rect: bounds, action: Action::Account });
    }

    fn paint_footer(&mut self, p: &Painter, width: i32, top: i32, bottom: i32) {
        p.fill(rc(0, top, width, bottom), WHITE);
        p.fill(rc(0, top, width, top + 1), LINE);
        let (status, ink) = self.status();
        let left = self.s(22);
        let has_details = self.reader.models_error.is_some() || self.reader.signed_in && (
            self.reader.translation_error.is_some() || self.reader.load_error.is_some()
                || self.reader.document.as_ref().is_some_and(|pdf| !pdf.notices.is_empty())
        );
        let right = width - left;
        let details = rc(right - self.s(60), top + self.s(4), right, bottom - self.s(4));
        if has_details {
            self.button(p, details, "详情", Some(Action::Details), false);
        }
        let update = self.updates.button();
        let update_right = if has_details { details.left - self.s(8) } else { right };
        let update_bounds = rc(update_right - self.s(136), top + self.s(4), update_right, bottom - self.s(4));
        self.button(p, update_bounds, &update.label, update.enabled.then_some(Action::Update), update.primary);
        if !status.is_empty() {
            p.fill(rc(left, top + self.s(14), left + self.s(6), top + self.s(20)), ink);
            p.line_text(&status, SMALL, ink, rc(left + self.s(15), top + 1,
                update_bounds.left - self.s(12), bottom), DT_LEFT);
        }
    }

    fn paint_login(&mut self, p: &Painter, body: RECT) {
        let width = self.s(384).min(body.right - self.s(48));
        let height = self.s(176);
        let x = (body.right - width) / 2;
        let top = body.top + ((body.bottom - body.top - height) / 2).max(self.s(16));
        let center = body.right / 2;
        p.round(rc(x, top, x + width, top + height), self.s(16), WHITE, LINE);
        p.line_text("登录后开始阅读", TITLE, TEXT,
            rc(x, top + self.s(25), x + width, top + self.s(79)), DT_CENTER);
        let button_top = top + self.s(96);
        let button_bottom = top + self.s(140);
        let signing_in = self.reader.signing_in();
        if signing_in && self.login_link.is_some() {
            self.button(p, rc(center - self.s(143), button_top, center + self.s(31), button_bottom),
                "复制登录链接", Some(Action::CopyLoginLink), true);
            self.button(p, rc(center + self.s(43), button_top, center + self.s(143), button_bottom),
                "取消登录", Some(Action::Login), false);
            return;
        }
        let label = if signing_in { "取消登录" } else { "登录 CodeBuddy" };
        let action = (!self.reader.checking_models).then_some(Action::Login);
        self.button(p, rc(center - self.s(92), button_top, center + self.s(92), button_bottom),
            label, action, !signing_in);
    }

    fn paint_empty(&mut self, p: &Painter, body: RECT) {
        let width = self.s(440).min(body.right - self.s(48));
        let x = (body.right - width) / 2;
        let center = body.right / 2;
        let top = body.top + ((body.bottom - body.top - self.s(236)) / 2).max(self.s(16));
        p.round(rc(x, top, x + width, top + self.s(224)), self.s(18), WHITE, LINE);
        p.round(rc(center - self.s(22), top + self.s(24), center + self.s(22), top + self.s(68)), self.s(11), TINT, TINT);
        for line in 0..3 {
            let ly = top + self.s(36 + line * 8);
            p.fill(rc(center - self.s(10), ly, center + self.s(if line == 2 { 5 } else { 10 }), ly + self.s(2)), ACCENT);
        }
        let failed = self.reader.load_error.is_some();
        let (title, subtitle) = if self.reader.loading {
            ("正在打开论文", "正在识别段落、插图和公式")
        } else if failed {
            ("无法打开这份 PDF", "查看状态栏详情，或打开另一份 PDF")
        } else {
            ("打开一份论文", "拖放 PDF 到这里，或从本机选择文件")
        };
        p.line_text(title, TITLE, TEXT, rc(x, top + self.s(77), x + width, top + self.s(117)), DT_CENTER);
        p.line_text(subtitle, BODY, MUTED, rc(x, top + self.s(119), x + width, top + self.s(148)), DT_CENTER);
        if !self.reader.loading {
            self.button(p, rc(center - self.s(64), top + self.s(164), center + self.s(64), top + self.s(202)), "选择 PDF", Some(Action::Open), true);
        }
    }

    fn paint_recent(&mut self, p: &Painter, body: RECT) {
        let width = (body.right - self.s(48)).min(self.s(760)).max(1);
        let left = (body.right - width) / 2;
        let right = left + width;
        let header = self.s(72);
        let row_height = self.s(62);
        let entries = self.reader.recent_documents.entries();
        self.body_height = header + row_height * entries.len() as i32 + self.s(26);
        self.scroll = self.scroll.min((self.body_height - self.viewport_height).max(0));
        let top = body.top - self.scroll;
        p.line_text("最近阅读", TITLE, TEXT, rc(left, top + self.s(19), right, top + self.s(62)), DT_LEFT);
        p.round(rc(left, top + header, right, top + header + row_height * entries.len() as i32), self.s(10), WHITE, LINE);
        for (index, entry) in entries.iter().enumerate() {
            let y = top + header + row_height * index as i32;
            let row = rc(left, y, right, y + row_height);
            let visible = clip_rect(row, body);
            if visible.bottom <= visible.top {
                continue;
            }
            let action = Action::OpenRecent(index);
            if self.hover == Some(action) {
                p.round(rc(left + self.s(3), y + self.s(3), right - self.s(3), y + row_height - self.s(3)), self.s(7), TINT, TINT);
            }
            let text_left = left + self.s(16);
            let text_right = right - self.s(16);
            p.line_text(&entry.title, MEDIUM, TEXT, rc(text_left, y + self.s(7), text_right, y + self.s(32)), DT_LEFT);
            p.text(&entry.path.to_string_lossy(), SMALL, MUTED,
                rc(text_left, y + self.s(32), text_right, y + self.s(52)), DT_SINGLELINE | DT_VCENTER | DT_PATH_ELLIPSIS);
            if index + 1 < entries.len() {
                p.fill(rc(text_left, row.bottom - 1, text_right, row.bottom), LINE);
            }
            self.hits.push(Hit { rect: visible, action });
        }
    }

    fn paint_original(&mut self, p: &Painter, body: RECT) {
        let key = ImageKey::Page(self.reader.page);
        self.reader.request_image(key);
        let Some(image) = self.reader.image(key) else {
            let message = self.reader.image_error(key).unwrap_or("正在渲染 PDF 原页…");
            p.line_text(message, BODY, MUTED, rc(0, body.top, body.right, body.top + self.s(150)), DT_CENTER);
            self.body_height = self.viewport_height.saturating_add(self.scroll);
            return;
        };
        let width = (body.right - self.s(64)).min(self.s(900)).max(1);
        let height = raster_height(image, width);
        self.body_height = height + self.s(48);
        let maximum = (self.body_height - self.viewport_height).max(0);
        self.scroll = self.scroll.min(maximum);
        if let Some(restored) = self.reading_position.take_original_scroll(self.dpi, maximum) {
            self.scroll = restored;
        }
        let x = (body.right - width) / 2;
        let y = body.top + self.s(24) - self.scroll;
        p.image(image, rc(x, y, x + width, y + height));
        p.frame(rc(x - 1, y - 1, x + width + 1, y + height + 1), LINE);
    }

    fn first_visible_row(&self) -> usize {
        self.rows.partition_point(|row| row.y + row.height <= self.scroll)
    }

    fn layout_bilingual(&mut self, p: &Painter, width: i32) {
        let key = LayoutKey { width, dpi: self.dpi, generation: self.reader.generation };
        let reflow = self.layout_key != Some(key);
        let reset = self.translation_layout_reset != self.reader.translation_layout_reset;
        let changed = self.reader.take_changed_translations();
        if !reflow && !reset && changed.is_empty() {
            return;
        }
        if reset {
            self.clear_text_selection();
        }

        let anchor = self.layout_key.filter(|previous| previous.generation == key.generation)
            .and_then(|previous| {
                let index = self.first_visible_row();
                let row = self.rows.get(index)?;
                Some((index, rescale(self.scroll - row.y, previous.dpi, key.dpi)))
            });
        let document = self.reader.document.as_ref().unwrap();
        let content_width = (width - self.s(PAPER_INSET) * 2).max(1);
        let mut first_changed = 0;

        if reflow {
            let (contents, bibliography) = paper_contents(document);
            let mut source_rows = vec![None; document.blocks.len()];
            let mut translation_rows = vec![None; document.blocks.len()];
            self.rows = contents.iter().enumerate().map(|(index, content)| {
                if let Some(block) = content.original_block(document) {
                    source_rows[block] = Some(index);
                }
                if let RowKind::Media { .. } = content.kind {
                    source_rows[content.block] = Some(index);
                }
                if let Some(block) = content.translated_block(document) {
                    translation_rows[block] = Some(index);
                }
                self.measure_row(p, document, *content, content_width)
            }).collect();
            self.source_rows = source_rows;
            self.translation_rows = translation_rows;
            self.bibliography = bibliography;
        } else {
            first_changed = self.rows.len();
            let mut update = |index: usize| {
                let row = &mut self.rows[index];
                let (next, layout) = measure_translation(
                    p, &self.reader, document, row.content, content_width, self.dpi,
                );
                let delta = next - row.translated_height;
                row.translated_height = next;
                row.translated_layout = layout;
                if delta != 0 {
                    row.height += delta;
                    first_changed = first_changed.min(index);
                }
            };
            if reset {
                for index in self.translation_rows.iter().flatten().copied() {
                    update(index);
                }
            } else {
                for index in changed.iter().filter_map(|block| self.translation_rows[*block]) {
                    update(index);
                }
            }
        }

        let mut y = if first_changed == 0 {
            self.s(16)
        } else {
            let previous = &self.rows[first_changed - 1];
            previous.y + previous.height + previous.gap
        };
        for row in &mut self.rows[first_changed..] {
            row.y = y;
            y += row.height + row.gap;
        }
        if let Some((index, offset)) = anchor {
            if let Some(row) = self.rows.get(index) {
                self.scroll = (row.y + offset.min((row.height - 1).max(0))).max(0);
            }
        }
        self.layout_key = Some(key);
        self.translation_layout_reset = self.reader.translation_layout_reset;
    }

    fn measure_row(&self, p: &Painter, document: &Document, content: RowContent, width: i32) -> Row {
        let block = &document.blocks[content.block];
        let (top, bottom, gap) = match content.kind {
            RowKind::Media { .. } if block.kind == BlockKind::Equation => (5, 5, 4),
            RowKind::Media { .. } => (14, 14, 14),
            RowKind::ListSource { first } => (if first { 7 } else { 0 }, 1, 0),
            RowKind::ListTranslation { first, last } => (if first { 8 } else { 0 }, 0, if last { 12 } else { 0 }),
            RowKind::ReferenceHeading => (17, 10, 0),
            RowKind::ReferenceTranslationHeading => (14, 10, 0),
            RowKind::ReferenceSource | RowKind::ReferenceTranslation => (6, 8, 0),
            RowKind::Text => match block.kind {
                BlockKind::Title => (22, 10, 9),
                BlockKind::Author => (3, 3, 4),
                BlockKind::Heading => (13, 3, 12),
                _ => (3, 3, 12),
            },
        };
        let (original_height, original_layout) = content.original_text(document).map_or((0, None), |text| {
            let font = content.original_font(document);
            let runs = if content.supports_inline_math(document) {
                split_inline(text)
            } else {
                vec![(text, false)]
            };
            let layout = NativeText::layout(p, runs, font, width, self.dpi);
            (layout.as_ref().map_or(0, NativeText::height).max(self.s(18)), layout)
        });
        let (translated_height, translated_layout) =
            measure_translation(p, &self.reader, document, content, width, self.dpi);
        let equation = document.native_equations.get(&content.block)
            .and_then(|text| NativeMath::display(p, text, width, self.dpi));
        let image_height = match content.kind {
            RowKind::Media { .. } => equation.as_ref().map_or_else(
                || self.media_height(block, width),
                NativeMath::height,
            ),
            _ => 0,
        };
        let (links, links_height) = match content.kind {
            RowKind::Text | RowKind::ListSource { .. } | RowKind::Media { caption: Some(_) } => {
                let links = content.original_block(document)
                    .and_then(|index| self.reader.citation_links.get(index));
                self.layout_links(p, width, links.into_iter().flatten()
                    .map(|link| (link.label.as_str(), Action::JumpCitation(link.target))))
            }
            RowKind::ReferenceSource => {
                let links = self.reader.reference_links.get(content.block);
                self.layout_links(p, width, links.into_iter().flatten().enumerate()
                    .map(|(link, source)| (source.label.as_str(), Action::OpenSource {
                        block: content.block, link,
                    })))
            }
            _ => (Vec::new(), 0),
        };
        let top = self.s(top);
        let bottom = self.s(bottom);
        let gap = self.s(gap);
        let caption_gap = if image_height > 0 && original_height > 0 { self.s(12) } else { 0 };
        let translated_gap = if translated_height > 0 && original_height > 0 { self.s(8) } else { 0 };
        let height = top + image_height + caption_gap + original_height + links_height
            + translated_gap + translated_height + bottom;
        Row {
            content, y: 0, height, original_height, translated_height, image_height,
            original_layout, translated_layout, equation,
            links, links_height, top, gap,
        }
    }

    fn media_height(&self, block: &Block, width: i32) -> i32 {
        let bounds = block.bounds;
        let source_width = (bounds.right - bounds.left).max(1.0);
        let source_height = (bounds.bottom - bounds.top).max(1.0);
        if block.kind == BlockKind::Equation {
            // Equation crops include four points of padding on each side.
            let scale = (width as f32 / (source_width + 8.0)).min(self.dpi as f32 / 72.0 * 1.18);
            return ((source_height + 8.0) * scale).round().max(self.s(22) as f32) as i32;
        }
        let (min, max) = match block.kind {
            BlockKind::Table => (76, 660),
            _ => (105, 540),
        };
        let scale = (width as f32 / source_width).min(self.dpi as f32 / 72.0 * 1.5);
        (source_height * scale).round().clamp(self.s(min) as f32, self.s(max) as f32) as i32
    }

    fn layout_links<'a>(
        &self,
        p: &Painter,
        width: i32,
        links: impl Iterator<Item = (&'a str, Action)>,
    ) -> (Vec<PaperLink>, i32) {
        let padding = self.s(10);
        let gap = self.s(8);
        let step = self.s(26);
        let mut items = Vec::new();
        let mut x = 0;
        let mut y = self.s(4);
        for (label, action) in links {
            let size = (p.text_width(label, PAPER_LINK) + padding).min(width);
            if x > 0 && x + size > width {
                y += step;
                x = 0;
            }
            items.push(PaperLink { label: label.to_owned(), action, x, y, width: size });
            x += size + gap;
        }
        let height = if items.is_empty() { 0 } else { y + step };
        (items, height)
    }

    fn paint_bilingual(&mut self, p: &Painter, body: RECT) {
        let width = (body.right - self.s(48)).min(self.s(880)).max(1);
        let x = (body.right - width) / 2;
        self.layout_bilingual(p, width);
        self.restore_bilingual_position();
        self.body_height = self.rows.last().map_or(0, |row| row.y + row.height + self.s(24));
        self.scroll = self.scroll.min((self.body_height - self.viewport_height).max(0));
        let y_offset = body.top - self.scroll;

        if self.rows.is_empty() {
            let message = "这份 PDF 没有可提取的文字或插图，可切换到 PDF 原页。";
            p.line_text(message, BODY, MUTED, rc(x, body.top, x + width, body.top + self.s(150)), DT_CENTER);
            return;
        }

        let paper = rc(x, y_offset + self.s(10), x + width, y_offset + self.body_height - self.s(10));
        p.round(paper, self.s(13), WHITE, LINE);
        if let Some((first, last)) = self.bibliography {
            let inset = self.s(13);
            let section = rc(
                x + inset, y_offset + self.rows[first].y, x + width - inset,
                y_offset + self.rows[last].y + self.rows[last].height,
            );
            if section.bottom > body.top && section.top < body.bottom {
                p.round(section, self.s(9), color(249, 251, 250), LINE);
            }
        }

        let document = self.reader.document.as_ref().unwrap().clone();
        let inset = self.s(PAPER_INSET);
        let bounds = rc(x + inset, body.top, x + width - inset, body.bottom);
        let first = self.first_visible_row();
        // Drawing a row may populate image requests and click targets. Keep rows
        // separate from mutable UI state while drawing just the visible slice.
        let paint = PaperPaint {
            p, document: &document, body, left: bounds.left, right: bounds.right, y_offset,
            dpi: self.dpi, hover: self.hover, focused_reference: self.focused_reference,
        };
        for (index, row) in self.rows.iter().enumerate().skip(first) {
            if y_offset + row.y >= body.bottom {
                break;
            }
            paint.row(index, row, &mut self.reader, &mut self.hits, &mut self.copy_targets, &mut self.selection);
        }
    }

    fn button(&mut self, p: &Painter, bounds: RECT, label: &str, action: Option<Action>, primary: bool) {
        let active = action.is_some();
        let hovered = active && action == self.hover;
        let (fill, border, ink) = if primary && active {
            (if hovered { ACCENT_DARK } else { ACCENT }, ACCENT, WHITE)
        } else if hovered {
            (TINT, ACCENT, ACCENT_DARK)
        } else if active {
            (WHITE, LINE, TEXT)
        } else {
            (CANVAS, LINE, color(119, 139, 134))
        };
        p.round(bounds, self.s(7), fill, border);
        let inset = self.s(6);
        p.line_text(label, MEDIUM, ink, rc(bounds.left + inset, bounds.top, bounds.right - inset, bounds.bottom), DT_CENTER);
        if let Some(action) = action {
            self.hits.push(Hit { rect: bounds, action });
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        self.flush_reading_position();
        self.reader.cancel_translation();
        self.reader.cancel_login();
        for font in self.fonts {
            unsafe { DeleteObject(font) };
        }
    }
}

fn paper_contents(document: &Document) -> (Vec<RowContent>, Option<(usize, usize)>) {
    let references: Vec<_> = document.blocks.iter().enumerate()
        .filter_map(|(index, block)| (block.kind == BlockKind::Reference).then_some(index))
        .collect();
    let references_start = references.first().map(|&first| {
        document.blocks[..first].iter().rposition(|block| block.kind == BlockKind::ReferenceHeading)
            .unwrap_or(first)
    });
    let paired_captions: HashSet<_> = document.asset_captions.values().copied().collect();
    let mut rows = Vec::with_capacity(document.blocks.len() + references.len());
    let mut bibliography = None;
    let mut list_end = 0;
    for (index, block) in document.blocks.iter().enumerate() {
        if Some(index) == references_start {
            let start = rows.len();
            rows.push(RowContent { block: index, kind: RowKind::ReferenceHeading });
            rows.extend(references.iter().map(|&block| RowContent {
                block, kind: RowKind::ReferenceSource,
            }));
            rows.push(RowContent { block: index, kind: RowKind::ReferenceTranslationHeading });
            rows.extend(references.iter().map(|&block| RowContent {
                block, kind: RowKind::ReferenceTranslation,
            }));
            bibliography = Some((start, rows.len() - 1));
            continue;
        }
        if index < list_end || paired_captions.contains(&index)
            || (references_start.is_some()
                && matches!(block.kind, BlockKind::Reference | BlockKind::ReferenceHeading))
        {
            continue;
        }
        if let Some(&end) = document.list_groups.get(&index) {
            rows.extend((index..end).map(|block| RowContent {
                block, kind: RowKind::ListSource { first: block == index },
            }));
            rows.extend((index..end).map(|block| RowContent {
                block, kind: RowKind::ListTranslation { first: block == index, last: block + 1 == end },
            }));
            list_end = end;
            continue;
        }
        let kind = match block.kind {
            BlockKind::Equation | BlockKind::Figure | BlockKind::Table => RowKind::Media {
                caption: document.asset_captions.get(&index).copied(),
            },
            _ => RowKind::Text,
        };
        rows.push(RowContent { block: index, kind });
    }
    (rows, bibliography)
}

impl RowContent {
    fn original_block(self, document: &Document) -> Option<usize> {
        match self.kind {
            RowKind::Text | RowKind::ListSource { .. } | RowKind::ReferenceSource => Some(self.block),
            RowKind::Media { caption } => caption,
            RowKind::ReferenceHeading if document.blocks[self.block].kind == BlockKind::ReferenceHeading => {
                Some(self.block)
            }
            _ => None,
        }
    }

    fn original_text(self, document: &Document) -> Option<&str> {
        self.original_block(document).map(|index| {
            document.native_inline.get(&index).unwrap_or(&document.blocks[index].text).as_str()
        })
            .or(match self.kind {
                RowKind::ReferenceHeading => Some("References"),
                RowKind::ReferenceTranslationHeading => Some("参考文献译文"),
                _ => None,
            })
    }

    fn supports_inline_math(self, document: &Document) -> bool {
        match self.kind {
            RowKind::Text | RowKind::ListSource { .. } => matches!(document.blocks[self.block].kind,
                BlockKind::Paragraph | BlockKind::Title | BlockKind::Heading | BlockKind::Caption),
            RowKind::Media { caption } => caption.is_some(),
            _ => false,
        }
    }

    fn translated_block(self, document: &Document) -> Option<usize> {
        match self.kind {
            RowKind::Text if is_translatable(document.blocks[self.block].kind) => Some(self.block),
            RowKind::Media { caption } => caption,
            RowKind::ListTranslation { .. } | RowKind::ReferenceTranslation => Some(self.block),
            _ => None,
        }
    }

    fn original_font(self, document: &Document) -> usize {
        match self.kind {
            RowKind::ReferenceHeading | RowKind::ReferenceTranslationHeading => PAPER_HEADING,
            RowKind::ReferenceSource | RowKind::Media { .. } => PAPER_SMALL,
            _ => paper_font(document.blocks[self.block].kind, false),
        }
    }

    fn translation_font(self, document: &Document) -> usize {
        match self.kind {
            RowKind::ReferenceTranslation | RowKind::Media { .. } => PAPER_SMALL_ZH,
            _ => paper_font(document.blocks[self.block].kind, true),
        }
    }

    fn alignment(self, document: &Document) -> u32 {
        match self.kind {
            RowKind::Text if matches!(document.blocks[self.block].kind, BlockKind::Title | BlockKind::Author) => DT_CENTER,
            _ => DT_LEFT,
        }
    }

    fn pending_translation_height(self, minimum: i32) -> i32 {
        if matches!(self.kind, RowKind::ListTranslation { first: false, .. }) { 0 } else { minimum }
    }
}

fn measure_translation(
    p: &Painter, reader: &Reader, document: &Document, content: RowContent, width: i32, dpi: u32,
) -> (i32, Option<NativeText>) {
    let Some(block) = content.translated_block(document) else {
        return (0, None);
    };
    let minimum = scale(20, dpi);
    let Some((text, ranges)) = reader.displayed_translation(block) else {
        return (content.pending_translation_height(minimum), None);
    };
    let font = content.translation_font(document);
    let layout = NativeText::layout(p, span_runs(text, ranges), font, width, dpi);
    (layout.as_ref().map_or(0, NativeText::height).max(minimum), layout)
}

struct PaperPaint<'a> {
    p: &'a Painter,
    document: &'a Document,
    body: RECT,
    left: i32,
    right: i32,
    y_offset: i32,
    dpi: u32,
    hover: Option<Action>,
    focused_reference: Option<usize>,
}

impl PaperPaint<'_> {
    fn row(&self, index: usize, row: &Row, reader: &mut Reader, hits: &mut Vec<Hit>,
        copy: &mut Vec<CopyTarget>, selection: &mut Selection,
    ) {
        let s = |pixels| scale(pixels, self.dpi);
        let content = row.content;
        let block = &self.document.blocks[content.block];
        let y = self.y_offset + row.y;
        let mut text_top = y + row.top;

        if let RowKind::Media { .. } = content.kind {
            if block.kind != BlockKind::Equation {
                let card = rc(self.left - s(10), y, self.right + s(10), y + row.height);
                self.p.round(card, s(8), WHITE, LINE);
            }
            let bounds = rc(self.left, text_top, self.right, text_top + row.image_height);
            if let Some(equation) = &row.equation {
                equation.draw_centered(self.p, bounds, color(24, 24, 24));
            } else {
                let key = ImageKey::Block(content.block);
                reader.request_image(key);
                if let Some(image) = reader.image(key) {
                    self.p.fit_image(image, bounds);
                } else {
                    let label = reader.image_error(key).unwrap_or(match block.kind {
                        BlockKind::Table => "正在渲染表格…",
                        BlockKind::Equation => "正在渲染公式…",
                        _ => "正在渲染插图…",
                    });
                    self.p.wrapped_text_aligned(label, SMALL, MUTED, bounds, DT_CENTER);
                }
            }
            text_top += row.image_height;
            if row.original_height > 0 {
                text_top += s(12);
            }
        }
        if matches!(content.kind, RowKind::ReferenceSource)
            && self.focused_reference == Some(content.block)
        {
            let highlight = rc(self.left - s(8), y, self.right + s(8), y + row.height);
            self.p.round(highlight, s(5), TINT, TINT);
        }
        if matches!(content.kind, RowKind::ReferenceTranslationHeading) {
            self.p.fill(rc(self.left, y + s(4), self.right, y + s(5)), LINE);
        }

        let alignment = content.alignment(self.document);
        if let Some(layout) = &row.original_layout {
            let ink = if block.kind == BlockKind::Author { MUTED } else { TEXT };
            let bounds = rc(self.left, text_top, self.right, text_top + row.original_height + 2);
            selection.draw_lines(self.p, TextPart { row: index, translated: false },
                layout.selection_lines(bounds, alignment, self.body), self.body);
            layout.draw(self.p, bounds, alignment, ink);
            if let Some(index) = content.original_block(self.document) {
                copy.push(CopyTarget { rect: clip_rect(bounds, self.body), index, translation: false });
            }
        }

        let links_top = text_top + row.original_height;
        for link in &row.links {
            let x = self.left + link.x;
            let y = links_top + link.y;
            let bounds = rc(x, y, x + link.width, y + s(24));
            let visible = clip_rect(bounds, self.body);
            if visible.bottom <= visible.top {
                continue;
            }
            if self.hover == Some(link.action) {
                self.p.round(bounds, s(4), TINT, TINT);
            }
            self.p.line_text(&link.label, PAPER_LINK, ACCENT_DARK, rc(
                bounds.left + s(3), bounds.top, bounds.right - s(3), bounds.bottom,
            ), DT_LEFT);
            hits.push(Hit { rect: visible, action: link.action });
        }

        if row.translated_height > 0 && let Some(block_index) = content.translated_block(self.document) {
            let y = links_top + row.links_height + if row.original_height > 0 { s(8) } else { 0 };
            let bounds = rc(self.left, y, self.right, y + row.translated_height + 2);
            if let Some(layout) = &row.translated_layout {
                let section_title = matches!(block.kind, BlockKind::Title | BlockKind::Heading);
                let ink = if section_title { ACCENT_DARK } else { TEXT };
                if matches!(content.kind, RowKind::Text | RowKind::ListTranslation { .. }) && !section_title {
                    self.p.fill(rc(self.left - s(9), y + s(2), self.left - s(7), bounds.bottom - s(1)), LINE);
                }
                if reader.translations[block_index].is_some() {
                    selection.draw_lines(self.p, TextPart { row: index, translated: true },
                        layout.selection_lines(bounds, alignment, self.body), self.body);
                    copy.push(CopyTarget { rect: clip_rect(bounds, self.body), index: block_index, translation: true });
                }
                layout.draw(self.p, bounds, alignment, ink);
            } else {
                self.p.wrapped_text_aligned("待翻译", SMALL, MUTED, bounds, alignment);
            }
        }
    }
}

struct Painter {
    dc: HDC,
    fonts: [HFONT; FONT_COUNT],
}

impl Painter {
    fn fill(&self, rect: RECT, ink: u32) {
        unsafe {
            SetDCBrushColor(self.dc, ink);
            FillRect(self.dc, &rect, GetStockObject(DC_BRUSH));
        }
    }

    fn frame(&self, rect: RECT, ink: u32) {
        unsafe {
            SetDCBrushColor(self.dc, ink);
            FrameRect(self.dc, &rect, GetStockObject(DC_BRUSH));
        }
    }

    fn round(&self, rect: RECT, diameter: i32, fill: u32, border: u32) {
        unsafe {
            let brush = SelectObject(self.dc, GetStockObject(DC_BRUSH));
            let pen = SelectObject(self.dc, GetStockObject(DC_PEN));
            SetDCBrushColor(self.dc, fill);
            SetDCPenColor(self.dc, border);
            RoundRect(self.dc, rect.left, rect.top, rect.right, rect.bottom, diameter, diameter);
            SelectObject(self.dc, pen);
            SelectObject(self.dc, brush);
        }
    }

    fn line_text(&self, text: &str, font: usize, ink: u32, bounds: RECT, align: u32) {
        self.text(text, font, ink, bounds, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | align);
    }

    fn wrapped_text_aligned(&self, text: &str, font: usize, ink: u32, bounds: RECT, align: u32) {
        self.text(text, font, ink, bounds, DT_WORDBREAK | DT_EDITCONTROL | align);
    }

    fn text_width(&self, text: &str, font: usize) -> i32 {
        let wide: Vec<_> = text.encode_utf16().collect();
        self.run_width(&wide, font)
    }

    fn text(&self, text: &str, font: usize, ink: u32, mut bounds: RECT, flags: u32) -> i32 {
        if bounds.right <= bounds.left || text.is_empty() {
            return 0;
        }
        let wide: Vec<_> = text.encode_utf16().collect();
        unsafe {
            let previous = SelectObject(self.dc, self.fonts[font]);
            SetBkMode(self.dc, TRANSPARENT as i32);
            SetTextColor(self.dc, ink);
            let height = DrawTextW(self.dc, wide.as_ptr(), wide.len() as i32, &mut bounds, flags | DT_NOPREFIX);
            SelectObject(self.dc, previous);
            height
        }
    }

    fn image(&self, raster: &Raster, rect: RECT) {
        let mut info: BITMAPINFO = unsafe { zeroed() };
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = raster.width;
        info.bmiHeader.biHeight = -raster.height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        unsafe {
            SetStretchBltMode(self.dc, HALFTONE as i32);
            SetBrushOrgEx(self.dc, 0, 0, null_mut());
            StretchDIBits(self.dc, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top,
                0, 0, raster.width, raster.height, raster.bgra.as_ptr().cast(), &info, DIB_RGB_COLORS, SRCCOPY);
        }
    }

    fn fit_image(&self, image: &Raster, area: RECT) {
        let ratio = ((area.right - area.left) as f64 / f64::from(image.width))
            .min((area.bottom - area.top) as f64 / f64::from(image.height))
            .min(1.25);
        let width = (f64::from(image.width) * ratio).round() as i32;
        let height = (f64::from(image.height) * ratio).round() as i32;
        let x = (area.left + area.right - width) / 2;
        let y = (area.top + area.bottom - height) / 2;
        self.image(image, rc(x, y, x + width, y + height));
    }
}

fn scale(pixels: i32, dpi: u32) -> i32 {
    (pixels * dpi as i32 + 48) / 96
}

fn rescale(pixels: i32, from_dpi: u32, to_dpi: u32) -> i32 {
    (i64::from(pixels) * i64::from(to_dpi) / i64::from(from_dpi.max(1))) as i32
}

fn make_fonts(dpi: u32) -> [HFONT; FONT_COUNT] {
    const STYLES: [(&str, i32, i32, bool); UI_FONT_COUNT] = [
        ("Microsoft YaHei UI", 16, 400, false),
        ("Microsoft YaHei UI", 13, 400, false),
        ("Microsoft YaHei UI", 14, 600, false),
        ("Microsoft YaHei UI", 24, 700, false),
        ("Microsoft YaHei UI", 17, 700, false),
        ("Times New Roman", 30, 700, false),
        ("Microsoft YaHei UI", 21, 600, false),
        ("Microsoft YaHei UI", 22, 700, false),
        ("Times New Roman", 16, 400, false),
        ("Times New Roman", 18, 400, false),
        ("SimSun", 17, 400, false),
        ("Times New Roman", 15, 400, false),
        ("SimSun", 15, 400, false),
        ("Microsoft YaHei UI", 13, 400, true),
    ];
    std::array::from_fn(|index| {
        let (family, size, weight, underline, italic) = match STYLES.get(index).copied() {
            Some((family, size, weight, underline)) => (family, size, weight, underline, false),
            None => {
                let (family, size, italic) = math::font_style(index - UI_FONT_COUNT);
                (family, size, 400, false, italic)
            }
        };
        let face = wide(family);
        unsafe {
            CreateFontW(-scale(size, dpi), 0, 0, 0, weight, italic.into(), underline.into(), 0,
                DEFAULT_CHARSET.into(), OUT_DEFAULT_PRECIS.into(), CLIP_DEFAULT_PRECIS.into(),
                CLEARTYPE_QUALITY.into(), DEFAULT_PITCH.into(), face.as_ptr())
        }
    })
}

fn paper_font(kind: BlockKind, translated: bool) -> usize {
    match kind {
        BlockKind::Title if translated => PAPER_TITLE_ZH,
        BlockKind::Title => PAPER_TITLE,
        BlockKind::Author => PAPER_AUTHOR,
        BlockKind::Heading | BlockKind::ReferenceHeading => PAPER_HEADING,
        BlockKind::Caption | BlockKind::Reference if translated => PAPER_SMALL_ZH,
        BlockKind::Caption | BlockKind::Reference => PAPER_SMALL,
        _ if translated => PAPER_BODY_ZH,
        _ => PAPER_BODY,
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn raster_height(image: &Raster, width: i32) -> i32 {
    (i64::from(image.height) * i64::from(width) / i64::from(image.width)) as i32
}

const fn color(r: u32, g: u32, b: u32) -> u32 { r | g << 8 | b << 16 }

fn rc(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT { left, top, right, bottom }
}

fn contains(rect: RECT, p: POINT) -> bool {
    p.x >= rect.left && p.x < rect.right && p.y >= rect.top && p.y < rect.bottom
}

fn clip_rect(rect: RECT, clip: RECT) -> RECT {
    rc(rect.left.max(clip.left), rect.top.max(clip.top), rect.right.min(clip.right), rect.bottom.min(clip.bottom))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::Bounds;
    use crate::reading_position::{ReadingPosition, Section};

    #[test]
    fn paper_rows_group_lists_and_bind_assets_and_references() {
        use BlockKind::*;
        let kinds = [
            Title, Author, Paragraph, Caption, Figure, ReferenceHeading, Reference,
            Reference, Heading, Table, Caption, Paragraph, Paragraph, Heading,
        ];
        let document = Document {
            title: String::new(),
            pages: 2,
            blocks: kinds.into_iter().enumerate().map(|(index, kind)| Block {
                kind, page: 1, text: String::new(),
                bounds: Bounds { left: 0.0, top: index as f32, right: 1.0, bottom: index as f32 + 1.0 },
            }).collect(),
            asset_captions: [(4, 3), (9, 10)].into_iter().collect(),
            native_equations: Default::default(),
            native_inline: Default::default(),
            paragraph_continuations: [(2, vec![(2, Bounds { left: 4.0, top: 5.0, right: 8.0, bottom: 9.0 })])]
                .into_iter().collect(),
            list_groups: [(11, 13)].into_iter().collect(),
            notices: Vec::new(),
        };
        let (rows, section) = paper_contents(&document);
        assert_eq!(rows.iter().map(|row| row.block).collect::<Vec<_>>(),
            vec![0, 1, 2, 4, 5, 6, 7, 5, 6, 7, 8, 9, 11, 12, 11, 12, 13]);
        assert_eq!(section, Some((4, 9)));
        assert_eq!(rows[1].translated_block(&document), None);
        assert_eq!(rows[3].original_block(&document), Some(3));
        assert_eq!(rows[3].translated_block(&document), Some(3));
        assert_eq!(rows[5].original_block(&document), Some(6));
        assert_eq!(rows[8].translated_block(&document), Some(6));
        assert_eq!(rows[11].translated_block(&document), Some(10));
        assert_eq!(rows[12].original_block(&document), Some(11));
        assert_eq!(rows[13].original_block(&document), Some(12));
        assert_eq!(rows[12].translated_block(&document), None);
        assert_eq!(rows[14].translated_block(&document), Some(11));
        assert_eq!(rows[15].translated_block(&document), Some(12));
        assert_eq!(rows[14].pending_translation_height(20), 20);
        assert_eq!(rows[15].pending_translation_height(20), 0);
        let saved_reference = ReadingPosition::Bilingual {
            block: 8, page: 1, left: 0.0, top: 7.0, section: Section::TranslatedReference,
        };
        assert_eq!(position::anchor_row(rows.iter().copied(), &document, saved_reference), Some(9));
        let saved_continuation = ReadingPosition::Bilingual {
            block: 3, page: 2, left: 4.0, top: 5.0, section: Section::Content,
        };
        assert_eq!(position::anchor_row(rows.iter().copied(), &document, saved_continuation), Some(2));
        let saved_list_translation = ReadingPosition::Bilingual {
            block: 12, page: 1, left: 0.0, top: 12.0, section: Section::TranslatedList,
        };
        assert_eq!(position::anchor_row(rows.iter().copied(), &document, saved_list_translation), Some(15));
    }
}
