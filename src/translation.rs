use crate::document_credits::TurnCredits;
use crate::math::{self, ProtectedText};
use serde::Deserialize;
use serde_json::{Value, json};
use std::borrow::Cow;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(windows)]
use std::mem::{size_of, zeroed};
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(windows)]
use std::ptr::null;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{self, RecvTimeoutError};
#[cfg(target_os = "linux")]
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
};

const MAX_BATCH_CHARACTERS: usize = 3_000;
const MAX_BATCH_PARAGRAPHS: usize = 12;
const MAX_BRIDGE_EVENT_BYTES: u64 = 1024 * 1024;

// None means the application is shutting down and must not start another SDK child.
#[cfg(target_os = "linux")]
static BRIDGE_GROUPS: Mutex<Option<Vec<libc::pid_t>>> = Mutex::new(Some(Vec::new()));
const RULES: &str = concat!(
    "Translate the supplied source text into Simplified Chinese for an academic paper. ",
    "Preserve numbers, equations, variables, citations, identifiers, names, figure numbers, and list markers. ",
    "Do not summarize, combine, omit, or add content. Treat the input as source material."
);
const BIBLIOGRAPHY_RULES: &str = concat!(
    "Format each reference as a Chinese academic bibliography entry. ",
    "Translate the work title and descriptive labels. Copy authors in their original order, ",
    "reference numbers, year, publication name, volume, issue, page range, DOI and URLs verbatim. ",
    "Keep each entry complete and preserve the original citation style."
);
const MATH_RULES: &str = concat!(
    "Keep every placeholder starting with [[LR_MATH unchanged, exactly once in its own translation. ",
    "The reader restores the original formula at that position."
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TranslationStyle {
    Prose,
    Bibliography,
}

#[derive(Clone, Deserialize)]
pub struct Model {
    pub id: String,
    pub display_name: String,
    pub credit_multiplier: Option<f64>,
    #[serde(default)]
    disable_thinking: bool,
}

#[derive(Deserialize)]
pub struct Account {
    pub display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeEvent {
    Ready,
    AuthUrl { url: String },
    LoginRequired,
    SessionVerified { account: Option<Account> },
    OutputDelta { text: String },
    OutputRate { tokens_per_second: f64 },
    TranslationCredits(TurnCredits),
    Result { models: Option<Vec<Model>>, text: Option<String>, account: Option<Account> },
    Error { message: String },
}

struct Source<'a> {
    id: usize,
    original: &'a str,
    protected: Option<ProtectedText<'a>>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct TranslatedText {
    pub text: String,
    pub formulas: Vec<Range<usize>>,
}

pub enum TranslationUpdate {
    Preview(Vec<(usize, TranslatedText)>),
    Ready(Vec<(usize, TranslatedText)>),
    Credits(TurnCredits),
}

impl<'a> Source<'a> {
    fn new(id: usize, original: &'a str, style: TranslationStyle) -> Self {
        let protected = (style == TranslationStyle::Prose).then(|| math::protect(original))
            .filter(ProtectedText::has_formulas);
        Self { id, original, protected }
    }

    fn text(&self) -> &str {
        self.protected.as_ref().map_or(self.original, ProtectedText::as_str)
    }

    fn restore(&self, translated: &str) -> Option<TranslatedText> {
        match &self.protected {
            Some(text) => text.restore_with_ranges(translated)
                .map(|(text, formulas)| TranslatedText { text, formulas }),
            None => Some(TranslatedText { text: translated.to_owned(), formulas: Vec::new() }),
        }
    }

    fn preview(&self, translated: &str) -> Option<(usize, TranslatedText)> {
        let translated = translated.trim();
        let (text, formulas) = match &self.protected {
            Some(text) => text.preview_with_ranges(translated)?,
            None => (translated.to_owned(), Vec::new()),
        };
        (!text.is_empty()).then_some((self.id, TranslatedText { text, formulas }))
    }
}

#[derive(Deserialize)]
struct TranslationList {
    translations: Vec<Translation>,
}

#[derive(Deserialize)]
struct Translation {
    id: usize,
    text: String,
}

pub struct Translator<'a> {
    model: &'a Model,
    bridge: BridgeProcess,
    on_rate: Box<dyn FnMut(f64) + 'a>,
}

impl<'a> Translator<'a> {
    pub fn new(
        model: &'a Model, cancel: &AtomicBool, on_rate: impl FnMut(f64) + 'a,
    ) -> Result<Self, String> {
        check_cancelled(Some(cancel))?;
        let mut bridge = BridgeProcess::spawn("translate")?;
        match bridge.receive(Instant::now() + Duration::from_secs(45), Some(cancel))? {
            BridgeEvent::Ready => Ok(Self { model, bridge, on_rate: Box::new(on_rate) }),
            _ => Err("CodeBuddy 翻译进程未完成初始化。".into()),
        }
    }

    pub fn discover_models(check_login: bool, on_verified: impl FnMut(Option<Account>)) -> Result<Option<Vec<Model>>, String> {
        let payload = json!({"check_login": check_login, "report_session": true});
        match run_bridge("models", payload, 55, None, |_| {}, on_verified)? {
            BridgeEvent::Result { models: Some(models), .. } => Ok(Some(models)),
            BridgeEvent::LoginRequired => Ok(None),
            _ => Err("CodeBuddy 没有返回可读取的模型列表。".into()),
        }
    }

    pub fn login(cancel: &AtomicBool, on_url: impl FnMut(String)) -> Result<Option<Account>, String> {
        match run_bridge("login", json!({}), 320, Some(cancel), on_url, |_| {})? {
            BridgeEvent::Result { account, .. } => Ok(account),
            _ => Err("CodeBuddy 登录未完成。".into()),
        }
    }

    pub fn translate_batch(
        &mut self,
        paragraphs: &[&str],
        style: TranslationStyle,
        cancel: &AtomicBool,
        mut publish: impl FnMut(TranslationUpdate) -> Result<(), String>,
    ) -> Result<usize, String> {
        let mut batch = Vec::new();
        let mut size = 0;
        let mut skipped = 0;
        for (id, text) in paragraphs.iter().copied().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            let source = Source::new(id, text, style);
            let characters = source.text().chars().count();
            if !batch.is_empty() && (batch.len() >= MAX_BATCH_PARAGRAPHS || size + characters > MAX_BATCH_CHARACTERS) {
                skipped += self.translate_chunk(&mut batch, style, cancel, &mut publish)?;
                batch.clear();
                size = 0;
            }
            batch.push(source);
            size += characters;
        }
        if !batch.is_empty() {
            skipped += self.translate_chunk(&mut batch, style, cancel, &mut publish)?;
        }
        Ok(skipped)
    }

    fn translate_chunk(
        &mut self,
        batch: &mut Vec<Source<'_>>,
        style: TranslationStyle,
        cancel: &AtomicBool,
        publish: &mut impl FnMut(TranslationUpdate) -> Result<(), String>,
    ) -> Result<usize, String> {
        let attempts = if batch.len() == 1 { 1 } else { 2 };
        for attempt in 0..attempts {
            let output = self.translate_text(batch, style, cancel, attempt > 0, publish)?;
            let ready = collect_translated_rows(batch, &output);
            if !ready.is_empty() {
                publish(TranslationUpdate::Ready(ready))?;
            }
            if batch.is_empty() {
                break;
            }
        }
        Ok(batch.len())
    }

    fn translate_text(
        &mut self,
        batch: &[Source<'_>],
        style: TranslationStyle,
        cancel: &AtomicBool,
        fresh_session: bool,
        publish: &mut impl FnMut(TranslationUpdate) -> Result<(), String>,
    ) -> Result<String, String> {
        check_cancelled(Some(cancel))?;
        let (source, format) = if let [paragraph] = batch {
            (Cow::Borrowed(paragraph.text()), "Return only the translated text.")
        } else {
            let rows: Vec<_> = batch.iter().map(|paragraph| json!({ "id": paragraph.id, "text": paragraph.text() })).collect();
            let source = serde_json::to_string(&rows).map_err(|_| "无法编码待翻译段落。".to_owned())?;
            (Cow::Owned(source), concat!(
                "Return only a JSON object with a translations array. Include one object per input. ",
                "In each object write its original numeric id in the id field first, then the translated text in the text field."
            ))
        };
        let has_math = batch.iter().any(|paragraph| paragraph.protected.is_some());
        let instruction = match (style, has_math) {
            (TranslationStyle::Bibliography, _) => format!("{BIBLIOGRAPHY_RULES} {format}"),
            (TranslationStyle::Prose, true) => format!("{MATH_RULES} {format}"),
            (TranslationStyle::Prose, false) => format.to_owned(),
        };
        publish(TranslationUpdate::Preview(Vec::new()))?;
        self.bridge.send(&json!({
            "model": self.model.id, "source": source, "rules": RULES, "instruction": instruction,
            "disable_thinking": self.model.disable_thinking, "fresh_session": fresh_session, "stream_output": true
        }))?;
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut partial = Some(String::new());
        let mut previous = Vec::new();
        let mut credits = TurnCredits::default();
        let result = (|| {
            loop {
                match self.bridge.receive(deadline, Some(cancel))? {
                    BridgeEvent::OutputRate { tokens_per_second } => (self.on_rate)(tokens_per_second),
                    BridgeEvent::TranslationCredits(usage) => credits = usage,
                    BridgeEvent::OutputDelta { text } => {
                        let Some(output) = &mut partial else { continue };
                        if output.len() + text.len() > MAX_BRIDGE_EVENT_BYTES as usize {
                            partial = None;
                            continue;
                        }
                        output.push_str(&text);
                        let snapshot = preview_rows(batch, output);
                        if snapshot != previous {
                            previous.clone_from(&snapshot);
                            publish(TranslationUpdate::Preview(snapshot))?;
                        }
                    }
                    BridgeEvent::Result { text: Some(text), .. } if !text.trim().is_empty() => {
                        return Ok(text.trim().to_owned());
                    }
                    _ => return Err("CodeBuddy 没有返回可读取的译文。".into()),
                }
            }
        })();
        publish(TranslationUpdate::Credits(credits))?;
        result
    }
}

fn collect_translated_rows(batch: &mut Vec<Source<'_>>, output: &str) -> Vec<(usize, TranslatedText)> {
    let single = batch.len() == 1;
    let received = if single {
        None
    } else {
        parse_translation_list(output).or_else(|| {
            streamed_translation_rows(output, false).map(|translations| TranslationList { translations })
        })
    };
    let mut ready = Vec::new();
    batch.retain(|paragraph| {
        let text = if single { Some(output) } else { received.as_ref().and_then(|list| list.text_for(paragraph.id)) };
        if let Some(translation) = text.and_then(|text| paragraph.restore(text)) {
            ready.push((paragraph.id, translation));
            false
        } else {
            true
        }
    });
    ready
}

impl TranslationList {
    fn text_for(&self, id: usize) -> Option<&str> {
        let mut matches = self.translations.iter().filter(|item| item.id == id);
        let text = matches.next()?.text.trim();
        (!text.is_empty() && matches.next().is_none()).then_some(text)
    }
}

fn preview_rows(batch: &[Source<'_>], output: &str) -> Vec<(usize, TranslatedText)> {
    if let [paragraph] = batch {
        return paragraph.preview(output).into_iter().collect();
    }

    let rows = streamed_translation_rows(output, true).unwrap_or_default();
    batch.iter().filter_map(|paragraph| {
        let mut received = rows.iter().filter(|row| row.id == paragraph.id);
        let row = received.next()?;
        if received.next().is_some() {
            return None;
        }
        paragraph.preview(&row.text)
    }).collect()
}

fn streamed_translation_rows(input: &str, include_partial: bool) -> Option<Vec<Translation>> {
    let input = input.trim_start();
    let input = if let Some(fence) = input.strip_prefix("```") {
        let (language, body) = fence.split_once('\n')?;
        let language = language.trim();
        if !language.is_empty() && !language.eq_ignore_ascii_case("json") {
            return None;
        }
        body.trim_start()
    } else {
        input
    };
    let mut input = json_field(input.strip_prefix('{')?, "\"translations\"")?.strip_prefix('[')?;
    let mut rows = Vec::new();
    while !input.trim_start().is_empty() {
        input = input.trim_start();
        let mut reader = serde_json::Deserializer::from_str(input).into_iter::<Translation>();
        match reader.next()? {
            Ok(row) => rows.push(row),
            Err(error) => {
                if include_partial && error.is_eof() {
                    rows.extend(streamed_translation_row(input));
                }
                break;
            }
        }
        let Some(tail) = input[reader.byte_offset()..].trim_start().strip_prefix(',') else { break };
        input = tail;
    }
    Some(rows)
}

fn json_field<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    Some(input.trim_start().strip_prefix(name)?.trim_start().strip_prefix(':')?.trim_start())
}

fn streamed_translation_row(input: &str) -> Option<Translation> {
    let (id, tail) = json_field(input.strip_prefix('{')?, "\"id\"")?.split_once(',')?;
    Some(Translation {
        id: id.trim().parse().ok()?,
        text: json_string_prefix(json_field(tail, "\"text\"")?)?,
    })
}

fn json_string_prefix(input: &str) -> Option<String> {
    input.strip_prefix('"')?;
    let bytes = input.as_bytes();
    let mut end = 1;
    while let Some(&byte) = bytes.get(end) {
        match byte {
            b'"' => return serde_json::from_str(&input[..=end]).ok(),
            b'\\' => {
                let Some(length) = json_escape_length(&bytes[end..]) else { break };
                end += length;
            }
            0..=31 => return None,
            _ => end += 1,
        }
    }
    // Close only the complete prefix. serde_json still decodes the escapes and validates it.
    serde_json::from_str(&format!("{}\"", &input[..end])).ok()
}

fn json_escape_length(escape: &[u8]) -> Option<usize> {
    match *escape.get(1)? {
        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => Some(2),
        b'u' => {
            let hex = std::str::from_utf8(escape.get(2..6)?).ok()?;
            let code = u16::from_str_radix(hex, 16).ok()?;
            if (0xd800..=0xdbff).contains(&code) {
                let suffix = escape.get(6..12)?.strip_prefix(b"\\u")?;
                let low = u16::from_str_radix(std::str::from_utf8(suffix).ok()?, 16).ok()?;
                (0xdc00..=0xdfff).contains(&low).then_some(12)
            } else {
                Some(6)
            }
        }
        _ => None,
    }
}

fn parse_translation_list(output: &str) -> Option<TranslationList> {
    let output = output.trim();
    let content = output.strip_prefix("```")
        .and_then(|body| body.split_once('\n'))
        .and_then(|(language, body)| {
            let language = language.trim();
            (language.is_empty() || language.eq_ignore_ascii_case("json")).then_some(body)
        })
        .and_then(|body| body.strip_suffix("```"))
        .unwrap_or(output);
    serde_json::from_str(content).ok()
}

pub(crate) fn official_login_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    let official = matches!(url.host_str(), Some(
        "copilot.tencent.com" | "www.codebuddy.cn" | "codebuddy.cn" | "www.workbuddy.cn" | "workbuddy.cn"
    ));
    (official && url.scheme() == "https" && url.port().is_none()
        && url.username().is_empty() && url.password().is_none())
        .then(|| url.into())
}

fn bridge_path() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| format!("无法定位阅读器：{error}"))?;
    crate::runtime::bridge(&executable)
        .ok_or_else(|| "找不到 CodeBuddy 接入组件，请使用完整的阅读器目录。".into())
}

fn run_bridge(
    operation: &str,
    payload: Value,
    deadline_seconds: u64,
    cancel: Option<&AtomicBool>,
    mut on_url: impl FnMut(String),
    mut on_verified: impl FnMut(Option<Account>),
) -> Result<BridgeEvent, String> {
    check_cancelled(cancel)?;
    let mut bridge = BridgeProcess::spawn(operation)?;
    bridge.send(&payload)?;
    bridge.input.take();
    let deadline = Instant::now() + Duration::from_secs(deadline_seconds);
    let mut received_login_url = false;
    loop {
        match bridge.receive(deadline, cancel)? {
            event @ BridgeEvent::Result { .. } => return Ok(event),
            BridgeEvent::LoginRequired if operation == "models" => return Ok(BridgeEvent::LoginRequired),
            BridgeEvent::SessionVerified { account } if operation == "models" => on_verified(account),
            BridgeEvent::AuthUrl { url } if operation == "login" && !received_login_url => {
                let url = official_login_url(&url).ok_or_else(|| "CodeBuddy 返回了无法确认的登录地址。".to_owned())?;
                on_url(url);
                received_login_url = true;
            }
            _ => return Err("CodeBuddy 返回了意外的操作响应。".into()),
        }
    }
}

struct BridgeProcess {
    child: Child,
    input: Option<ChildStdin>,
    incoming: mpsc::Receiver<Result<BridgeEvent, String>>,
    #[cfg(windows)]
    job: Option<OwnedHandle>,
    _working_dir: Option<tempfile::TempDir>,
}

impl BridgeProcess {
    fn spawn(operation: &str) -> Result<Self, String> {
        let path = bridge_path()?;
        let temp = (operation == "translate").then(tempfile::tempdir).transpose()
            .map_err(|error| format!("无法创建翻译进程的工作目录：{error}"))?;
        let cwd = temp.as_ref().map_or_else(
            || path.parent().expect("bridge script resides in a directory"),
            |work| work.path(),
        );
        let mut command = Command::new(crate::runtime::node());
        command.arg(&path)
            .arg(operation)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        #[cfg(target_os = "linux")]
        command.process_group(0);
        #[cfg(target_os = "linux")]
        let mut bridge_groups = BRIDGE_GROUPS.lock().unwrap_or_else(PoisonError::into_inner);
        #[cfg(target_os = "linux")]
        let groups = bridge_groups.as_mut().ok_or_else(|| "阅读器正在关闭。".to_owned())?;
        let mut child = command.spawn()
            .map_err(|error| format!("无法启动 CodeBuddy 接入组件，请确认 Node.js 已安装：{error}"))?;
        #[cfg(target_os = "linux")]
        {
            groups.push(child.id() as libc::pid_t);
            drop(bridge_groups);
        }
        #[cfg(windows)]
        let job = match child_job(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("无法管理 CodeBuddy 接入进程：{error}"));
            }
        };
        let input = child.stdin.take();
        let stdout = child.stdout.take().expect("bridge stdout piped");
        let (events, incoming) = mpsc::sync_channel(4);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.by_ref().take(MAX_BRIDGE_EVENT_BYTES + 1).read_line(&mut line) {
                    Ok(0) => break,
                    Ok(length) if length as u64 <= MAX_BRIDGE_EVENT_BYTES => {
                        let event = serde_json::from_str::<BridgeEvent>(&line)
                            .map_err(|_| "无法读取 CodeBuddy 接入组件的响应。".to_owned());
                        let invalid = event.is_err();
                        if events.send(event).is_err() || invalid {
                            break;
                        }
                    }
                    _ => {
                        let _ = events.send(Err("CodeBuddy 接入组件的响应无法读取或过长。".into()));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child, input, incoming,
            #[cfg(windows)]
            job: Some(job),
            _working_dir: temp,
        })
    }

    fn send(&mut self, payload: &Value) -> Result<(), String> {
        let input = self.input.as_mut().ok_or_else(|| "CodeBuddy 接入组件的输入已关闭。".to_owned())?;
        serde_json::to_writer(&mut *input, payload).map_err(|_| "无法向 CodeBuddy 发送请求。".to_owned())?;
        input.write_all(b"\n").map_err(|_| "无法向 CodeBuddy 发送请求。".to_owned())
    }

    fn receive(&mut self, deadline: Instant, cancel: Option<&AtomicBool>) -> Result<BridgeEvent, String> {
        loop {
            check_cancelled(cancel)?;
            if Instant::now() >= deadline {
                return Err("等待 CodeBuddy 响应超时。".into());
            }
            match self.incoming.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(BridgeEvent::Error { message })) | Ok(Err(message)) => return Err(message),
                Ok(event) => return event,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    #[cfg(windows)]
                    let status = self.child.try_wait().ok().flatten().and_then(|status| status.code())
                        .map_or(String::new(), |code| format!("（退出码 {code}）"));
                    #[cfg(target_os = "linux")]
                    let status = "";
                    return Err(format!("CodeBuddy 接入组件未返回结果{status}。"));
                }
            }
        }
    }
}

impl Drop for BridgeProcess {
    fn drop(&mut self) {
        self.input.take();
        // Closing the job terminates both the bridge and any SDK CLI child.
        #[cfg(windows)]
        drop(self.job.take());
        #[cfg(target_os = "linux")]
        {
            // Keep the group leader unreaped until after signalling its group,
            // so the PID cannot be recycled as an unrelated process group.
            let group = self.child.id() as libc::pid_t;
            let mut bridge_groups = BRIDGE_GROUPS.lock().unwrap_or_else(PoisonError::into_inner);
            if unsafe { libc::killpg(group, libc::SIGKILL) } != 0 {
                let _ = self.child.kill();
            }
            if let Some(groups) = bridge_groups.as_mut() {
                groups.retain(|&active| active != group);
            }
        }
        let _ = self.child.wait();
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn shutdown_bridges() {
    let mut bridge_groups = BRIDGE_GROUPS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(groups) = bridge_groups.take() {
        for group in groups {
            // The registry lock excludes BridgeProcess::drop while signalling.
            unsafe { libc::killpg(group, libc::SIGKILL) };
        }
    }
}

fn check_cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|flag| flag.load(Relaxed)) {
        Err("操作已取消。".into())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn child_job(child: &Child) -> std::io::Result<OwnedHandle> {
    unsafe {
        let raw = CreateJobObjectW(null(), null());
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let handle = OwnedHandle::from_raw_handle(raw);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(raw, JobObjectExtendedLimitInformation, (&raw const limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) == 0
            || AssignProcessToJobObject(raw, child.as_raw_handle()) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_unique_translation_ids_from_a_fenced_reply() {
        let reply = "```json\n{\"translations\":[{\"id\":7,\"text\":\"第二段\"},{\"id\":2,\"text\":\"第一段\"},{\"id\":7,\"text\":\"重复\"}]}\n```";
        let list = parse_translation_list(reply).unwrap();
        assert_eq!(list.text_for(2), Some("第一段"));
        assert_eq!(list.text_for(7), None);
        assert_eq!(list.text_for(8), None);
    }

    #[test]
    fn salvages_closed_rows_from_an_interrupted_reply_without_losing_formulas() {
        let mut batch = vec![
            Source::new(0, "First", TranslationStyle::Prose),
            Source::new(1, "Given $x_1$", TranslationStyle::Prose),
            Source::new(2, "Third", TranslationStyle::Prose),
        ];
        let output = r#"{"translations":[{"id":0,"text":"第一段"},{"id":1,"text":"缺少公式"},{"id":2,"text":"第三"#;
        let ready = collect_translated_rows(&mut batch, output);
        assert_eq!(ready.iter().map(|(id, row)| (*id, row.text.as_str())).collect::<Vec<_>>(), [(0, "第一段")]);
        assert_eq!(batch.iter().map(|row| row.id).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn previews_batched_translations_as_json_strings_arrive() {
        let batch = [Source::new(2, "One", TranslationStyle::Prose), Source::new(7, "Two", TranslationStyle::Prose)];
        let prefix = "```json\n".to_owned()
            + r#"{"translations":[{"id":7,"text":"第二段\n已生成"},{"id":2,"text":"第一段\"\u6587\uD83D"#;
        let rows = preview_rows(&batch, &prefix);
        assert_eq!(rows.iter().map(|(id, row)| (*id, row.text.as_str())).collect::<Vec<_>>(),
            [(2, "第一段\"文"), (7, "第二段\n已生成")]);
        let complete = prefix + r#"\uDE00"}]}"#;
        assert_eq!(preview_rows(&batch, &complete)[0].1.text, "第一段\"文😀");
    }

    #[test]
    fn previews_protected_formulas_without_displaying_torn_placeholders() {
        let batch = [Source::new(0, "Given $x_1$", TranslationStyle::Prose)];
        let marker = batch[0].text().strip_prefix("Given ").unwrap();
        for end in 1..marker.len() {
            let preview = preview_rows(&batch, &format!("设 {}", &marker[..end]));
            assert_eq!(preview[0].1.text.trim(), "设");
            assert!(preview[0].1.formulas.is_empty());
        }
        let rows = preview_rows(&batch, &format!("设 {marker} 为常数"));
        assert_eq!(rows[0].1.text, "设 $x_1$ 为常数");
        assert_eq!(&rows[0].1.text[rows[0].1.formulas[0].clone()], "$x_1$");
    }

    #[test]
    fn validates_codebuddy_browser_destination() {
        assert!(official_login_url("https://copilot.tencent.com/login?state=example").is_some());
        assert!(official_login_url("https://copilot.tencent.com.example.com/login").is_none());
        assert!(official_login_url("http://copilot.tencent.com/login").is_none());
        assert!(official_login_url("https://other@copilot.tencent.com/login").is_none());
    }
}
