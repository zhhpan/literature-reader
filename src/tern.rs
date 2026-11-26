//! Headless backend for the Tern plugin: `--tern` commands run without a window or UI.

use crate::math;
use crate::model_settings;
use crate::pdf::{self, BlockKind};
use crate::reader::{Raster, Reader, TranslateScope};
use crate::recent_documents::RecentDocuments;
use crate::translation::{Account, Model, Translator};
use image::ImageEncoder as _;
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(750);
const SNAPSHOT_CHANGE_INTERVAL: Duration = Duration::from_millis(200);
const CREDITS_WAIT: Duration = Duration::from_millis(400);

pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(2).collect();
    match args.as_slice() {
        [command, out] if command == "list" => list(Path::new(out)),
        [command, pdf, out] if command == "state" => state(pdf, Path::new(out)),
        [command, pdf, out, model] if command == "translate" => translate(pdf, Path::new(out), model),
        [command, out] if command == "models" => models(Path::new(out)),
        [command, out] if command == "login" => login(Path::new(out)),
        [command, pdf, kind, value, out] if command == "image" => image(pdf, kind, value, Path::new(out)),
        _ => usage(),
    }
}

fn usage() -> i32 {
    eprintln!(
        "用法：literature-reader --tern list <out.json>｜state <pdf> <out.json>｜translate <pdf> <out.json> <model-id>｜models <out.json>｜login <out.json>｜image <pdf> page|block <n> <out.png>"
    );
    2
}

fn list(out: &Path) -> i32 {
    let recent = RecentDocuments::load();
    let documents: Vec<Value> = recent.entries().iter().map(|entry| json!({
        "title": entry.title,
        "path": entry.path.to_string_lossy().into_owned(),
    })).collect();
    write_value(out, json!({"schema": 1, "error": null, "documents": documents}))
}

fn state(pdf: &str, out: &Path) -> i32 {
    let mut reader = Reader::new_headless(false);
    reader.open(PathBuf::from(pdf));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        reader.poll();
        if let Some(error) = &reader.load_error {
            return fail(out, error.clone());
        }
        if reader.document.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            return fail(out, "读取 PDF 超时");
        }
        std::thread::sleep(POLL);
    }
    wait_for_credits(&mut reader);
    if reader.preferred_model().is_some() {
        reader.load_cached_translations();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            reader.poll();
            if reader.cache_ready() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(POLL);
        }
    }
    wait_for_credits(&mut reader);
    let model_id = reader.preferred_model().map(str::to_owned);
    write_value(out, snapshot(&reader, model_id.as_deref(), None))
}

/// Credits load on their own thread; a short wait keeps the snapshot from missing them.
fn wait_for_credits(reader: &mut Reader) {
    if reader.credits_loaded() {
        return;
    }
    let deadline = Instant::now() + CREDITS_WAIT;
    while !reader.credits_loaded() && Instant::now() < deadline {
        reader.poll();
        if !reader.credits_loaded() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn translate(pdf: &str, out: &Path, model_id: &str) -> i32 {
    let lock_path = sibling(out, ".lock");
    match OpenOptions::new().write(true).create_new(true).open(&lock_path) {
        Ok(mut file) => {
            let _ = write!(file, "{{\"pid\":{}}}", std::process::id());
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return fail(out, "已有翻译任务在运行");
        }
        Err(error) => return fail(out, format!("无法创建翻译锁：{error}")),
    }
    let _lock = Lock(lock_path);
    let cancel_path = sibling(out, ".cancel");

    let mut reader = Reader::new_headless(true);
    reader.open(PathBuf::from(pdf));
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        reader.poll();
        if consume_cancel(&cancel_path) {
            // Cancelled before any translation ran: keep the previous snapshot instead of blanking it.
            return 0;
        }
        if let Some(error) = &reader.load_error {
            return fail(out, error.clone());
        }
        if reader.document.is_some() && reader.models_settled() {
            break;
        }
        if Instant::now() >= deadline {
            return fail(out, "连接 CodeBuddy 超时");
        }
        std::thread::sleep(POLL);
    }
    if reader.models_error.is_some() || !reader.signed_in {
        return fail(out, "请先在文献阅读器中登录 CodeBuddy");
    }
    if !reader.models.iter().any(|model| model.id == model_id) {
        return fail(out, "所选模型不可用");
    }
    if consume_cancel(&cancel_path) {
        // Cancelled just before the model was selected; nothing has been written yet.
        return 0;
    }
    reader.select_model(model_id);
    reader.start_translation(TranslateScope::All);

    let mut last_write: Option<Instant> = None;
    loop {
        let changed = reader.poll();
        let now = Instant::now();
        let write = last_write.is_none_or(|last| {
            let elapsed = now.duration_since(last);
            (changed && elapsed >= SNAPSHOT_CHANGE_INTERVAL) || elapsed >= SNAPSHOT_INTERVAL
        });
        if write {
            if let Err(error) = write_json(out, &snapshot(&reader, Some(model_id), None)) {
                eprintln!("{error}");
                return 1;
            }
            last_write = Some(now);
        }
        if cancel_path.exists() {
            return stop_translation(&mut reader, out, &cancel_path, model_id);
        }
        if reader.translation.is_none()
            && (reader.translation_error.is_some() || !reader.has_pending_translation())
        {
            break;
        }
        std::thread::sleep(POLL);
    }
    write_value(out, snapshot(&reader, Some(model_id), None))
}

fn stop_translation(reader: &mut Reader, out: &Path, cancel_path: &Path, model_id: &str) -> i32 {
    let _ = fs::remove_file(cancel_path);
    reader.cancel_translation();
    // Let the worker notice the flag and close its bridge before this process exits.
    let deadline = Instant::now() + Duration::from_secs(2);
    while reader.translation.is_some() && Instant::now() < deadline {
        reader.poll();
        if reader.translation.is_none() {
            break;
        }
        std::thread::sleep(POLL);
    }
    if reader.document.is_none() {
        // Cancelled before the document was ready: keep the plugin's previous snapshot.
        return 0;
    }
    write_value(out, snapshot(reader, Some(model_id), Some("stopped")))
}

fn consume_cancel(path: &Path) -> bool {
    if path.exists() {
        let _ = fs::remove_file(path);
        true
    } else {
        false
    }
}

fn models(out: &Path) -> i32 {
    let found = match Translator::discover_models(true, |_| {}) {
        Ok(models) => models,
        Err(error) => return fail(out, error),
    };
    let signed_in = found.is_some();
    let (account, account_error) = if signed_in {
        match Translator::account() {
            Ok(account) => (account, None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (None, None)
    };
    write_value(out, models_output(found.as_deref(), account, account_error, signed_in))
}

fn login(out: &Path) -> i32 {
    let cancel_path = sibling(out, ".cancel");
    let mut reader = Reader::new_headless(false);
    reader.start_login();
    if !reader.signing_in() {
        return fail(out, "无法启动 CodeBuddy 登录");
    }
    let mut url: Option<String> = None;
    let mut cancelled = false;
    loop {
        reader.poll();
        if let Some(found) = reader.take_login_url() {
            url = Some(found);
            let _ = write_json(out, &login_output(&reader, "waiting", url.as_deref(), None));
        }
        if !cancelled && cancel_path.exists() {
            let _ = fs::remove_file(&cancel_path);
            cancelled = true;
            reader.cancel_login();
        }
        if !reader.signing_in() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if cancelled {
        return write_value(out, login_output(&reader, "stopped", url.as_deref(), None));
    }
    if let Some(error) = reader.models_error.clone() {
        let _ = write_json(out, &login_output(&reader, "error", url.as_deref(), Some(&error)));
        eprintln!("{error}");
        return 1;
    }
    let deadline = Instant::now() + Duration::from_secs(70);
    loop {
        reader.poll();
        if reader.models_settled()
            && !reader.checking_account
            && (reader.account.is_some() || reader.account_error.is_some() || !reader.signed_in)
        {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(POLL);
    }
    if let Some(error) = reader.models_error.clone() {
        let _ = write_json(out, &login_output(&reader, "error", url.as_deref(), Some(&error)));
        eprintln!("{error}");
        return 1;
    }
    if !reader.signed_in {
        let error = "CodeBuddy 登录未完成。";
        let _ = write_json(out, &login_output(&reader, "error", url.as_deref(), Some(error)));
        eprintln!("{error}");
        return 1;
    }
    write_value(out, login_output(&reader, "done", url.as_deref(), None))
}

fn image(pdf: &str, kind: &str, value: &str, out: &Path) -> i32 {
    let path = Path::new(pdf);
    let result = match kind {
        "page" => {
            let Ok(page) = value.parse::<usize>() else { return usage() };
            pdf::render_page(path, page, 135).and_then(|ppm| Raster::decode(&ppm))
        }
        "block" => {
            let Ok(index) = value.parse::<usize>() else { return usage() };
            pdf::extract(path).and_then(|document| {
                let block = document.blocks.get(index).ok_or_else(|| "找不到该内容块".to_owned())?;
                pdf::render_block(path, block, 155).and_then(|ppm| Raster::decode(&ppm))
            })
        }
        _ => return usage(),
    };
    match result.and_then(|raster| write_png(out, &raster)) {
        Ok(()) => {
            let _ = fs::remove_file(sibling(out, ".error"));
            0
        }
        Err(error) => {
            eprintln!("{error}");
            let _ = fs::write(sibling(out, ".error"), &error);
            1
        }
    }
}

fn write_png(out: &Path, raster: &Raster) -> Result<(), String> {
    let mut rgba = raster.bgra.clone();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, raster.width as u32, raster.height as u32, image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("无法生成页面图像：{error}"))?;
    write_atomic(out, &png)
}

fn snapshot(reader: &Reader, model_id: Option<&str>, state: Option<&str>) -> Value {
    let document = reader.document.as_deref();
    let mut blocks = Vec::new();
    if let Some(document) = document {
        for (index, block) in document.blocks.iter().enumerate() {
            let shown = reader.displayed_translation(index);
            let translation = shown.map(|(text, _)| text.to_owned());
            let preview = translation.is_some()
                && reader.translations.get(index).is_some_and(Option::is_none);
            let math: Vec<Value> = shown.map(|(_, ranges)| {
                ranges.iter().map(|range| json!([range.start, range.end - range.start])).collect()
            }).unwrap_or_default();
            let tex = document.native_equations.get(&index).map(|source| tex_block(source, block.text.as_str()));
            let runs = if matches!(block.kind, BlockKind::Paragraph | BlockKind::Title | BlockKind::Heading | BlockKind::Caption) {
                let source = document.native_inline.get(&index).unwrap_or(&block.text);
                math_runs(source)
            } else {
                Vec::new()
            };
            let runs_zh = shown.filter(|(_, ranges)| !ranges.is_empty())
                .map(|(text, ranges)| run_ranges(text, ranges))
                .unwrap_or_default();
            blocks.push(json!({
                "index": index,
                "kind": kind_name(block.kind),
                "page": block.page,
                "text": block.text,
                "translation": translation,
                "preview": preview,
                "math": math,
                "tex": tex,
                "runs": runs,
                "runs_zh": runs_zh,
                "asset": matches!(block.kind, BlockKind::Figure | BlockKind::Table),
                "caption": document.asset_captions.get(&index).copied(),
            }));
        }
    }
    let mut groups: Vec<(usize, usize)> = document
        .map(|document| document.list_groups.iter().map(|(&start, &end)| (start, end)).collect())
        .unwrap_or_default();
    groups.sort_unstable();
    let groups: Vec<Value> = groups.into_iter().map(|(start, end)| json!({"start": start, "end": end})).collect();
    let (credits_total, credits_incomplete) = reader.credits.summary();
    let credits = credits_total
        .filter(|total| total.is_finite())
        .map_or(Value::Null, |total| json!({"total": total, "complete": !credits_incomplete}));
    let job = reader.translation.as_ref();
    let state = state.unwrap_or_else(|| {
        if let Some(job) = job {
            if job.stopping() { "stopped" } else { "running" }
        } else if reader.translation_error.is_some() {
            "error"
        } else {
            "idle"
        }
    });
    json!({
        "schema": 1,
        "error": null,
        "path": reader.path.as_ref().map(|path| path.to_string_lossy().into_owned()),
        "title": document.map_or("", |document| document.title.as_str()),
        "pages": document.map_or(0, |document| document.pages),
        "notices": document.map(|document| &document.notices),
        "blocks": blocks,
        "groups": groups,
        "translated": reader.translated_count,
        "translatable": reader.translatable_count(),
        "model_id": model_id,
        "preferred_model": model_settings::load_model(),
        "credits": credits,
        "translation": {
            "state": state,
            "done": job.map_or(reader.translated_count, |job| job.done),
            "total": reader.translatable_count(),
            "rate": job.and_then(|job| job.output_rate()).filter(|rate| rate.is_finite()),
            "error": reader.translation_error.as_deref(),
        },
    })
}

/// Display equations: the extractor's TeX keeps commands, so only literals are escaped.
fn tex_block(source: &str, fallback: &str) -> String {
    let text = if source.trim().is_empty() { fallback } else { source };
    tex_escape(text, false)
}

/// Inline formulas come from PDF text, not TeX: a backslash is literal there.
fn tex_escape(text: &str, literal_backslash: bool) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '\\' if literal_backslash => out.push_str("\\backslash "),
            '\\' => out.push('\\'),
            '%' => out.push_str("\\%"),
            '#' => out.push_str("\\#"),
            '&' => out.push_str("\\&"),
            '$' => out.push_str("\\$"),
            '−' | '–' | '—' => out.push('-'),
            '±' => out.push_str("\\pm "),
            '×' => out.push_str("\\times "),
            '÷' => out.push_str("\\div "),
            '≤' => out.push_str("\\le "),
            '≥' => out.push_str("\\ge "),
            '≠' => out.push_str("\\ne "),
            '≈' => out.push_str("\\approx "),
            '∞' => out.push_str("\\infty "),
            '∑' => out.push_str("\\sum "),
            '∏' => out.push_str("\\prod "),
            '∫' => out.push_str("\\int "),
            '→' => out.push_str("\\to "),
            '←' => out.push_str("\\leftarrow "),
            '⇒' => out.push_str("\\Rightarrow "),
            '∈' => out.push_str("\\in "),
            _ => out.push(ch),
        }
    }
    out.trim().to_owned()
}

/// Original text split into plain and TeX runs; empty when the block has no math.
fn math_runs(source: &str) -> Vec<Value> {
    let pieces = math::split_inline(source);
    if !pieces.iter().any(|(_, is_math)| *is_math) {
        return Vec::new();
    }
    pieces.into_iter().filter_map(|(text, is_math)| {
        if text.is_empty() {
            None
        } else if is_math {
            let tex = tex_escape(text, false);
            (!tex.is_empty()).then(|| json!({"tex": tex}))
        } else {
            Some(json!({"text": text}))
        }
    }).collect()
}

/// A translation split at its restored formula byte ranges.
fn run_ranges(text: &str, ranges: &[std::ops::Range<usize>]) -> Vec<Value> {
    let mut runs = Vec::new();
    let mut offset = 0;
    for range in ranges {
        let (Some(plain), Some(formula)) = (text.get(offset..range.start), text.get(range.clone())) else {
            return Vec::new();
        };
        if !plain.is_empty() {
            runs.push(json!({"text": plain}));
        }
        let tex = tex_escape(formula, true);
        if !tex.is_empty() {
            runs.push(json!({"tex": tex}));
        }
        offset = range.end;
    }
    if let Some(tail) = text.get(offset..).filter(|tail| !tail.is_empty()) {
        runs.push(json!({"text": tail}));
    }
    runs
}

fn kind_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Title => "title",
        BlockKind::Author => "author",
        BlockKind::Heading => "heading",
        BlockKind::Paragraph => "paragraph",
        BlockKind::Equation => "equation",
        BlockKind::Caption => "caption",
        BlockKind::ReferenceHeading => "reference_heading",
        BlockKind::Reference => "reference",
        BlockKind::Figure => "figure",
        BlockKind::Table => "table",
    }
}

fn models_output(
    models: Option<&[Model]>, account: Option<Account>, account_error: Option<String>, signed_in: bool,
) -> Value {
    let models: Vec<Value> = models.unwrap_or_default().iter().map(model_json).collect();
    json!({
        "schema": 1,
        "error": null,
        "signed_in": signed_in,
        "models": models,
        "account": account.as_ref().map(account_json),
        "account_error": account_error,
    })
}

fn login_output(reader: &Reader, state: &str, url: Option<&str>, error: Option<&str>) -> Value {
    let models: Vec<Value> = reader.models.iter().map(model_json).collect();
    json!({
        "schema": 1,
        "error": error,
        "state": state,
        "url": url,
        "signed_in": reader.signed_in,
        "account": reader.account.as_ref().map(account_json),
        "models": models,
        "account_error": reader.account_error,
    })
}

fn model_json(model: &Model) -> Value {
    json!({
        "id": model.id,
        "name": model.display_name,
        "multiplier": model.credit_multiplier.filter(|value| value.is_finite()),
    })
}

fn account_json(account: &Account) -> Value {
    json!({
        "name": account.display_name,
        "plan": account.plan,
        "credits": account.remaining_credits.filter(|value| value.is_finite()),
        "error": account.balance_error,
    })
}

fn write_value(out: &Path, value: Value) -> i32 {
    match write_json(out, &value) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn fail(out: &Path, message: impl Into<String>) -> i32 {
    let message = message.into();
    eprintln!("{message}");
    let _ = write_json(out, &json!({"schema": 1, "error": message}));
    1
}

fn write_json(out: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| format!("无法生成输出内容：{error}"))?;
    write_atomic(out, &bytes)
}

fn write_atomic(out: &Path, bytes: &[u8]) -> Result<(), String> {
    let directory = out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| format!("无法写入输出文件：{error}"))?;
    file.as_file_mut()
        .write_all(bytes)
        .map_err(|error| format!("无法写入输出文件：{error}"))?;
    file.persist(out)
        .map_err(|error| format!("无法写入输出文件：{}", error.error))?;
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
