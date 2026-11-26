mod http;

use crate::digest::{Sha256, sha256_reader};
use crate::model_settings;
use serde::Deserialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::ptr::null;
use std::time::{Duration, Instant, SystemTime};
use url::Url;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "zhhpan/literature-reader";
const GITEE_REPO: &str = "zhhpan/literature-reader";
const MANIFEST_FILE: &str = "reader-update.json";
/// Gitee is checked first; it mirrors releases from the same tag and carries
/// its own manifest so downloads stay on Gitee when it has the version.
const GITEE_MANIFEST: &str = "https://gitee.com/zhhpan/literature-reader/raw/master/gitee-update.json";
const ASSET_PREFIX: &str = "LiteratureReader-Setup-";
const ASSET_SUFFIX: &str = "-x64.exe";
const MAX_RELEASE_JSON: u64 = 1024 * 1024;
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const FILE_SHARE_READ: u32 = 1;

#[derive(Clone)]
pub struct Release {
    pub version: String,
    pub size: u64,
    url: Url,
    digest: [u8; 32],
}

pub struct VerifiedInstaller {
    path: PathBuf,
    // Prevent a verified installer from being replaced until we hand it to Windows.
    _verified_file: File,
}

#[derive(Deserialize)]
struct ReleaseInfo {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: u64,
    browser_download_url: String,
    digest: Option<String>,
}

pub fn check() -> Result<Option<Release>, String> {
    let current = version(CURRENT_VERSION).expect("package version is numeric");
    remove_old_downloads(current);
    let mut errors = Vec::new();
    match latest_gitee_manifest() {
        Ok(Some(info)) => match newer_gitee_release(info, current) {
            Ok(Some(release)) => return Ok(Some(release)),
            Ok(None) => {}
            Err(error) => errors.push(error),
        },
        Ok(None) => {}
        Err(error) => errors.push(error),
    }
    match latest_manifest() {
        Ok(Some(info)) => match newer_release(info, current) {
            Ok(Some(release)) => return Ok(Some(release)),
            Ok(None) => {}
            Err(error) => errors.push(error),
        },
        Ok(None) => {}
        Err(error) => errors.push(error),
    }
    latest_api(current).map_err(|error| {
        if errors.is_empty() {
            error
        } else {
            format!("{} {error}", errors.join(" "))
        }
    })
}

fn latest_gitee_manifest() -> Result<Option<ReleaseInfo>, String> {
    let mut url = Url::parse(GITEE_MANIFEST).expect("static Gitee manifest URL");
    for _ in 0..=3 {
        let response = http::get(&url, false).map_err(|error| format!("无法读取 Gitee 更新清单：{error}"))?;
        match response.status {
            200 => {
                let info = read_release(response)?;
                if info.draft || info.prerelease {
                    return Ok(None);
                }
                return Ok(Some(info));
            }
            404 => return Ok(None),
            301 | 302 | 303 | 307 | 308 => {
                url = response.location.as_deref().and_then(|location| url.join(location).ok())
                    .filter(allowed_gitee_manifest_redirect)
                    .ok_or_else(|| "Gitee 返回了无法确认的更新清单地址。".to_owned())?;
            }
            status => return Err(format!("Gitee 更新清单返回 HTTP {status}。")),
        }
    }
    Err("Gitee 更新清单重定向次数过多。".into())
}

fn allowed_gitee_manifest_redirect(url: &Url) -> bool {
    safe_download_url(url) && matches!(url.host_str(), Some("gitee.com" | "raw.giteeusercontent.com"))
}

fn gitee_download_url(url: &Url, tag: &str, name: &str) -> bool {
    safe_download_url(url)
        && url.host_str() == Some("gitee.com")
        && url.path() == format!("/{GITEE_REPO}/releases/download/{tag}/{name}")
}

fn newer_gitee_release(info: ReleaseInfo, current: [u32; 3]) -> Result<Option<Release>, String> {
    if info.draft || info.prerelease {
        return Ok(None);
    }
    let (number, parsed) = info.tag_name.strip_prefix('v')
        .and_then(|number| version(number).map(|parsed| (number, parsed)))
        .ok_or_else(|| "Gitee 正式版本号不符合 v主版本.次版本.修订号 格式。".to_owned())?;
    if parsed <= current {
        return Ok(None);
    }
    let name = asset_name(number);
    let asset = info.assets.into_iter().find(|asset| asset.name == name && asset.state == "uploaded")
        .ok_or_else(|| "Gitee 新版还没有可用的 Windows x64 安装包。".to_owned())?;
    let url = Url::parse(&asset.browser_download_url)
        .map_err(|_| "Gitee 安装包的下载地址无效。".to_owned())?;
    if !gitee_download_url(&url, &info.tag_name, &name) || !(1..=MAX_INSTALLER_BYTES).contains(&asset.size) {
        return Err("Gitee 新版安装包的下载地址或大小不符合要求。".into());
    }
    let digest = asset.digest.as_deref().and_then(parse_digest)
        .ok_or_else(|| "Gitee 未提供新版安装包的 SHA-256 校验值。".to_owned())?;
    Ok(Some(Release {
        version: number.to_owned(), size: asset.size, url, digest,
    }))
}

fn latest_manifest() -> Result<Option<ReleaseInfo>, String> {
    let mut url = Url::parse(&format!("https://github.com/{REPO}/releases/latest/download/{MANIFEST_FILE}")).unwrap();
    let mut expected_tag: Option<String> = None;
    for _ in 0..=3 {
        let response = http::get(&url, false).map_err(|error| format!("无法读取 GitHub 更新清单：{error}"))?;
        match response.status {
            200 => {
                let info = read_release(response)?;
                if info.draft || info.prerelease || expected_tag.as_ref().is_some_and(|tag| tag != &info.tag_name) {
                    return Err("更新清单与 GitHub 正式版本不符。".into());
                }
                return Ok(Some(info));
            }
            404 => return Ok(None),
            301 | 302 | 303 | 307 | 308 => {
                let next = response.location.as_deref().and_then(|location| url.join(location).ok())
                    .ok_or_else(|| "GitHub 返回了无法确认的更新清单地址。".to_owned())?;
                if let Some(tag) = manifest_release_tag(&next) {
                    if expected_tag.as_deref().is_some_and(|expected| expected != tag) {
                        return Err("GitHub 更新清单重定向到了不同版本。".into());
                    }
                    expected_tag = Some(tag.to_owned());
                } else if !allowed_asset_redirect(&next) {
                    return Err("GitHub 返回了无法确认的更新清单地址。".into());
                }
                url = next;
            }
            status => return Err(format!("GitHub 更新清单返回 HTTP {status}。")),
        }
    }
    Err("GitHub 更新清单重定向次数过多。".into())
}

fn manifest_release_tag(url: &Url) -> Option<&str> {
    if !safe_download_url(url) || url.host_str() != Some("github.com") || url.query().is_some() {
        return None;
    }
    let release_path = url.path().strip_prefix('/')?.strip_prefix(REPO)?.strip_prefix("/releases/download/")?;
    let (tag, name) = release_path.split_once('/')?;
    (name == MANIFEST_FILE && version(tag.strip_prefix('v')?).is_some()).then_some(tag)
}

fn latest_api(current: [u32; 3]) -> Result<Option<Release>, String> {
    let url = Url::parse(&format!("https://api.github.com/repos/{REPO}/releases/latest")).unwrap();
    let response = http::get(&url, true).map_err(|error| format!("无法连接 GitHub API：{error}"))?;
    if response.status != 200 {
        let limited = response.status == 429 || response.status == 403 && (
            response.header("x-ratelimit-remaining").is_some_and(|value| value.trim() == "0")
                || response.header("retry-after").is_some()
        );
        if limited {
            return Err(format!("GitHub API 已限流（HTTP {}）。请稍后重试，或从 GitHub 发布页下载安装包。", response.status));
        }
        if response.status == 403 {
            return Err("GitHub API 拒绝访问（HTTP 403）。可检查网络或代理，也可在浏览器访问 GitHub 发布页。".into());
        }
        return Err(format!("GitHub API 返回 HTTP {}，请稍后重试。", response.status));
    }
    newer_release(read_release(response)?, current)
}

fn read_release(response: http::Response) -> Result<ReleaseInfo, String> {
    let mut buffer = Vec::new();
    response.take(MAX_RELEASE_JSON + 1).read_to_end(&mut buffer)
        .map_err(|error| format!("无法读取 GitHub 发布信息：{error}"))?;
    if buffer.len() as u64 > MAX_RELEASE_JSON {
        return Err("GitHub 返回的发布信息过长。".into());
    }
    serde_json::from_slice(&buffer).map_err(|_| "无法解析 GitHub 发布信息。".to_owned())
}

fn newer_release(info: ReleaseInfo, current: [u32; 3]) -> Result<Option<Release>, String> {
    if info.draft || info.prerelease {
        return Ok(None);
    }
    let (number, parsed) = info.tag_name.strip_prefix('v')
        .and_then(|number| version(number).map(|parsed| (number, parsed)))
        .ok_or_else(|| "GitHub 正式版本号不符合 v主版本.次版本.修订号 格式。".to_owned())?;
    if parsed <= current {
        return Ok(None);
    }

    let name = asset_name(number);
    let expected_url = format!("https://github.com/{REPO}/releases/download/{}/{name}", info.tag_name);
    let asset = info.assets.into_iter().find(|asset| asset.name == name && asset.state == "uploaded")
        .ok_or_else(|| "新版还没有可用的 Windows x64 安装包。".to_owned())?;
    if asset.browser_download_url != expected_url || !(1..=MAX_INSTALLER_BYTES).contains(&asset.size) {
        return Err("新版安装包的下载地址或大小不符合要求。".into());
    }
    let digest = asset.digest.as_deref().and_then(parse_digest)
        .ok_or_else(|| "GitHub 未提供新版安装包的 SHA-256 校验值。".to_owned())?;
    Ok(Some(Release {
        version: number.to_owned(), size: asset.size, url: Url::parse(&expected_url).unwrap(), digest,
    }))
}

fn version(text: &str) -> Option<[u32; 3]> {
    fn component(part: &str) -> Option<u32> {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0')) {
            return None;
        }
        part.parse().ok()
    }

    let mut parts = text.split('.');
    let number = [component(parts.next()?)?, component(parts.next()?)?, component(parts.next()?)?];
    parts.next().is_none().then_some(number)
}

fn parse_digest(text: &str) -> Option<[u8; 32]> {
    let digits = text.strip_prefix("sha256:")?.as_bytes();
    let mut digest = [0; 32];
    if digits.len() != digest.len() * 2 {
        return None;
    }
    for (byte, digits) in digest.iter_mut().zip(digits.chunks_exact(2)) {
        let high = char::from(digits[0]).to_digit(16)?;
        let low = char::from(digits[1]).to_digit(16)?;
        *byte = (high * 16 + low) as u8;
    }
    Some(digest)
}

fn asset_name(number: &str) -> String {
    format!("{ASSET_PREFIX}{number}{ASSET_SUFFIX}")
}

fn asset_response(url: &Url) -> Result<http::Response, String> {
    let mut url = url.clone();
    for _ in 0..=3 {
        let response = http::get(&url, false).map_err(|error| format!("无法下载更新：{error}"))?;
        match response.status {
            200 => return Ok(response),
            301 | 302 | 303 | 307 | 308 => {
                url = response.location.as_deref().and_then(|location| url.join(location).ok())
                    .filter(allowed_asset_redirect)
                    .ok_or_else(|| "GitHub 返回了无法确认的安装包下载地址。".to_owned())?;
            }
            status => return Err(format!("GitHub 安装包下载返回 HTTP {status}。")),
        }
    }
    Err("GitHub 安装包下载重定向次数过多。".into())
}

fn allowed_asset_redirect(url: &Url) -> bool {
    if !safe_download_url(url) {
        return false;
    }
    match url.host_str() {
        Some("release-assets.githubusercontent.com") => true,
        Some("gitee.com" | "file.gitee.com" | "raw.giteeusercontent.com") => true,
        _ => false,
    }
}

fn safe_download_url(url: &Url) -> bool {
    url.scheme() == "https" && url.port().is_none() && url.username().is_empty() && url.password().is_none()
        && url.fragment().is_none()
}

pub fn download(release: &Release, mut progress: impl FnMut(u64, u64)) -> Result<VerifiedInstaller, String> {
    let directory = update_directory().ok_or_else(|| "无法定位更新下载目录。".to_owned())?;
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建更新下载目录：{error}"))?;
    let mut partial = tempfile::Builder::new().prefix("download-").suffix(".partial").tempfile_in(&directory)
        .map_err(|error| format!("无法创建更新文件：{error}"))?;
    let mut response = asset_response(&release.url)?;
    let mut buffer = vec![0; 64 * 1024];
    let hash = Sha256::new().map_err(|error| error.to_string())?;
    let mut received = 0;
    let started = Instant::now();
    let mut last_report = started;
    loop {
        if started.elapsed() > Duration::from_secs(15 * 60) {
            return Err("下载安装包超时。请检查网络后重试。".into());
        }
        let read = response.read(&mut buffer).map_err(|error| format!("下载安装包时连接中断：{error}"))?;
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > release.size {
            return Err("安装包的下载大小与 GitHub 发布记录不一致。".into());
        }
        partial.write_all(&buffer[..read]).map_err(|error| format!("无法保存安装包：{error}"))?;
        hash.update(&buffer[..read]).map_err(|error| error.to_string())?;
        if received == release.size || last_report.elapsed() >= Duration::from_millis(200) {
            progress(received, release.size);
            last_report = Instant::now();
        }
    }
    if received != release.size || hash.finish().map_err(|error| error.to_string())? != release.digest {
        return Err("安装包未通过 SHA-256 校验，请重新下载。".into());
    }
    let temporary = partial.into_temp_path();
    mark_internet_download(&temporary, &release.url)?;
    let path = directory.join(asset_name(&release.version));
    temporary.persist(&path).map_err(|error| format!("无法保存已校验的安装包：{}", error.error))?;

    match lock_verified(&path, release) {
        Ok(file) => Ok(VerifiedInstaller { path, _verified_file: file }),
        Err(error) => {
            let _ = fs::remove_file(&path);
            Err(error)
        }
    }
}

fn mark_internet_download(path: &Path, source: &Url) -> Result<(), String> {
    // Preserve the Internet zone when handing a download to Windows for execution.
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    fs::write(stream, format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={source}\r\n"))
        .map_err(|error| format!("无法保留安装包的 Windows 下载安全标记：{error}"))
}

fn lock_verified(path: &Path, release: &Release) -> Result<File, String> {
    let file = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(path)
        .map_err(|error| format!("无法打开安装包进行校验：{error}"))?;
    let digest = sha256_reader(&file).map_err(|error| format!("无法校验安装包：{error}"))?;
    if digest != release.digest {
        return Err("安装包内容已变更，请重新下载。".into());
    }
    Ok(file)
}

pub fn launch(installer: &VerifiedInstaller, owner: HWND) -> Result<(), String> {
    let reader = std::env::current_exe().map_err(|error| format!("无法定位当前安装位置：{error}"))?;
    let path: Vec<u16> = installer.path.as_os_str().encode_wide().chain([0]).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain([0]).collect();
    let options = format!(
        "/READERUPDATE=1 /SILENT /SP- /NORESTART /READERPID={} /READEREXE=\"",
        std::process::id()
    );
    let arguments: Vec<u16> = options.encode_utf16().chain(reader.as_os_str().encode_wide())
        .chain([b'"' as u16, 0]).collect();
    let result = unsafe { ShellExecuteW(owner, verb.as_ptr(), path.as_ptr(), arguments.as_ptr(), null(), SW_SHOWNORMAL) };
    if result as isize <= 32 {
        return Err(format!("Windows 未能启动安装程序（{}）。", result as isize));
    }
    Ok(())
}

fn update_directory() -> Option<PathBuf> {
    Some(model_settings::data_directory()?.join("updates"))
}

fn remove_old_downloads(current: [u32; 3]) {
    let Some(directory) = update_directory() else { return };
    let Ok(entries) = fs::read_dir(directory) else { return };
    for entry in entries.flatten().filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file())) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let older = name.strip_prefix(ASSET_PREFIX).and_then(|name| name.strip_suffix(ASSET_SUFFIX))
            .and_then(version).is_some_and(|number| number <= current);
        let abandoned = name.starts_with("download-") && name.ends_with(".partial")
            && entry.metadata().and_then(|metadata| metadata.modified()).ok().is_some_and(|modified| {
                SystemTime::now().duration_since(modified).is_ok_and(|age| age > Duration::from_secs(24 * 60 * 60))
            });
        if older || abandoned {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_info() -> ReleaseInfo {
        ReleaseInfo {
            tag_name: "v0.1.10".into(),
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: asset_name("0.1.10"),
                state: "uploaded".into(),
                size: 12,
                browser_download_url: format!("https://github.com/{REPO}/releases/download/v0.1.10/{}", asset_name("0.1.10")),
                digest: Some(format!("sha256:{}", "aa".repeat(32))),
            }],
        }
    }

    #[test]
    fn accepts_only_newer_stable_versions_and_the_official_asset() {
        let release = newer_release(release_info(), [0, 1, 2]).unwrap().unwrap();
        assert_eq!(release.version, "0.1.10");
        assert_eq!(release.digest, [0xaa; 32]);
        assert!(newer_release(release_info(), [0, 1, 10]).unwrap().is_none());
        assert!(newer_release(release_info(), [0, 2, 0]).unwrap().is_none());
        let mut preview = release_info();
        preview.prerelease = true;
        assert!(newer_release(preview, [0, 1, 2]).unwrap().is_none());
        let mut unrelated = release_info();
        unrelated.assets[0].browser_download_url = "https://example.com/installer.exe".into();
        assert!(newer_release(unrelated, [0, 1, 2]).is_err());
        let mut unknown = release_info();
        unknown.assets[0].digest = None;
        assert!(newer_release(unknown, [0, 1, 2]).is_err());
        assert!(version("0.01.2").is_none());
    }

    #[test]
    fn accepts_only_gitee_release_assets() {
        let mut info = release_info();
        info.tag_name = "v0.2.0".into();
        info.assets[0].name = asset_name("0.2.0");
        info.assets[0].browser_download_url = format!(
            "https://gitee.com/{GITEE_REPO}/releases/download/v0.2.0/{}", asset_name("0.2.0"));
        let release = newer_gitee_release(info, [0, 1, 2]).unwrap().unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.url.host_str(), Some("gitee.com"));

        let mut wrong = release_info();
        wrong.tag_name = "v0.2.0".into();
        wrong.assets[0].name = asset_name("0.2.0");
        wrong.assets[0].browser_download_url = format!(
            "https://gitee.com/other/literature-reader/releases/download/v0.2.0/{}", asset_name("0.2.0"));
        assert!(newer_gitee_release(wrong, [0, 1, 2]).is_err());
    }

    #[test]
    fn restricts_download_redirects_to_githubs_release_host() {
        let allowed = |url| allowed_asset_redirect(&Url::parse(url).unwrap());
        assert!(allowed("https://release-assets.githubusercontent.com/a?token=example"));
        assert!(!allowed("https://release-assets.githubusercontent.com.example.com/a"));
        assert!(!allowed("http://release-assets.githubusercontent.com/a"));
        assert!(!allowed("https://name@release-assets.githubusercontent.com/a"));
    }

    #[test]
    fn accepts_only_our_versioned_manifest_redirect() {
        let url = |path: &str| Url::parse(&format!("https://github.com/{path}")).unwrap();
        assert_eq!(manifest_release_tag(&url(&format!("{REPO}/releases/download/v0.1.10/{MANIFEST_FILE}"))), Some("v0.1.10"));
        for path in [
            format!("other/repo/releases/download/v0.1.10/{MANIFEST_FILE}"),
            format!("{REPO}/releases/download/v0.1.10-rc/{MANIFEST_FILE}"),
            format!("{REPO}/releases/download/v0.1.10/{MANIFEST_FILE}?other=yes"),
            format!("{REPO}/releases/download/v0.1.10/extra/{MANIFEST_FILE}"),
        ] {
            assert!(manifest_release_tag(&url(&path)).is_none());
        }
    }

    #[test]
    fn preserves_the_windows_download_mark_while_renaming() {
        let directory = tempfile::tempdir().unwrap();
        let temporary = tempfile::NamedTempFile::new_in(directory.path()).unwrap().into_temp_path();
        let release = newer_release(release_info(), [0, 1, 2]).unwrap().unwrap();
        mark_internet_download(&temporary, &release.url).unwrap();
        temporary.persist(directory.path().join("installer.exe")).unwrap();
        let marked = fs::read_to_string(directory.path().join("installer.exe:Zone.Identifier")).unwrap();
        assert!(marked.contains("ZoneId=3\r\n"));
        assert!(marked.contains(release.url.as_str()));
    }
}
