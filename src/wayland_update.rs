use serde::Deserialize;
use std::process::{Command, Stdio};
use url::Url;

const REPO: &str = "zhhpan/literature-reader";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MANIFEST: &str = "reader-update.json";
const MAX_RELEASE_JSON: usize = 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone)]
pub struct Release {
    pub version: String,
    pub download_url: String,
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

#[derive(Deserialize)]
struct Headers {
    status: u16,
    location: Option<String>,
    limited: bool,
}

pub fn check() -> Result<Option<Release>, String> {
    if std::env::consts::ARCH != "x86_64" {
        return Err("当前发布没有此处理器架构的 Wayland 压缩包。".into());
    }
    let current = parse_version(CURRENT_VERSION).expect("package version is numeric");
    match latest_manifest()? {
        Some(info) => newer_release(info, current),
        None => newer_release(latest_api()?, current),
    }
}

fn latest_manifest() -> Result<Option<ReleaseInfo>, String> {
    let mut url = Url::parse(&format!("https://github.com/{REPO}/releases/latest/download/{MANIFEST}"))
        .expect("valid GitHub manifest URL");
    let mut expected_tag: Option<String> = None;
    for _ in 0..=3 {
        let (headers, body) = match get(&url, false) {
            Ok(response) => response,
            Err(_) => return Ok(None),
        };
        match headers.status {
            200 => {
                let info = read_release(&body)?;
                if info.draft || info.prerelease
                    || expected_tag.as_ref().is_some_and(|tag| tag != &info.tag_name)
                {
                    return Err("更新清单与 GitHub 正式版本不符。".into());
                }
                return Ok(Some(info));
            }
            403 | 404 | 429 | 500..=599 => return Ok(None),
            301 | 302 | 303 | 307 | 308 => {
                let next = headers.location.as_deref().and_then(|location| url.join(location).ok())
                    .filter(safe_https)
                    .ok_or_else(|| "GitHub 返回了无法确认的更新清单地址。".to_owned())?;
                if let Some(tag) = manifest_release_tag(&next) {
                    if expected_tag.as_deref().is_some_and(|expected| expected != tag) {
                        return Err("GitHub 更新清单重定向到了不同版本。".into());
                    }
                    expected_tag = Some(tag.to_owned());
                } else if next.host_str() != Some("release-assets.githubusercontent.com") {
                    return Err("GitHub 返回了无法确认的更新清单地址。".into());
                }
                url = next;
            }
            status => return Err(format!("GitHub 更新清单返回 HTTP {status}。")),
        }
    }
    Err("GitHub 更新清单重定向次数过多。".into())
}

fn safe_https(url: &Url) -> bool {
    url.scheme() == "https" && url.port().is_none() && url.username().is_empty()
        && url.password().is_none() && url.fragment().is_none()
}

fn manifest_release_tag(url: &Url) -> Option<&str> {
    if url.host_str() != Some("github.com") || url.query().is_some() {
        return None;
    }
    let release_path = url.path().strip_prefix('/')?.strip_prefix(REPO)?.strip_prefix("/releases/download/")?;
    let (tag, name) = release_path.split_once('/')?;
    (name == MANIFEST && parse_version(tag.strip_prefix('v')?).is_some()).then_some(tag)
}

fn latest_api() -> Result<ReleaseInfo, String> {
    let url = Url::parse(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .expect("valid GitHub API URL");
    let (headers, body) = get(&url, true).map_err(|error| format!("无法连接 GitHub API。{error}"))?;
    match headers.status {
        200 => read_release(&body),
        429 => Err("GitHub API 已限流。请稍后重试。".into()),
        403 if headers.limited => Err("GitHub API 已限流。请稍后重试。".into()),
        403 => Err("GitHub API 拒绝访问（HTTP 403）。请检查网络或代理。".into()),
        404 => Err("GitHub 上没有可用的正式版本（HTTP 404）。".into()),
        status => Err(format!("GitHub API 返回 HTTP {status}。请稍后重试。")),
    }
}

fn read_release(body: &[u8]) -> Result<ReleaseInfo, String> {
    serde_json::from_slice(body).map_err(|_| "无法解析 GitHub 发布信息。".into())
}

fn newer_release(info: ReleaseInfo, current: [u32; 3]) -> Result<Option<Release>, String> {
    if info.draft || info.prerelease {
        return Err("GitHub 返回的版本不是正式发布。".into());
    }
    let (version, parsed) = info.tag_name.strip_prefix('v')
        .and_then(|version| parse_version(version).map(|parsed| (version, parsed)))
        .ok_or_else(|| "GitHub 正式版本号不符合 v主版本.次版本.修订号 格式。".to_owned())?;
    if parsed <= current {
        return Ok(None);
    }

    let name = format!("LiteratureReader-Wayland-{version}-x86_64.tar.gz");
    let expected_url = format!("https://github.com/{REPO}/releases/download/{}/{name}", info.tag_name);
    let asset = info.assets.into_iter().find(|asset| asset.name == name)
        .ok_or_else(|| format!("新版 v{version} 尚未发布 Linux Wayland x86_64 压缩包。"))?;
    if asset.state != "uploaded" || asset.browser_download_url != expected_url
        || !(1..=MAX_ARCHIVE_BYTES).contains(&asset.size)
    {
        return Err("新版 Linux 压缩包的上传状态、下载地址或大小不符合要求。".into());
    }
    let digest = asset.digest.as_deref().and_then(|digest| digest.strip_prefix("sha256:"));
    if digest.is_none_or(|digits| digits.len() != 64 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())) {
        return Err("GitHub 未提供新版 Linux 压缩包的有效 SHA-256 校验值。".into());
    }
    Ok(Some(Release { version: version.to_owned(), download_url: expected_url }))
}

fn parse_version(text: &str) -> Option<[u32; 3]> {
    let mut parts = text.split('.');
    let mut version = [0; 3];
    for part in &mut version {
        let digits = parts.next()?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())
            || (digits.len() > 1 && digits.starts_with('0'))
        {
            return None;
        }
        *part = digits.parse().ok()?;
    }
    parts.next().is_none().then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_owned(), state: "uploaded".into(), size: 5,
            browser_download_url: format!("https://github.com/{REPO}/releases/download/v0.1.10/{name}"),
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        }
    }

    fn release(assets: Vec<Asset>) -> ReleaseInfo {
        ReleaseInfo { tag_name: "v0.1.10".into(), draft: false, prerelease: false, assets }
    }

    #[test]
    fn a_mixed_release_selects_only_the_matching_wayland_archive() {
        let windows = "LiteratureReader-Setup-0.1.10-x64.exe";
        let archive = "LiteratureReader-Wayland-0.1.10-x86_64.tar.gz";
        let newest = newer_release(release(vec![asset(windows), asset(archive)]), [0, 1, 9])
            .unwrap().unwrap();
        assert_eq!(newest.version, "0.1.10");
        assert_eq!(newest.download_url, asset(archive).browser_download_url);
        assert!(newer_release(release(vec![asset(windows)]), [0, 1, 9]).is_err());
        assert!(newer_release(release(vec![asset(archive)]), [0, 1, 10]).unwrap().is_none());
    }
}

fn get(url: &Url, api: bool) -> Result<(Headers, Vec<u8>), String> {
    let output = Command::new(crate::runtime::node())
        .args(["-e", FETCH_RELEASE, "--", url.as_str(), CURRENT_VERSION, if api { "api" } else { "asset" }])
        .arg(MAX_RELEASE_JSON.to_string())
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_TLS_REJECT_UNAUTHORIZED")
        .env("NODE_USE_ENV_PROXY", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("无法启动更新网络组件。{error}"))?;
    if !output.status.success() {
        return Err("读取 GitHub 响应失败、超时或响应过长。".into());
    }
    let separator = output.stdout.iter().position(|byte| *byte == b'\n')
        .ok_or_else(|| "更新网络组件返回了无效响应。".to_owned())?;
    let header = &output.stdout[..separator];
    let body = &output.stdout[separator + 1..];
    if body.len() > MAX_RELEASE_JSON {
        return Err("GitHub 返回的发布信息过长。".into());
    }
    let headers = serde_json::from_slice(header).map_err(|_| "更新网络组件返回了无效响应。".to_owned())?;
    Ok((headers, body.to_vec()))
}

// The bundled Node runtime handles TLS. Rust owns redirect and release validation.
const FETCH_RELEASE: &str = r#"
const { get } = require('node:https');
const [, url, version, mode, limitText] = process.argv;
const limit = Number(limitText);
const headers = { 'User-Agent': `LiteratureReader/${version}` };
if (mode === 'api') {
    headers.Accept = 'application/vnd.github+json';
    headers['X-GitHub-Api-Version'] = '2022-11-28';
} else {
    headers.Accept = 'application/octet-stream';
}

let settled = false;
const request = get(url, { headers, rejectUnauthorized: true }, (response) => {
    response.on('error', fail);
    const { statusCode: status, headers: reply } = response;
    const metadata = {
        status,
        location: reply.location,
        limited: reply['x-ratelimit-remaining'] === '0' || reply['retry-after'] !== undefined,
    };
    if (status !== 200) {
        finish(metadata, []);
        response.destroy();
        return;
    }
    const chunks = [];
    let size = 0;
    response.on('data', (chunk) => {
        size += chunk.length;
        if (size > limit) {
            fail();
            response.destroy();
            return;
        }
        chunks.push(chunk);
    });
    response.on('end', () => finish(metadata, chunks));
});
const timer = setTimeout(() => { fail(); request.destroy(); }, 8000);
request.on('error', fail);

function fail() {
    if (settled) return;
    settled = true;
    clearTimeout(timer);
    process.exitCode = 1;
}

function finish(metadata, chunks) {
    if (settled) return;
    settled = true;
    clearTimeout(timer);
    process.stdout.write(JSON.stringify(metadata) + '\n');
    for (const chunk of chunks) process.stdout.write(chunk);
}
"#;
