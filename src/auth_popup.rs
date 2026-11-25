use crate::translation::official_login_url;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use windows_sys::Win32::UI::Shell::{ASSOCF_IS_PROTOCOL, ASSOCSTR_EXECUTABLE, AssocQueryStringW};

pub fn open(url: &str) -> Result<(), String> {
    let url = official_login_url(url).ok_or_else(|| "CodeBuddy 授权地址无效。".to_owned())?;
    let browser = default_browser().ok_or_else(|| "无法找到 Windows 的默认浏览器。".to_owned())?;
    if !supports_compact_window(&browser) {
        return Err("当前默认浏览器无法打开登录小窗。".into());
    }
    Command::new(browser)
        .arg(format!("--app={url}"))
        .arg("--window-size=520,700")
        .spawn()
        .map_err(|error| format!("无法打开登录小窗：{error}"))?;
    Ok(())
}

fn supports_compact_window(browser: &Path) -> bool {
    let name = browser.file_name().and_then(|name| name.to_str());
    name.is_some_and(|name| matches!(name.to_ascii_lowercase().as_str(),
        "msedge.exe" | "chrome.exe" | "chromium.exe" | "brave.exe" | "vivaldi.exe"))
}

fn default_browser() -> Option<PathBuf> {
    let https: Vec<_> = "https".encode_utf16().chain([0]).collect();
    let mut length = 0u32;
    // An initial call with no output buffer reports the required character count.
    unsafe {
        AssocQueryStringW(ASSOCF_IS_PROTOCOL, ASSOCSTR_EXECUTABLE, https.as_ptr(), null(), null_mut(), &mut length);
    }
    if !(2..=32_768).contains(&length) {
        return None;
    }
    let mut path = vec![0u16; length as usize];
    let result = unsafe {
        AssocQueryStringW(ASSOCF_IS_PROTOCOL, ASSOCSTR_EXECUTABLE, https.as_ptr(), null(), path.as_mut_ptr(), &mut length)
    };
    if result != 0 {
        return None;
    }
    let end = path.iter().position(|&character| character == 0)?;
    let path = PathBuf::from(OsString::from_wide(&path[..end]));
    (path.is_absolute() && path.is_file()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_compact_mode_only_in_supported_browsers() {
        assert!(supports_compact_window(Path::new(r"C:\Program Files\Google\Chrome\Chrome.EXE")));
        assert!(supports_compact_window(Path::new("msedge.exe")));
        assert!(!supports_compact_window(Path::new("firefox.exe")));
    }
}
