//! HTTPS transport on WinHTTP (no extra dependency, uses the system certificate store, proxy
//! settings and TLS stack). Blocking: only ever call it from the update worker thread.
//!
//! Safety properties: https only, redirects are followed manually so that every target passes
//! the host allowlist in `net`, response size and total duration are bounded, no cookies, no
//! credentials, no automatic redirect or authentication handling by WinHTTP.

use super::{
    engine::{Transport, TransportError},
    net::{self, HttpsUrl, MAX_REDIRECTS},
};
use std::{
    ffi::c_void,
    time::{Duration, Instant},
};
use windows::{
    Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_AUTHENTICATION,
        WINHTTP_DISABLE_COOKIES, WINHTTP_FLAG_SECURE, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
        WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3, WINHTTP_OPTION_DISABLE_FEATURE,
        WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
        WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
        WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
    },
    core::{PCWSTR, w},
};

const RESOLVE_TIMEOUT_MS: i32 = 10_000;
const CONNECT_TIMEOUT_MS: i32 = 10_000;
const SEND_TIMEOUT_MS: i32 = 15_000;
const RECEIVE_TIMEOUT_MS: i32 = 20_000;
/// Small responses (release list, hash file) must complete quickly.
const SMALL_DEADLINE: Duration = Duration::from_secs(30);
/// Responses above this limit count as the package download.
const SMALL_LIMIT: u64 = 1024 * 1024;
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(10 * 60);
const READ_CHUNK: usize = 64 * 1024;

/// Closes the WinHTTP handle when dropped.
struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void, what: &str) -> Result<Self, TransportError> {
        if raw.is_null() {
            Err(network(what, &windows::core::Error::from_thread()))
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful WinHTTP call and is closed exactly once.
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

fn network(what: &str, error: &windows::core::Error) -> TransportError {
    TransportError::Network(format!("{what}: {error}"))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub struct WinHttpTransport {
    session: Handle,
}

impl WinHttpTransport {
    pub fn new() -> Result<Self, TransportError> {
        let agent = wide(&format!("NexPanel-Updater/{}", env!("CARGO_PKG_VERSION")));
        // SAFETY: `agent` is NUL terminated and outlives the call; null proxy arguments are
        // valid for the automatic proxy access type.
        let raw = unsafe {
            WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        };
        let session = Handle::new(raw, "WinHTTP-Sitzung")?;
        let protocols = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        // SAFETY: the session handle is valid; buffers outlive the calls.
        unsafe {
            WinHttpSetTimeouts(
                session.0,
                RESOLVE_TIMEOUT_MS,
                CONNECT_TIMEOUT_MS,
                SEND_TIMEOUT_MS,
                RECEIVE_TIMEOUT_MS,
            )
            .map_err(|e| network("Timeouts", &e))?;
            let set_protocols = |flags: u32| {
                WinHttpSetOption(
                    Some(session.0),
                    WINHTTP_OPTION_SECURE_PROTOCOLS,
                    Some(&flags.to_ne_bytes()),
                )
            };
            // TLS 1.2 is the floor; 1.3 only where the OS knows the flag.
            if set_protocols(protocols).is_err() {
                set_protocols(WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2)
                    .map_err(|e| network("TLS-Protokolle", &e))?;
            }
        }
        Ok(Self { session })
    }

    /// One request without following redirects. Returns connection, request (drop order: request first) and the status code.
    fn request(&self, url: &HttpsUrl) -> Result<(Handle, Handle, u32), TransportError> {
        let host = wide(&url.host);
        let target = wide(&url.target);
        // SAFETY: all strings are NUL terminated and outlive the calls; handles are checked and
        // closed by `Handle::drop` (request before connection, by declaration order).
        unsafe {
            let connection = Handle::new(
                WinHttpConnect(self.session.0, PCWSTR(host.as_ptr()), 443, 0),
                "Verbindung",
            )?;
            let request = Handle::new(
                WinHttpOpenRequest(
                    connection.0,
                    w!("GET"),
                    PCWSTR(target.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    WINHTTP_FLAG_SECURE,
                ),
                "Anfrage",
            )?;
            WinHttpSetOption(
                Some(request.0),
                WINHTTP_OPTION_REDIRECT_POLICY,
                Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes()),
            )
            .map_err(|e| network("Weiterleitungsrichtlinie", &e))?;
            WinHttpSetOption(
                Some(request.0),
                WINHTTP_OPTION_DISABLE_FEATURE,
                Some(&(WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION).to_ne_bytes()),
            )
            .map_err(|e| network("Anfrageoptionen", &e))?;
            let headers: Vec<u16> = if url.host == "api.github.com" {
                "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n"
            } else {
                "Accept: application/octet-stream\r\n"
            }
            .encode_utf16()
            .collect();
            WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0)
                .map_err(|e| network("Senden", &e))?;
            WinHttpReceiveResponse(request.0, std::ptr::null_mut())
                .map_err(|e| network("Antwort", &e))?;
            let status = query_number(&request, WINHTTP_QUERY_STATUS_CODE)
                .ok_or_else(|| TransportError::Network("kein HTTP-Status".into()))?;
            Ok((connection, request, status))
        }
    }
}

/// Numeric header (status code, content length); `None` when absent or unreadable.
fn query_number(request: &Handle, which: u32) -> Option<u32> {
    let mut value = 0u32;
    let mut length = size_of::<u32>() as u32;
    // SAFETY: `value` and `length` are valid for the duration of the call and describe a
    // 4-byte buffer, as required for WINHTTP_QUERY_FLAG_NUMBER.
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            which | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut value).cast()),
            &mut length,
            std::ptr::null_mut(),
        )
    }
    .ok()
    .map(|()| value)
}

/// `Location` header, at most 4096 UTF-16 units.
fn query_location(request: &Handle) -> Option<String> {
    let mut buffer = [0u16; 4096];
    let mut length = size_of_val(&buffer) as u32;
    // SAFETY: `buffer` and `length` describe the same writable memory.
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_LOCATION,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr().cast()),
            &mut length,
            std::ptr::null_mut(),
        )
    }
    .ok()?;
    let units = (length as usize / 2).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..units]))
}

impl Transport for WinHttpTransport {
    fn fetch(
        &self,
        url: &str,
        max_bytes: u64,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Vec<u8>, TransportError> {
        let deadline = Instant::now()
            + if max_bytes > SMALL_LIMIT {
                DOWNLOAD_DEADLINE
            } else {
                SMALL_DEADLINE
            };
        let mut current = net::parse_url(url).map_err(TransportError::Rejected)?;
        for _ in 0..=MAX_REDIRECTS {
            let (_connection, request, status) = self.request(&current)?;
            match status {
                301 | 302 | 303 | 307 | 308 => {
                    let location = query_location(&request).ok_or_else(|| {
                        TransportError::Network("Weiterleitung ohne Location".into())
                    })?;
                    current = net::resolve_redirect(&current, &location)
                        .map_err(TransportError::Rejected)?;
                    continue;
                }
                200 => {}
                other => return Err(TransportError::Network(format!("HTTP {other}"))),
            }
            let total = query_number(&request, WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from);
            if total.is_some_and(|t| t > max_bytes) {
                return Err(TransportError::TooLarge);
            }
            let mut body = Vec::with_capacity(total.map_or(64 * 1024, |t| t as usize));
            let mut chunk = vec![0u8; READ_CHUNK];
            loop {
                if Instant::now() > deadline {
                    return Err(TransportError::Network("Zeitüberschreitung".into()));
                }
                let mut read = 0u32;
                // SAFETY: `chunk` is writable for READ_CHUNK bytes; `read` is a valid out value.
                unsafe {
                    WinHttpReadData(
                        request.0,
                        chunk.as_mut_ptr().cast(),
                        READ_CHUNK as u32,
                        &mut read,
                    )
                }
                .map_err(|e| network("Lesen", &e))?;
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..read as usize]);
                if body.len() as u64 > max_bytes {
                    return Err(TransportError::TooLarge);
                }
                if !progress(body.len() as u64, total) {
                    return Err(TransportError::Cancelled);
                }
            }
            if total.is_some_and(|t| t != body.len() as u64) {
                return Err(TransportError::Network(
                    "Download unvollständig (Länge weicht von Content-Length ab)".into(),
                ));
            }
            return Ok(body);
        }
        Err(TransportError::Rejected("zu viele Weiterleitungen".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs network access: a public GitHub release asset that redirects to the asset host.
    /// Run explicitly with `cargo test -- --ignored live_redirect`.
    #[test]
    #[ignore = "needs network"]
    fn live_redirect_to_asset_host() {
        let transport = WinHttpTransport::new().unwrap();
        let mut seen = 0;
        let body = transport
            .fetch(
                "https://github.com/cli/cli/releases/download/v2.40.0/gh_2.40.0_checksums.txt",
                1024 * 1024,
                &mut |n, _| {
                    seen = n;
                    true
                },
            )
            .unwrap();
        assert!(body.len() > 100 && seen == body.len() as u64);
        assert!(String::from_utf8_lossy(&body).contains("gh_2.40.0_windows_amd64.zip"));
    }

    #[test]
    #[ignore = "needs network"]
    fn live_size_limit_and_policy() {
        let transport = WinHttpTransport::new().unwrap();
        let url = "https://github.com/cli/cli/releases/download/v2.40.0/gh_2.40.0_checksums.txt";
        assert_eq!(
            transport.fetch(url, 10, &mut |_, _| true),
            Err(TransportError::TooLarge)
        );
        assert_eq!(
            transport.fetch(url, 1024 * 1024, &mut |_, _| false),
            Err(TransportError::Cancelled)
        );
        for bad in ["http://github.com/x", "https://example.com/x"] {
            assert!(matches!(
                transport.fetch(bad, 10, &mut |_, _| true),
                Err(TransportError::Rejected(_))
            ));
        }
    }
}
