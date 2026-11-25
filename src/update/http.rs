use std::ffi::c_void;
use std::io::{self, Read};
use std::ptr::{null, null_mut};
use url::{Position, Url};
use windows_sys::Win32::Foundation::{BOOL, ERROR_INSUFFICIENT_BUFFER};
use windows_sys::Win32::Networking::WinHttp::{
    ERROR_WINHTTP_HEADER_NOT_FOUND, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_AUTHENTICATION,
    WINHTTP_DISABLE_COOKIES, WINHTTP_DISABLE_REDIRECTS, WINHTTP_FLAG_ESCAPE_DISABLE, WINHTTP_QUERY_CUSTOM,
    WINHTTP_FLAG_SECURE, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetOption, WinHttpSetTimeouts,
};

pub(super) struct Response {
    pub(super) status: u32,
    pub(super) location: Option<String>,
    // The request must close before its parent connection and session.
    request: Handle,
    _connection: Handle,
    _session: Handle,
}

impl Response {
    pub(super) fn header(&self, name: &str) -> Option<String> {
        let name = wide(name);
        text_header(&self.request, WINHTTP_QUERY_CUSTOM, name.as_ptr()).ok().flatten()
    }
}

pub(super) fn get(url: &Url, github_json: bool) -> io::Result<Response> {
    let host = url.host_str().filter(|_| {
        url.scheme() == "https" && url.port().is_none() && url.username().is_empty()
            && url.password().is_none() && url.fragment().is_none()
    }).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "更新地址必须使用 HTTPS"))?;
    let user_agent = wide(concat!("LiteratureReader/", env!("CARGO_PKG_VERSION")));
    let host = wide(host);
    let resource = wide(&url[Position::BeforePath..Position::AfterQuery]);
    let headers = wide(if github_json {
        "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n"
    } else {
        "Accept: application/octet-stream\r\n"
    });

    unsafe {
        let session = Handle::new(WinHttpOpen(user_agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0))?;
        check(WinHttpSetTimeouts(session.0, 5_000, 5_000, 8_000, 15_000))?;
        let connection = Handle::new(WinHttpConnect(session.0, host.as_ptr(), 443, 0))?;
        let request = Handle::new(WinHttpOpenRequest(connection.0, null(), resource.as_ptr(), null(), null(),
            null(), WINHTTP_FLAG_SECURE | WINHTTP_FLAG_ESCAPE_DISABLE))?;
        let disabled = WINHTTP_DISABLE_AUTHENTICATION | WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_REDIRECTS;
        check(WinHttpSetOption(request.0, WINHTTP_OPTION_DISABLE_FEATURE, (&raw const disabled).cast(), 4))?;
        check(WinHttpSendRequest(request.0, headers.as_ptr(), (headers.len() - 1) as u32, null(), 0, 0, 0))?;
        check(WinHttpReceiveResponse(request.0, null_mut()))?;

        let mut status = 0u32;
        let mut size = 4;
        check(WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(), (&raw mut status).cast(), &mut size, null_mut()))?;
        let location = if matches!(status, 301 | 302 | 303 | 307 | 308) {
            text_header(&request, WINHTTP_QUERY_LOCATION, null())?
        } else {
            None
        };
        Ok(Response { status, location, request, _connection: connection, _session: session })
    }
}

fn text_header(request: &Handle, field: u32, name: *const u16) -> io::Result<Option<String>> {
    const MAX_HEADER_BYTES: u32 = 16 * 1024;
    let mut bytes = 0;
    unsafe {
        if WinHttpQueryHeaders(request.0, field, name, null_mut(), &mut bytes, null_mut()) != 0 {
            return Ok(None);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error().map(|code| code as u32) {
            Some(ERROR_WINHTTP_HEADER_NOT_FOUND) => return Ok(None),
            Some(ERROR_INSUFFICIENT_BUFFER) if bytes <= MAX_HEADER_BYTES => {}
            _ => return Err(error),
        }
        let mut data = vec![0u16; bytes.div_ceil(2) as usize];
        check(WinHttpQueryHeaders(request.0, field, name, data.as_mut_ptr().cast(),
            &mut bytes, null_mut()))?;
        data.truncate(bytes as usize / 2);
        let text = String::from_utf16(&data).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Some(text))
    }
}

impl Read for Response {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut read = 0;
        let size = buffer.len().min(64 * 1024) as u32;
        check(unsafe { WinHttpReadData(self.request.0, buffer.as_mut_ptr().cast(), size, &mut read) })?;
        Ok(read as usize)
    }
}

struct Handle(*mut c_void);

impl Handle {
    fn new(handle: *mut c_void) -> io::Result<Self> {
        if handle.is_null() { Err(io::Error::last_os_error()) } else { Ok(Self(handle)) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn check(result: BOOL) -> io::Result<()> {
    if result == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}
