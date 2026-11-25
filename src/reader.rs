use crate::citations::{self, CitationLink, SourceLink};
use crate::document_credits::{self, DocumentCredits, TurnCredits};
use crate::model_settings;
use crate::pdf::{self, BlockKind, Document};
use crate::reading_position::{self, ReadingPosition};
use crate::recent_documents::RecentDocuments;
use crate::translation::{Account, Model, TranslatedText, TranslationStyle, TranslationUpdate, Translator};
use crate::translation_cache::{self, Cache, PdfId};
use base64::Engine as _;
use std::collections::{HashMap, HashSet};
use std::io::{self, Cursor};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const IMAGE_BUDGET: usize = 48 * 1024 * 1024;
const OUTPUT_RATE_STALE_AFTER: Duration = Duration::from_secs(3);
const ACCOUNT_REFRESH_INTERVAL: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageKey {
    Page(usize),
    Block(usize),
}

pub struct Raster {
    pub width: i32,
    pub height: i32,
    pub bgra: Vec<u8>,
}

impl Raster {
    fn decode_avatar(encoded: &str) -> Option<Self> {
        const EDGE: u32 = 96;
        const MAX_BYTES: usize = 256 * 1024;
        if encoded.len() > MAX_BYTES.div_ceil(3) * 4 {
            return None;
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
        if !matches!(reader.format(), Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif | image::ImageFormat::WebP)) {
            return None;
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let thumbnail = reader.decode().ok()?.resize_to_fill(EDGE, EDGE, image::imageops::FilterType::Triangle);
        let mut bgra = thumbnail.into_rgba8().into_raw();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        Some(Self { width: EDGE as i32, height: EDGE as i32, bgra })
    }

    fn decode(ppm: &[u8]) -> Result<Self, String> {
        fn decode_ppm(ppm: &[u8]) -> Option<Raster> {
            let mut pixels = ppm;
            let mut read_token = || {
                while let Some(&byte) = pixels.first() {
                    if byte.is_ascii_whitespace() {
                        pixels = &pixels[1..];
                    } else if byte == b'#' {
                        pixels = &pixels[pixels.iter().position(|byte| *byte == b'\n')? + 1..];
                    } else {
                        break;
                    }
                }
                let end = pixels.iter().position(u8::is_ascii_whitespace)?;
                let (token, tail) = pixels.split_at(end);
                pixels = tail;
                std::str::from_utf8(token).ok()
            };
            if read_token()? != "P6" {
                return None;
            }
            let width = read_token()?.parse::<i32>().ok()?;
            let height = read_token()?.parse::<i32>().ok()?;
            if read_token()? != "255" || width <= 0 || height <= 0 {
                return None;
            }
            let pixel_count = (width as usize).checked_mul(height as usize)?;
            let pixels = pixels.strip_prefix(b"\r\n").unwrap_or(&pixels[1..]);
            if pixels.len() != pixel_count.checked_mul(3)? {
                return None;
            }
            let mut bgra = Vec::with_capacity(pixel_count.checked_mul(4)?);
            for rgb in pixels.chunks_exact(3) {
                bgra.extend_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
            }
            Some(Raster { width, height, bgra })
        }

        decode_ppm(ppm).ok_or_else(|| "无法读取 Poppler 生成的页面图像".to_owned())
    }
}

struct RenderJob {
    generation: usize,
    key: ImageKey,
    path: PathBuf,
    document: Arc<Document>,
}

struct PreparedDocument {
    document: Document,
    pdf_id: Option<PdfId>,
    reading_position: Option<ReadingPosition>,
    citation_links: Vec<Vec<CitationLink>>,
    reference_links: Vec<Vec<SourceLink>>,
}

impl PreparedDocument {
    fn open(path: &std::path::Path) -> Result<Self, String> {
        let (document, pdf_id) = thread::scope(|scope| {
            let id = scope.spawn(|| translation_cache::pdf_id(path).ok());
            (pdf::extract(path), id.join().ok().flatten())
        });
        let document = document?;
        let citation_links = citations::build(&document);
        let reference_links = document.blocks.iter().map(|block| {
            if block.kind == BlockKind::Reference {
                citations::source_links(&block.text)
            } else {
                Vec::new()
            }
        }).collect();
        let reading_position = pdf_id.and_then(reading_position::load);
        Ok(Self { document, pdf_id, reading_position, citation_links, reference_links })
    }
}

type TranslationRows = Vec<(usize, TranslatedText)>;

enum Event {
    Opened(usize, Result<PreparedDocument, String>),
    Rendered(usize, ImageKey, Result<Raster, String>),
    Models(Result<Option<Vec<Model>>, String>),
    SessionVerified(Option<Account>),
    AccountLoaded(u64, Result<(Account, Option<Raster>), String>),
    LoginUrl(String),
    LoginEnded(Result<Option<Account>, String>),
    CacheLoaded(u64, io::Result<(Cache, TranslationRows)>),
    CacheSaved(u64),
    CreditsLoaded(usize, DocumentCredits),
    CreditsRecorded { pdf_id: PdfId, key: Option<String>, usage: TurnCredits },
    TranslationRate { revision: u64, rate: f64 },
    TranslationPreview { revision: u64, rows: TranslationRows },
    Translated { revision: u64, rows: TranslationRows, saved: bool },
    TranslationEnded(usize, Result<(), String>),
}

enum CacheState {
    Pending,
    Loading,
    Ready(Cache),
    Failed,
}

pub struct ActiveTranslation {
    pub done: usize,
    pub total: usize,
    generation: usize,
    cancel: Arc<AtomicBool>,
    output_rate: Option<(f64, Instant)>,
}

impl ActiveTranslation {
    pub fn stopping(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn output_rate(&self) -> Option<f64> {
        self.output_rate.map(|(rate, _)| rate).filter(|_| !self.stopping())
    }

    fn update_rate(&mut self, rate: Option<f64>) -> bool {
        let rate = rate.map(|rate| (rate * 10.0).round() / 10.0);
        let previous = self.output_rate.map(|(rate, _)| rate);
        self.output_rate = rate.map(|rate| (rate, Instant::now()));
        previous != rate
    }
}

pub struct Reader {
    pub path: Option<PathBuf>,
    pub document: Option<Arc<Document>>,
    pub recent_documents: RecentDocuments,
    pub citation_links: Vec<Vec<CitationLink>>,
    pub reference_links: Vec<Vec<SourceLink>>,
    pub generation: usize,
    pub page: usize,
    pub loading: bool,
    pub load_error: Option<String>,
    pub models: Vec<Model>,
    pub model: Option<String>,
    pub model_save_failed: bool,
    pub models_error: Option<String>,
    pub checking_models: bool,
    pub signed_in: bool,
    pub account: Option<Account>,
    pub account_avatar: Option<Raster>,
    pub checking_account: bool,
    pub account_error: Option<String>,
    account_generation: u64,
    account_requested_at: Option<Instant>,
    pub translations: Vec<Option<String>>,
    translation_math: Vec<Vec<Range<usize>>>,
    pub translated_count: usize,
    pub translation_layout_reset: u64,
    pub translation_model: Option<String>,
    pub translation: Option<ActiveTranslation>,
    pub translation_error: Option<String>,
    pub credits: DocumentCredits,
    preferred_model: Option<String>,
    pdf_id: Option<PdfId>,
    reading_position: Option<ReadingPosition>,
    translation_cache: CacheState,
    unsaved_translations: TranslationRows,
    translation_requested: bool,
    translation_total: usize,
    translation_preview: HashMap<usize, TranslatedText>,
    changed_translations: Vec<usize>,
    login_cancel: Option<Arc<AtomicBool>>,
    login_url: Option<String>,
    images: HashMap<ImageKey, (u64, Raster)>,
    image_bytes: usize,
    image_use: u64,
    failed_images: HashMap<ImageKey, String>,
    requested_images: HashSet<ImageKey>,
    render_jobs: mpsc::Sender<RenderJob>,
    send: mpsc::Sender<Event>,
    events: mpsc::Receiver<Event>,
}

impl Reader {
    pub fn new() -> Self {
        let (send, events) = mpsc::channel();
        let (render_jobs, jobs) = mpsc::channel::<RenderJob>();
        let render_events = send.clone();
        thread::spawn(move || {
            for job in jobs {
                let raster = match job.key {
                    ImageKey::Page(page) => pdf::render_page(&job.path, page, 135),
                    ImageKey::Block(index) => pdf::render_block(&job.path, &job.document.blocks[index], 155),
                };
                let result = raster.and_then(|bytes| Raster::decode(&bytes));
                if render_events.send(Event::Rendered(job.generation, job.key, result)).is_err() {
                    break;
                }
            }
        });
        let mut reader = Self {
            path: None, document: None, generation: 0, page: 1, loading: false, load_error: None,
            recent_documents: RecentDocuments::load(),
            citation_links: Vec::new(), reference_links: Vec::new(),
            models: Vec::new(), model: None, models_error: None, checking_models: false, signed_in: false, account: None,
            account_avatar: None, checking_account: false, account_error: None,
            account_generation: 0, account_requested_at: None,
            preferred_model: model_settings::load_model(), model_save_failed: false,
            pdf_id: None, reading_position: None,
            translation_cache: CacheState::Pending, unsaved_translations: Vec::new(),
            translations: Vec::new(), translation_math: Vec::new(), translated_count: 0, translation_layout_reset: 0,
            translation_model: None, translation: None, translation_requested: false,
            translation_total: 0, translation_preview: HashMap::new(), changed_translations: Vec::new(),
            translation_error: None, login_cancel: None, login_url: None,
            credits: DocumentCredits::default(),
            images: HashMap::new(), image_bytes: 0,
            image_use: 0, failed_images: HashMap::new(), requested_images: HashSet::new(),
            render_jobs, send, events,
        };
        reader.refresh_models();
        reader
    }

    pub fn open(&mut self, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        self.stop_active_translation();
        self.generation += 1;
        self.path = Some(path.clone());
        self.document = None;
        self.pdf_id = None;
        self.credits = DocumentCredits::default();
        self.reading_position = None;
        self.citation_links.clear();
        self.reference_links.clear();
        self.page = 1;
        self.loading = true;
        self.load_error = None;
        self.translations.clear();
        self.translation_math.clear();
        self.reset_translations();
        self.translation_total = 0;
        self.translation_requested = self.preferred_model.is_some();
        self.translation_error = None;
        self.images.clear();
        self.image_bytes = 0;
        self.requested_images.clear();
        self.failed_images.clear();
        let generation = self.generation;
        let send = self.send.clone();
        thread::spawn(move || {
            let _ = send.send(Event::Opened(generation, PreparedDocument::open(&path)));
        });
    }

    pub fn take_reading_position(&mut self) -> Option<ReadingPosition> {
        self.reading_position.take()
    }

    pub fn save_reading_position(&self, position: ReadingPosition) -> bool {
        self.pdf_id.is_some_and(|id| reading_position::save(id, position).is_ok())
    }

    pub fn refresh_models(&mut self) {
        self.load_models(true);
    }

    pub fn refresh_account(&mut self) {
        if !self.signed_in || self.signing_in() || self.checking_account
            || self.account_requested_at.is_some_and(|at| at.elapsed() < ACCOUNT_REFRESH_INTERVAL)
        {
            return;
        }
        self.checking_account = true;
        self.account_requested_at = Some(Instant::now());
        self.account_error = None;
        let generation = self.account_generation;
        let send = self.send.clone();
        thread::spawn(move || {
            let result = Translator::account().and_then(|result| {
                let mut account = result.ok_or_else(|| "账户资料暂不可用，请重新确认 CodeBuddy 登录。".to_owned())?;
                let avatar = account.avatar_image.take().as_deref().and_then(Raster::decode_avatar);
                account.remaining_credits = account.remaining_credits.filter(|amount| amount.is_finite() && *amount >= 0.0);
                Ok((account, avatar))
            });
            let _ = send.send(Event::AccountLoaded(generation, result));
        });
    }

    fn reset_account(&mut self, account: Option<Account>) {
        self.account_generation = self.account_generation.wrapping_add(1);
        self.account = account;
        self.account_avatar = None;
        self.account_requested_at = None;
        self.checking_account = false;
        self.account_error = None;
    }

    fn load_models(&mut self, check_login: bool) {
        if self.checking_models || self.signing_in() {
            return;
        }
        self.checking_models = true;
        self.models_error = None;
        let send = self.send.clone();
        thread::spawn(move || {
            let result = Translator::discover_models(check_login, |account| {
                let _ = send.send(Event::SessionVerified(account));
            });
            let _ = send.send(Event::Models(result));
        });
    }

    pub fn model_name(&self, id: &str) -> Option<&str> {
        self.models.iter().find(|model| model.id == id).map(|model| model.display_name.as_str())
    }

    pub fn can_translate(&self) -> bool {
        self.signed_in && self.model.is_some() && self.models_error.is_none()
            && !self.checking_models && !self.signing_in()
    }

    pub fn has_pending_translation(&self) -> bool {
        self.translated_count < self.translation_total || !self.unsaved_translations.is_empty()
    }

    pub fn take_changed_translations(&mut self) -> Vec<usize> {
        let mut changed = std::mem::take(&mut self.changed_translations);
        changed.sort_unstable();
        changed.dedup();
        changed
    }

    pub fn displayed_translation(&self, index: usize) -> Option<(&str, &[Range<usize>])> {
        if let Some(text) = self.translations.get(index)?.as_deref() {
            return Some((text, &self.translation_math[index]));
        }
        self.translation_preview.get(&index).map(|row| (row.text.as_str(), row.formulas.as_slice()))
    }

    pub fn select_model(&mut self, id: &str) {
        if !self.signed_in || self.model_name(id).is_none() || self.signing_in() {
            return;
        }
        if self.preferred_model.as_deref() != Some(id) || self.model_save_failed {
            self.model_save_failed = model_settings::save_model(id).is_err();
            self.preferred_model = Some(id.to_owned());
        }
        self.set_model(Some(id.to_owned()));
        self.start_translation();
    }

    fn set_model(&mut self, model: Option<String>) {
        if self.model == model {
            return;
        }
        self.stop_active_translation();
        self.model = model;
    }

    pub fn signing_in(&self) -> bool {
        self.login_cancel.is_some()
    }

    pub fn start_login(&mut self) {
        if self.signing_in() || self.checking_models || self.translation.is_some() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.login_cancel = Some(cancel.clone());
        self.models_error = None;
        self.translation_error = None;
        let send = self.send.clone();
        thread::spawn(move || {
            let result = Translator::login(&cancel, |url| {
                let _ = send.send(Event::LoginUrl(url));
            });
            let _ = send.send(Event::LoginEnded(result));
        });
    }

    pub fn cancel_login(&mut self) {
        if let Some(cancel) = &self.login_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        self.login_url = None;
    }

    pub fn take_login_url(&mut self) -> Option<String> {
        self.login_url.take()
    }

    pub fn start_translation(&mut self) {
        if self.translation.as_ref().is_some_and(|job| !job.stopping()) {
            return;
        }
        self.translation_requested = true;
        if matches!(self.translation_cache, CacheState::Failed) {
            self.translation_cache = CacheState::Pending;
        }
        self.start_requested_translation();
    }

    fn load_translation_cache(&mut self) {
        let (Some(document), Some(preferred)) = (&self.document, &self.preferred_model) else { return };
        let switched = self.translation_model.as_ref() != Some(preferred);
        if !switched && !matches!(self.translation_cache, CacheState::Pending) {
            return;
        }
        let preferred = preferred.clone();
        let document = Arc::clone(document);
        if switched {
            self.reset_translations();
            self.translation_model = Some(preferred.clone());
            self.translation_error = None;
        }
        // A stopped worker can still be committing its final completed batch.
        if self.translation.is_some() {
            return;
        }
        let Some(pdf_id) = self.pdf_id else {
            self.translation_cache = CacheState::Failed;
            self.translation_error = Some("无法读取 PDF 内容以恢复译文缓存，请重新打开文件。".into());
            return;
        };
        let revision = self.translation_layout_reset;
        let send = self.send.clone();
        self.translation_error = None;
        self.translation_cache = CacheState::Loading;
        thread::spawn(move || {
            let result = Cache::new(pdf_id, &preferred).and_then(|cache| {
                let rows = cache.load(document.blocks.len(), |index| translation_source(&document, index))?;
                Ok((cache, rows))
            });
            let _ = send.send(Event::CacheLoaded(revision, result));
        });
    }

    fn start_requested_translation(&mut self) {
        self.load_translation_cache();
        if !self.translation_requested || self.translation.is_some() || !self.can_translate() {
            return;
        }
        let (Some(document), Some(model_id), Some(pdf_id)) = (&self.document, &self.model, self.pdf_id) else {
            return;
        };
        let Some(model) = self.models.iter().find(|model| &model.id == model_id).cloned() else { return };
        let CacheState::Ready(cache) = &self.translation_cache else { return };
        self.translation_requested = false;
        self.translation_error = None;
        if !self.has_pending_translation() {
            return;
        }
        let input: Vec<_> = (0..document.blocks.len())
            .filter(|&index| self.translations[index].is_none() && translation_source(document, index).is_some())
            .collect();
        let unsaved = self.unsaved_translations.clone();
        let cache = cache.clone();
        let document = Arc::clone(document);
        let cancel = Arc::new(AtomicBool::new(false));
        let generation = self.generation;
        let revision = self.translation_layout_reset;
        self.translation = Some(ActiveTranslation {
            done: self.translated_count, total: self.translation_total, generation, cancel: cancel.clone(),
            output_rate: None,
        });
        let send = self.send.clone();
        thread::spawn(move || {
            let result = (|| {
                if !unsaved.is_empty() {
                    save_translations(&cache, &document, &unsaved)?;
                    let _ = send.send(Event::CacheSaved(revision));
                }
                translate(revision, pdf_id, &model, &document, &input, &cache, &cancel, &send)
            })();
            let _ = send.send(Event::TranslationEnded(generation, result));
        });
    }

    pub fn cancel_translation(&mut self) {
        self.translation_requested = false;
        self.stop_active_translation();
    }

    fn stop_active_translation(&mut self) {
        if let Some(job) = &self.translation {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.update_translation_preview(Vec::new());
    }

    fn reset_translations(&mut self) {
        self.translations.fill(None);
        self.translation_math.iter_mut().for_each(Vec::clear);
        self.translation_model = None;
        self.translation_cache = CacheState::Pending;
        self.unsaved_translations.clear();
        self.translated_count = 0;
        self.translation_preview.clear();
        self.changed_translations.clear();
        self.translation_layout_reset += 1;
    }

    pub fn request_image(&mut self, key: ImageKey) {
        self.image_use += 1;
        if let Some((last_used, _)) = self.images.get_mut(&key) {
            *last_used = self.image_use;
            return;
        }
        if self.requested_images.len() >= 2 || self.requested_images.contains(&key) || self.failed_images.contains_key(&key) {
            return;
        }
        if let (Some(path), Some(document)) = (&self.path, &self.document) {
            let job = RenderJob { generation: self.generation, key, path: path.clone(), document: document.clone() };
            if self.render_jobs.send(job).is_ok() {
                self.requested_images.insert(key);
            }
        }
    }

    pub fn image(&self, key: ImageKey) -> Option<&Raster> {
        self.images.get(&key).map(|(_, image)| image)
    }

    pub fn image_error(&self, key: ImageKey) -> Option<&str> {
        self.failed_images.get(&key).map(String::as_str)
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Opened(generation, result) if generation == self.generation => {
                    self.loading = false;
                    match result {
                        Ok(PreparedDocument { document, pdf_id, reading_position, citation_links, reference_links }) => {
                            if let (Some(id), Some(path)) = (pdf_id, &self.path) {
                                let _ = self.recent_documents.record(id, path, &document.title);
                            }
                            self.pdf_id = pdf_id;
                            if let Some(pdf_id) = pdf_id {
                                let send = self.send.clone();
                                thread::spawn(move || {
                                    let _ = send.send(Event::CreditsLoaded(generation, document_credits::load(pdf_id)));
                                });
                            }
                            self.reading_position = reading_position;
                            self.translations.resize(document.blocks.len(), None);
                            self.translation_math.resize_with(document.blocks.len(), Vec::new);
                            self.translation_total = (0..document.blocks.len())
                                .filter(|&index| translation_source(&document, index).is_some()).count();
                            self.document = Some(Arc::new(document));
                            self.citation_links = citation_links;
                            self.reference_links = reference_links;
                        }
                        Err(error) => self.load_error = Some(error),
                    }
                }
                Event::Rendered(generation, key, result) if generation == self.generation => {
                    self.requested_images.remove(&key);
                    match result {
                        Ok(image) => self.cache_image(key, image),
                        Err(error) => { self.failed_images.insert(key, error); }
                    }
                }
                Event::SessionVerified(account) => {
                    self.signed_in = true;
                    self.reset_account(account);
                    self.refresh_account();
                }
                Event::AccountLoaded(generation, result) if generation == self.account_generation => {
                    self.checking_account = false;
                    match result {
                        Ok((mut account, avatar)) => {
                            self.account_error = account.balance_error.take();
                            if let Some(previous) = self.account.take() {
                                account.display_name = account.display_name.or(previous.display_name);
                                if self.account_error.is_some() {
                                    account.remaining_credits = account.remaining_credits.or(previous.remaining_credits);
                                    account.plan = account.plan.or(previous.plan);
                                }
                            }
                            self.account = Some(account);
                            if let Some(avatar) = avatar { self.account_avatar = Some(avatar); }
                        }
                        Err(error) => self.account_error = Some(error),
                    }
                }
                Event::Models(result) => {
                    self.checking_models = false;
                    match result {
                        Ok(Some(models)) => {
                            self.signed_in = true;
                            self.models = models;
                            let selected = self.preferred_model.as_deref()
                                .filter(|id| self.model_name(id).is_some())
                                .map(str::to_owned);
                            self.set_model(selected);
                        }
                        Ok(None) => {
                            self.signed_in = false;
                            self.reset_account(None);
                            self.models.clear();
                            self.set_model(None);
                        }
                        Err(error) => self.models_error = Some(error),
                    }
                }
                Event::LoginUrl(url) => {
                    if self.login_cancel.as_ref().is_some_and(|cancel| !cancel.load(Ordering::Relaxed)) {
                        self.login_url = Some(url);
                    }
                }
                Event::LoginEnded(result) => {
                    let cancelled = self.login_cancel.take().is_some_and(|cancel| cancel.load(Ordering::Relaxed));
                    if !cancelled {
                        match result {
                            Ok(account) => {
                                self.signed_in = true;
                                self.reset_account(account);
                                self.load_models(false);
                                self.refresh_account();
                            }
                            Err(error) => self.models_error = Some(error),
                        }
                    }
                }
                Event::CacheLoaded(revision, result) if revision == self.translation_layout_reset => {
                    match result {
                        Ok((cache, rows)) => {
                            self.translation_cache = CacheState::Ready(cache);
                            self.apply_translations(rows);
                        }
                        Err(_) => {
                            self.translation_cache = CacheState::Failed;
                            self.translation_error = Some("无法读取译文缓存。点「重试翻译」再次读取。".into());
                        }
                    }
                }
                Event::CacheSaved(revision) if revision == self.translation_layout_reset => {
                    self.unsaved_translations.clear();
                }
                Event::CreditsLoaded(generation, history) if generation == self.generation => {
                    self.credits.merge(history);
                }
                Event::CreditsRecorded { pdf_id, key, usage } if Some(pdf_id) == self.pdf_id => {
                    self.credits.record(key, usage);
                }
                Event::TranslationRate { revision, rate } if revision == self.translation_layout_reset => {
                    if let Some(job) = self.translation.as_mut().filter(|job| !job.stopping()) {
                        changed |= job.update_rate(Some(rate));
                    }
                    continue;
                }
                Event::TranslationPreview { revision, rows } if revision == self.translation_layout_reset => {
                    if self.translation.as_ref().is_some_and(|job| !job.stopping()) {
                        changed |= self.update_translation_preview(rows);
                    }
                    continue;
                }
                Event::Translated { revision, rows, saved } if revision == self.translation_layout_reset => {
                    if let Some(job) = self.translation.as_mut() {
                        job.done += rows.len();
                    }
                    if !saved {
                        self.unsaved_translations = rows.clone();
                    }
                    self.apply_translations(rows);
                }
                Event::TranslationEnded(generation, result) => {
                    if self.translation.as_ref().is_some_and(|job| job.generation == generation) {
                        let stopped = self.translation.take().is_some_and(|job| job.stopping());
                        self.update_translation_preview(Vec::new());
                        if generation == self.generation && !stopped {
                            self.translation_error = result.err();
                            self.refresh_account();
                        }
                    }
                }
                _ => continue,
            }
            changed = true;
        }
        if let Some(job) = self.translation.as_mut()
            && job.output_rate.is_some_and(|(_, updated)| updated.elapsed() >= OUTPUT_RATE_STALE_AFTER)
        {
            changed |= job.update_rate(None);
        }
        if changed {
            self.start_requested_translation();
        }
        changed
    }

    fn update_translation_preview(&mut self, rows: TranslationRows) -> bool {
        let preview: HashMap<_, _> = rows.into_iter()
            .filter(|(index, _)| self.translations[*index].is_none())
            .collect();
        let before = self.changed_translations.len();
        self.changed_translations.extend(self.translation_preview.keys()
            .filter(|index| !preview.contains_key(index)).copied());
        self.changed_translations.extend(preview.iter().filter_map(|(&index, row)| {
            (self.translation_preview.get(&index) != Some(row)).then_some(index)
        }));
        self.translation_preview = preview;
        self.changed_translations.len() != before
    }

    fn apply_translations(&mut self, rows: TranslationRows) {
        for (index, TranslatedText { text, formulas }) in rows {
            self.translation_preview.remove(&index);
            if self.translations[index].replace(text).is_none() {
                self.translated_count += 1;
            }
            self.translation_math[index] = formulas;
            self.changed_translations.push(index);
        }
    }

    fn cache_image(&mut self, key: ImageKey, image: Raster) {
        while self.image_bytes + image.bgra.len() > IMAGE_BUDGET {
            let Some((&oldest, _)) = self.images.iter().min_by_key(|(_, (access, _))| access) else {
                break;
            };
            if let Some((_, removed)) = self.images.remove(&oldest) {
                self.image_bytes -= removed.bgra.len();
            }
        }
        self.image_bytes += image.bgra.len();
        self.image_use += 1;
        self.images.insert(key, (self.image_use, image));
    }
}

pub fn is_translatable(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Title | BlockKind::Heading | BlockKind::Paragraph | BlockKind::Caption | BlockKind::Reference)
}

fn translation_source(document: &Document, index: usize) -> Option<(TranslationStyle, &str)> {
    let block = document.blocks.get(index)?;
    if !is_translatable(block.kind) {
        return None;
    }
    let source = document.native_inline.get(&index).map_or(block.text.as_str(), String::as_str);
    (!source.trim().is_empty()).then_some((translation_style(block.kind), source))
}

fn translation_style(kind: BlockKind) -> TranslationStyle {
    if kind == BlockKind::Reference { TranslationStyle::Bibliography } else { TranslationStyle::Prose }
}

fn save_translations(cache: &Cache, document: &Document, rows: &[(usize, TranslatedText)]) -> Result<(), String> {
    cache.save(rows, |index| translation_source(document, index))
        .map_err(|_| "无法保存译文缓存，翻译已暂停。".to_owned())
}

fn translate(
    revision: u64,
    pdf_id: PdfId,
    model: &Model,
    document: &Document,
    input: &[usize],
    cache: &Cache,
    cancel: &AtomicBool,
    send: &mpsc::Sender<Event>,
) -> Result<(), String> {
    let Some((first, remaining)) = input.split_at_checked(1) else {
        return Ok(());
    };
    let mut translator = Translator::new(model, cancel, |rate| {
        let _ = send.send(Event::TranslationRate { revision, rate });
    })?;
    let source = |index| translation_source(document, index);
    let style = |index: usize| translation_style(document.blocks[index].kind);
    let sections = remaining.chunk_by(|&left, &right| style(left) == style(right));
    let batches = std::iter::once(first).chain(sections);
    let mut skipped = 0;
    for batch in batches {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let paragraphs: Vec<_> = batch.iter().map(|&index| source(index).expect("translatable text").1).collect();
        let block_rows = |rows: TranslationRows| -> TranslationRows {
            rows.into_iter().map(|(offset, text)| (batch[offset], text)).collect()
        };
        let result = translator.translate_batch(&paragraphs, style(batch[0]), cancel, |update| {
            let (event, result) = match update {
                TranslationUpdate::Preview(rows) => {
                    (Event::TranslationPreview { revision, rows: block_rows(rows) }, Ok(()))
                }
                TranslationUpdate::Ready(rows) => {
                    let rows = block_rows(rows);
                    let saved = save_translations(cache, document, &rows);
                    (Event::Translated { revision, rows, saved: saved.is_ok() }, saved)
                }
                TranslationUpdate::Credits(usage) => {
                    let key = document_credits::save(pdf_id, usage).ok();
                    (Event::CreditsRecorded { pdf_id, key, usage }, Ok(()))
                }
            };
            if send.send(event).is_err() {
                cancel.store(true, Ordering::Relaxed);
            }
            result
        });
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        skipped += result?;
    }
    if skipped != 0 && !cancel.load(Ordering::Relaxed) {
        return Err(format!("有 {skipped} 个段落未返回可用译文，已保留原文。"));
    }
    Ok(())
}
