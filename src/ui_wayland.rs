use crate::pdf::BlockKind;
use crate::reader::{ImageKey, Raster, Reader, is_translatable};
use crate::reading_position::{ReadingPosition, Section};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

mod account;
mod updates;
use account::AccountPanel;
use updates::UpdateControl;

const APP_ID: &str = "io.github.zhhpan.LiteratureReader";

pub fn run() -> i32 {
    gdk::set_allowed_backends("wayland");
    if gtk::init().is_err() {
        eprintln!("无法打开 Wayland 显示。请在运行中的 Wayland 合成器会话内启动。");
        return 1;
    }
    let argument = std::env::args_os().nth(1);
    let application = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::NON_UNIQUE);
    let smoke = argument.as_deref().is_some_and(|arg| arg == "--wayland-smoke");
    let passed = Rc::new(Cell::new(false));
    let smoke_passed = passed.clone();
    application.connect_activate(move |application| {
        if smoke {
            let window = gtk::ApplicationWindow::builder().application(application)
                .title("Literature Reader").default_width(640).default_height(480).child(&brand()).build();
            let application = application.clone();
            let passed = smoke_passed.clone();
            window.connect_map(move |window| {
                if WidgetExt::display(window).type_().name() == "GdkWaylandDisplay" {
                    passed.set(true);
                    println!("WAYLAND_SMOKE_OK");
                }
                application.quit();
            });
            window.present();
        } else {
            App::open_window(application, argument.clone().map(PathBuf::from));
        }
    });
    let exit = i32::from(application.run_with_args(&["literature-reader"]));
    if smoke && !passed.get() { 1 } else { exit }
}

struct PaperRow {
    position: gtk::Box,
    translation: gtk::Label,
    picture: Option<gtk::Picture>,
}

struct App {
    reader: RefCell<Reader>,
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    paper: gtk::Box,
    paper_scroll: gtk::ScrolledWindow,
    original_scroll: gtk::ScrolledWindow,
    original: gtk::Picture,
    original_loaded: Cell<Option<ImageKey>>,
    status: gtk::Label,
    rate: gtk::Label,
    credit_count: gtk::Label,
    title: gtk::Label,
    mode: gtk::ToggleButton,
    previous: gtk::Button,
    next: gtk::Button,
    page: gtk::Label,
    login: gtk::Button,
    account: AccountPanel,
    updates: UpdateControl,
    translate: gtk::Button,
    models: gtk::DropDown,
    menu_models: RefCell<Vec<(String, String)>>,
    syncing_model: Cell<bool>,
    recent: gtk::MenuButton,
    recent_menu: gtk::Box,
    recent_paths: RefCell<Vec<PathBuf>>,
    link: gtk::LinkButton,
    rows: RefCell<Vec<Option<PaperRow>>>,
    paper_generation: Cell<usize>,
    translation_revision: Cell<u64>,
    pending_position: Cell<Option<ReadingPosition>>,
    dirty_position: Cell<Option<Instant>>,
    chooser: RefCell<Option<gtk::FileChooserNative>>,
}

impl App {
    fn open_window(application: &gtk::Application, path: Option<PathBuf>) {
        let window = gtk::ApplicationWindow::builder().application(application)
            .title("文献阅读器").default_width(1100).default_height(820).build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.set_child(Some(&root));
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        toolbar.add_css_class("lr-toolbar");
        toolbar.append(&brand());
        let open = gtk::Button::with_label("打开 PDF");
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let models = gtk::DropDown::from_strings(&["翻译模型"]);
        models.set_width_request(150);
        let rate = gtk::Label::new(None);
        rate.add_css_class("lr-rate");
        let login = gtk::Button::with_label("登录 CodeBuddy");
        let account = AccountPanel::new();
        toolbar.append(&open);
        toolbar.append(&title);
        toolbar.append(&rate);
        toolbar.append(&models);
        toolbar.append(&login);
        toolbar.append(&account.button);
        root.append(&toolbar);

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        controls.add_css_class("lr-toolbar");
        let recent = gtk::MenuButton::builder().label("最近阅读").build();
        let recent_menu = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let recent_popover = gtk::Popover::new();
        recent_popover.set_child(Some(&recent_menu));
        recent.set_popover(Some(&recent_popover));
        let mode = gtk::ToggleButton::with_label("PDF 原页");
        let previous = gtk::Button::with_label("‹");
        let next = gtk::Button::with_label("›");
        let page = gtk::Label::new(None);
        let translate = gtk::Button::with_label("继续翻译");
        let link = gtk::LinkButton::with_label("https://www.codebuddy.cn/", "复制授权链接");
        link.connect_activate_link(|link| {
            WidgetExt::display(link).clipboard().set_text(&link.uri());
            glib::Propagation::Stop
        });
        controls.append(&recent);
        controls.append(&mode);
        controls.append(&previous);
        controls.append(&page);
        controls.append(&next);
        controls.append(&link);
        controls.append(&translate);
        root.append(&controls);

        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        let introduction = gtk::Label::new(Some("打开 PDF 开始阅读"));
        introduction.add_css_class("lr-welcome");
        stack.add_named(&introduction, Some("welcome"));
        let paper = gtk::Box::new(gtk::Orientation::Vertical, 16);
        paper.add_css_class("lr-paper");
        let paper_scroll = scroller(&paper);
        stack.add_named(&paper_scroll, Some("paper"));
        let original = gtk::Picture::new();
        original.set_can_shrink(true);
        original.set_content_fit(gtk::ContentFit::Contain);
        original.set_halign(gtk::Align::Center);
        original.set_valign(gtk::Align::Start);
        let original_scroll = scroller(&original);
        stack.add_named(&original_scroll, Some("original"));
        root.append(&stack);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.add_css_class("lr-footer");
        let status = gtk::Label::new(None);
        status.set_hexpand(true);
        status.set_xalign(0.0);
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let credit_count = gtk::Label::new(None);
        let updates = UpdateControl::new();
        footer.append(&status);
        footer.append(&credit_count);
        footer.append(&updates.button);
        root.append(&footer);

        install_css(&WidgetExt::display(&window));
        let app = Rc::new(Self {
            reader: RefCell::new(Reader::new()), window, stack, paper, paper_scroll, original_scroll,
            original, original_loaded: Cell::new(None), status, rate, credit_count, title, mode, previous, next, page,
            login, account, updates, translate, models, menu_models: RefCell::new(Vec::new()), syncing_model: Cell::new(false),
            recent, recent_menu, recent_paths: RefCell::new(Vec::new()), link,
            rows: RefCell::new(Vec::new()), paper_generation: Cell::new(usize::MAX), translation_revision: Cell::new(0),
            pending_position: Cell::new(None), dirty_position: Cell::new(None), chooser: RefCell::new(None),
        });
        app.connect(&open);
        if let Some(path) = path { app.open_pdf(path); }
        app.sync_widgets();
        app.window.present();
        app.updates.check();
        glib::timeout_add_local(Duration::from_millis(65), move || {
            if !app.window.is_visible() { return glib::ControlFlow::Break }
            app.poll();
            glib::ControlFlow::Continue
        });
    }

    fn connect(self: &Rc<Self>, open: &gtk::Button) {
        let app = Rc::downgrade(self);
        self.updates.button.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() { app.updates.activate(); }
        });
        let app = Rc::downgrade(self);
        open.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() { app.choose_pdf(); }
        });
        let app = Rc::downgrade(self);
        self.login.connect_clicked(move |_| {
            let Some(app) = app.upgrade() else { return };
            {
                let mut reader = app.reader.borrow_mut();
                if reader.signing_in() { reader.cancel_login(); } else { reader.start_login(); }
            }
            app.sync_widgets();
        });
        let app = Rc::downgrade(self);
        self.account.popover.connect_show(move |_| {
            if let Some(app) = app.upgrade() {
                app.reader.borrow_mut().refresh_account();
                app.sync_widgets();
            }
        });
        let app = Rc::downgrade(self);
        self.account.refresh.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.reader.borrow_mut().refresh_account();
                app.sync_widgets();
            }
        });
        let app = Rc::downgrade(self);
        self.account.login.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.account.button.popdown();
                app.reader.borrow_mut().start_login();
                app.sync_widgets();
            }
        });
        let app = Rc::downgrade(self);
        self.translate.connect_clicked(move |_| {
            let Some(app) = app.upgrade() else { return };
            {
                let mut reader = app.reader.borrow_mut();
                if reader.translation.is_some() { reader.cancel_translation(); } else { reader.start_translation(); }
            }
            app.sync_widgets();
        });
        let app = Rc::downgrade(self);
        self.models.connect_selected_notify(move |dropdown| {
            let Some(app) = app.upgrade() else { return };
            if app.syncing_model.get() { return }
            let id = app.menu_models.borrow().get(dropdown.selected().wrapping_sub(1) as usize)
                .map(|(id, _)| id.clone());
            if let Some(id) = id {
                app.reader.borrow_mut().select_model(&id);
                app.sync_widgets();
            }
        });
        let app = Rc::downgrade(self);
        self.mode.connect_toggled(move |_| {
            if let Some(app) = app.upgrade() {
                app.dirty_position.set(Some(Instant::now()));
                app.sync_widgets();
            }
        });
        for (button, delta) in [(&self.previous, -1isize), (&self.next, 1)] {
            let app = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                let Some(app) = app.upgrade() else { return };
                let page = app.reader.borrow().page.saturating_add_signed(delta);
                app.goto_page(page);
                app.sync_widgets();
            });
        }
        for scroll in [&self.original_scroll, &self.paper_scroll] {
            let app = Rc::downgrade(self);
            scroll.vadjustment().connect_value_changed(move |_| {
                if let Some(app) = app.upgrade() { app.dirty_position.set(Some(Instant::now())); }
            });
        }
        let app = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(app) = app.upgrade() {
                app.save_position();
                let mut reader = app.reader.borrow_mut();
                reader.cancel_login();
                reader.cancel_translation();
            }
            glib::Propagation::Proceed
        });
    }

    fn choose_pdf(self: &Rc<Self>) {
        if self.chooser.borrow().is_some() { return }
        let chooser = gtk::FileChooserNative::new(Some("打开 PDF"), Some(&self.window),
            gtk::FileChooserAction::Open, Some("打开"), Some("取消"));
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("PDF 文档"));
        filter.add_mime_type("application/pdf");
        chooser.add_filter(&filter);
        let app = Rc::downgrade(self);
        chooser.connect_response(move |chooser, response| {
            let Some(app) = app.upgrade() else { return };
            if response == gtk::ResponseType::Accept && let Some(path) = chooser.file().and_then(|file| file.path()) {
                app.open_pdf(path);
            }
            chooser.hide();
            app.chooser.borrow_mut().take();
        });
        chooser.show();
        self.chooser.borrow_mut().replace(chooser);
    }

    fn open_pdf(self: &Rc<Self>, path: PathBuf) {
        self.save_position();
        self.pending_position.set(None);
        self.reader.borrow_mut().open(path);
        self.mode.set_active(false);
        self.paper_scroll.vadjustment().set_value(0.0);
        self.original.set_paintable(None::<&gdk::Paintable>);
        self.sync_widgets();
    }

    fn poll(self: &Rc<Self>) {
        self.updates.poll();
        let (changed, url) = {
            let mut reader = self.reader.borrow_mut();
            let changed = reader.poll();
            (changed, reader.take_login_url())
        };
        if let Some(url) = url.as_deref()
            && gio::AppInfo::launch_default_for_uri(url, None::<&gio::AppLaunchContext>).is_err()
        {
            self.link.set_uri(url);
        }
        if changed || url.is_some() { self.sync_widgets(); }
        self.load_visible_images();
        self.restore_position();
        if self.dirty_position.get().is_some_and(|time| time.elapsed() > Duration::from_millis(700)) {
            self.save_position();
            self.dirty_position.set(None);
        }
    }

    fn sync_widgets(self: &Rc<Self>) {
        let mut reader = self.reader.borrow_mut();
        let count = reader.document.as_ref().map_or(0, |document| document.pages);
        let login = if reader.signing_in() { "取消登录" } else { "登录 CodeBuddy" };
        self.login.set_label(login);
        self.login.set_visible(!reader.signed_in || reader.signing_in());
        self.login.set_sensitive(reader.signing_in() || !reader.checking_models && reader.translation.is_none());
        self.account.sync(&reader);
        self.link.set_visible(reader.signing_in() && self.link.uri() != "https://www.codebuddy.cn/");
        if !reader.signing_in() { self.link.set_uri("https://www.codebuddy.cn/"); }
        self.recent.set_sensitive(!reader.recent_documents.entries().is_empty());
        self.mode.set_sensitive(count > 0);
        self.models.set_sensitive(reader.signed_in && !reader.checking_models && !reader.signing_in());
        self.title.set_text(reader.document.as_ref().map_or("", |doc| doc.title.as_str()));
        self.sync_models(&reader);
        self.sync_recent(&reader);

        let original = self.mode.is_active() && count > 0;
        let page = reader.page;
        self.page.set_text(&format!("{page} / {count}"));
        self.page.set_visible(original);
        self.previous.set_visible(original);
        self.next.set_visible(original);
        self.previous.set_sensitive(page > 1);
        self.next.set_sensitive(page < count);
        self.stack.set_visible_child_name(if count == 0 { "welcome" } else if original { "original" } else { "paper" });

        let (rate, can_translate, action) = match &reader.translation {
            Some(job) if job.stopping() => (String::new(), false, "正在停止"),
            Some(job) => (job.output_rate().map_or_else(|| "等待模型输出…".into(), |rate| format!("≈ {rate:.1} token/s")), true, "停止翻译"),
            None => (String::new(), reader.can_translate() && reader.has_pending_translation(), "继续翻译"),
        };
        self.rate.set_text(&rate);
        self.rate.set_visible(!rate.is_empty());
        self.translate.set_label(action);
        self.translate.set_sensitive(can_translate);
        self.translate.set_visible(count > 0);
        self.status.set_text(&status(&reader));
        self.credit_count.set_text(&credit_label(&reader));
        self.credit_count.set_visible(count > 0);

        let rebuilt = self.paper_generation.replace(reader.generation) != reader.generation
            || (self.rows.borrow().is_empty() && count > 0);
        if rebuilt {
            self.build_paper(&reader);
            self.original_loaded.set(None);
        }
        let revision = reader.translation_layout_reset;
        let reset = self.translation_revision.replace(revision) != revision;
        let changed = reader.take_changed_translations();
        if rebuilt || reset {
            for index in 0..self.rows.borrow().len() { self.update_text(&reader, index); }
        } else {
            for index in changed { self.update_text(&reader, index); }
        }
        if let Some(position) = reader.take_reading_position() {
            self.pending_position.set(Some(position));
        }
    }

    fn sync_models(&self, reader: &Reader) {
        let items: Vec<_> = reader.models.iter().map(|model| {
            let name = model.credit_multiplier.map_or_else(|| model.display_name.clone(),
                |rate| format!("{}  ·  {rate}×", model.display_name));
            (model.id.clone(), name)
        }).collect();
        self.syncing_model.set(true);
        if *self.menu_models.borrow() != items {
            let names: Vec<_> = std::iter::once("翻译模型").chain(items.iter().map(|(_, name)| name.as_str())).collect();
            self.models.set_model(Some(&gtk::StringList::new(&names)));
            self.menu_models.replace(items);
        }
        let index = self.menu_models.borrow().iter().position(|(id, _)| reader.model.as_ref() == Some(id));
        self.models.set_selected(index.map_or(0, |index| index as u32 + 1));
        self.syncing_model.set(false);
    }

    fn sync_recent(self: &Rc<Self>, reader: &Reader) {
        let entries = reader.recent_documents.entries();
        let paths: Vec<_> = entries.iter().map(|entry| entry.path.clone()).collect();
        if *self.recent_paths.borrow() == paths { return }
        self.recent_paths.replace(paths);
        while let Some(child) = self.recent_menu.first_child() { self.recent_menu.remove(&child); }
        for entry in entries {
            let button = gtk::Button::with_label(&entry.title);
            self.recent_menu.append(&button);
            let app = Rc::downgrade(self);
            let path = entry.path.clone();
            button.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.recent.popdown();
                    app.open_pdf(path.clone());
                }
            });
        }
    }

    fn build_paper(self: &Rc<Self>, reader: &Reader) {
        while let Some(child) = self.paper.first_child() { self.paper.remove(&child); }
        self.rows.borrow_mut().clear();
        let Some(document) = &reader.document else { return };
        let paired: HashSet<_> = document.asset_captions.values().copied().collect();
        let mut rows: Vec<_> = (0..document.blocks.len()).map(|_| None).collect();
        for (index, block) in document.blocks.iter().enumerate() {
            if paired.contains(&index) { continue }
            let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 7);
            self.paper.append(&wrapper);
            let media = matches!(block.kind, BlockKind::Figure | BlockKind::Table | BlockKind::Equation);
            let picture = media.then(|| {
                let picture = gtk::Picture::new();
                picture.set_can_shrink(true);
                picture.set_content_fit(gtk::ContentFit::Contain);
                picture.set_alternative_text(Some(&block.text));
                picture.set_halign(gtk::Align::Center);
                picture.set_size_request(-1, if block.kind == BlockKind::Equation { 38 } else { 110 });
                wrapper.append(&picture);
                picture
            });
            if !media {
                wrapper.append(&block_label(block.kind, &block.text));
            }
            let translation = block_label(block.kind, "");
            translation.add_css_class("lr-translation");
            wrapper.append(&translation);
            rows[index] = Some(PaperRow { position: wrapper.clone(), translation, picture });

            if let Some(&caption) = document.asset_captions.get(&index)
                && let Some(caption_block) = document.blocks.get(caption)
            {
                wrapper.append(&block_label(BlockKind::Caption, &caption_block.text));
                let translation = block_label(BlockKind::Caption, "");
                translation.add_css_class("lr-translation");
                wrapper.append(&translation);
                rows[caption] = Some(PaperRow { position: wrapper.clone(), translation, picture: None });
            }
            let references = &reader.citation_links[index];
            let sources = &reader.reference_links[index];
            if references.is_empty() && sources.is_empty() { continue }
            let links = gtk::FlowBox::new();
            links.set_selection_mode(gtk::SelectionMode::None);
            links.set_column_spacing(5);
            for reference in references {
                let button = gtk::Button::with_label(&reference.label);
                button.add_css_class("flat");
                links.insert(&button, -1);
                let target = reference.target;
                let app = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(app) = app.upgrade() { app.scroll_to_block(target, 0.0); }
                });
            }
            for source in sources {
                links.insert(&gtk::LinkButton::with_label(&source.url, &source.label), -1);
            }
            wrapper.append(&links);
        }
        *self.rows.borrow_mut() = rows;
    }

    fn update_text(&self, reader: &Reader, index: usize) {
        if let Some(row) = self.rows.borrow().get(index).and_then(Option::as_ref) {
            let text = reader.displayed_translation(index).map(|(text, _)| text);
            let translatable = reader.document.as_ref().and_then(|doc| doc.blocks.get(index))
                .is_some_and(|block| is_translatable(block.kind));
            row.translation.set_visible(translatable);
            row.translation.set_text(text.unwrap_or("待翻译"));
        }
    }

    fn load_visible_images(&self) {
        let mut reader = self.reader.borrow_mut();
        if reader.document.is_none() { return }
        if self.mode.is_active() {
            let key = ImageKey::Page(reader.page);
            if self.original_loaded.get() == Some(key) { return }
            if let Some(image) = reader.image(key) {
                self.original.set_paintable(Some(&texture(image)));
                self.original_loaded.set(Some(key));
            } else {
                reader.request_image(key);
            }
            return;
        }
        let scroll = self.paper_scroll.vadjustment();
        let visible_top = scroll.value() - 240.0;
        let visible_bottom = scroll.value() + scroll.page_size() + 240.0;
        for (index, row) in self.rows.borrow().iter().enumerate() {
            let Some(row) = row else { continue };
            let Some(picture) = &row.picture else { continue };
            let bounds = row.position.compute_bounds(&self.paper);
            let visible = bounds.is_some_and(|bounds| f64::from(bounds.y() + bounds.height()) >= visible_top
                && f64::from(bounds.y()) <= visible_bottom);
            if !visible {
                if picture.paintable().is_some() {
                    picture.set_size_request(-1, picture.height());
                    picture.set_paintable(None::<&gdk::Paintable>);
                }
                continue;
            }
            if picture.paintable().is_some() { continue }
            let key = ImageKey::Block(index);
            if let Some(image) = reader.image(key) {
                picture.set_size_request(-1, -1);
                picture.set_paintable(Some(&texture(image)));
            } else {
                reader.request_image(key);
            }
        }
    }

    fn goto_page(&self, page: usize) {
        let mut reader = self.reader.borrow_mut();
        let Some(document) = &reader.document else { return };
        reader.page = page.clamp(1, document.pages.max(1));
        self.original_loaded.set(None);
        self.original.set_paintable(None::<&gdk::Paintable>);
        self.original_scroll.vadjustment().set_value(0.0);
        self.dirty_position.set(Some(Instant::now()));
    }

    fn scroll_to_block(&self, index: usize, offset: f64) -> bool {
        let rows = self.rows.borrow();
        let Some(row) = rows.get(index).and_then(Option::as_ref) else { return true };
        let Some(bounds) = row.position.compute_bounds(&self.paper) else { return false };
        if bounds.height() <= 0.0 { return false }
        self.paper_scroll.vadjustment().set_value(f64::from(bounds.y()) + offset);
        true
    }

    fn restore_position(&self) {
        let Some(position) = self.pending_position.get() else { return };
        match position {
            ReadingPosition::Bilingual { block, top, .. } => {
                if self.scroll_to_block(block, f64::from(top)) { self.pending_position.set(None); }
            }
            ReadingPosition::Original { page, offset } => {
                if !self.mode.is_active() {
                    self.goto_page(page);
                    self.mode.set_active(true);
                }
                let key = ImageKey::Page(self.reader.borrow().page);
                if self.original_loaded.get() == Some(key) || self.reader.borrow().image_error(key).is_some() {
                    self.original_scroll.vadjustment().set_value(f64::from(offset));
                    self.pending_position.set(None);
                }
            }
        }
    }

    fn save_position(&self) {
        if self.pending_position.get().is_some() { return }
        let reader = self.reader.borrow();
        let Some(document) = &reader.document else { return };
        if self.mode.is_active() {
            let _ = reader.save_reading_position(ReadingPosition::Original {
                page: reader.page, offset: self.original_scroll.vadjustment().value().round() as i32,
            });
            return;
        }
        let scroll = self.paper_scroll.vadjustment().value();
        let rows = self.rows.borrow();
        for (index, row) in rows.iter().enumerate() {
            let Some(bounds) = row.as_ref().and_then(|row| row.position.compute_bounds(&self.paper)) else { continue };
            if f64::from(bounds.y() + bounds.height()) <= scroll { continue }
            let _ = reader.save_reading_position(ReadingPosition::Bilingual {
                block: index, page: document.blocks[index].page, left: document.blocks[index].bounds.left,
                top: (scroll - f64::from(bounds.y())) as f32, section: Section::Content,
            });
            break;
        }
    }
}

fn brand() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let bytes = glib::Bytes::from_static(include_bytes!("../assets/reader.png"));
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    if let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, 36, 36, true, None::<&gio::Cancellable>) {
        row.append(&gtk::Image::from_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf))));
    }
    let name = gtk::Label::new(Some("文献阅读器"));
    name.add_css_class("lr-brand");
    row.append(&name);
    row
}

fn scroller(widget: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic).child(widget).build()
}

fn block_label(kind: BlockKind, text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_selectable(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(if matches!(kind, BlockKind::Title | BlockKind::Author | BlockKind::Caption) { 0.5 } else { 0.0 });
    label.add_css_class(match kind {
        BlockKind::Title => "lr-title",
        BlockKind::Heading | BlockKind::ReferenceHeading => "lr-heading",
        BlockKind::Author => "lr-author",
        BlockKind::Caption => "lr-caption",
        BlockKind::Reference => "lr-reference",
        _ => "lr-paragraph",
    });
    label
}

fn texture(image: &Raster) -> gdk::MemoryTexture {
    let bytes = glib::Bytes::from_owned(image.bgra.clone());
    gdk::MemoryTexture::new(image.width, image.height, gdk::MemoryFormat::B8g8r8a8,
        &bytes, image.width as usize * 4)
}

fn status(reader: &Reader) -> String {
    if let Some(error) = reader.load_error.as_ref().or(reader.translation_error.as_ref()) {
        return error.clone();
    }
    if reader.loading { return "正在提取 PDF 版面和文字…".into() }
    if reader.signing_in() { return "请在浏览器完成 CodeBuddy 授权".into() }
    if let Some(job) = &reader.translation {
        return format!("正在翻译  ·  {} / {} 段", job.done, job.total);
    }
    if let Some(error) = &reader.models_error { return error.clone(); }
    if reader.checking_models { return "正在读取账号和可用模型…".into() }
    if reader.translated_count > 0 { return format!("已翻译 {} 段", reader.translated_count) }
    if !reader.signed_in { return "登录 CodeBuddy 后可翻译".into() }
    "CodeBuddy 已连接".into()
}

fn credit_label(reader: &Reader) -> String {
    let (credits, incomplete) = reader.credits.summary();
    credits.map_or_else(|| {
        if incomplete { "积分用量不完整".into() } else { "文档积分未记录".into() }
    }, |total| {
        let text = format!("{total:.4}");
        let amount = if total > 0.0 && text == "0.0000" { "<0.0001" } else { text.trim_end_matches('0').trim_end_matches('.') };
        format!("已记录 {amount} 积分{}", if incomplete { "（部分）" } else { "" })
    })
}

fn install_css(display: &gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(r#"
        .lr-toolbar { padding: 8px 15px; border-bottom: 1px solid alpha(#186856, .16); }
        .lr-brand { font-weight: 700; font-size: 16px; }
        .lr-footer { padding: 5px 15px; border-top: 1px solid alpha(#186856, .16); }
        .lr-paper { padding: 28px 60px 44px; }
        .lr-title { font-family: serif; font-size: 30px; font-weight: 700; }
        .lr-title.lr-translation { font-size: 20px; }
        .lr-heading { font-size: 20px; font-weight: 700; }
        .lr-author, .lr-caption, .lr-reference { font-size: 14px; }
        .lr-paragraph { font-family: serif; font-size: 17px; }
        .lr-translation { opacity: .79; }
        .lr-rate { border-radius: 13px; padding: 2px 9px; background: alpha(#28a687, .18); }
        .lr-welcome { font-size: 22px; font-weight: 600; }
        .lr-account { padding: 10px; }
        .lr-account-name { font-size: 18px; font-weight: 700; }
        .lr-account-summary { background: alpha(#28a687, .14); border-radius: 11px; padding: 12px 14px; }
        .lr-account-credits { font-size: 25px; font-weight: 700; }
        .lr-account-hint { font-size: 12px; }
    "#);
    gtk::style_context_add_provider_for_display(display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}
