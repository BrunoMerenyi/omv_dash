//! Small helpers shared by the whole program.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

/// Any error. Its text ends up on the page, e.g. "Docker socket not found".
pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Read a small text file from /proc or /sys, trimmed. None if missing.
pub fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn read_num<T: FromStr>(path: impl AsRef<Path>) -> Option<T> {
    read(path)?.parse().ok()
}

pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Lock a mutex, even if another thread panicked while holding it.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Run one collector. Errors (and even panics) become
/// {"ok": false, "error": "..."}, so one broken sensor never breaks the page.
pub fn safe(name: &str, collect: impl FnOnce() -> Res<Value>) -> Value {
    let error = match catch_unwind(AssertUnwindSafe(collect)) {
        Ok(Ok(mut value)) => {
            if let Some(map) = value.as_object_mut() {
                map.insert("ok".into(), true.into());
            }
            return value;
        }
        Ok(Err(e)) => e.to_string(),
        Err(_) => "internal error".to_string(),
    };
    log_once(name, &error);
    json!({ "ok": false, "error": error })
}

/// Placeholder until a slow collector has run for the first time.
pub fn pending() -> Value {
    json!({ "ok": false, "pending": true, "error": "Loading …" })
}

/// Print each distinct problem only once instead of every second.
pub fn log_once(name: &str, message: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let line = format!("{name}: {message}");
    if lock(SEEN.get_or_init(Default::default)).insert(line.clone()) {
        eprintln!("WARN {line}");
    }
}

/// Turns ever-growing counters (bytes sent, sectors read …) into per-second rates.
#[derive(Default)]
pub struct Rate {
    last: HashMap<String, (Instant, u64)>,
}

impl Rate {
    pub fn per_second(&mut self, key: &str, value: u64) -> f64 {
        let now = Instant::now();
        match self.last.insert(key.to_string(), (now, value)) {
            Some((then, old)) if value >= old => {
                let seconds = now.duration_since(then).as_secs_f64();
                if seconds > 0.0 { (value - old) as f64 / seconds } else { 0.0 }
            }
            _ => 0.0, // first sample, or the counter was reset
        }
    }
}

/// Minimal HTTP/1.1 GET over any stream (TCP or Unix socket).
/// Returns the status code and the body. `extra` holds additional header lines.
pub fn http_get(stream: &mut (impl Read + Write), host: &str, path: &str, extra: &str) -> Res<(u16, Vec<u8>)> {
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{extra}\r\n")?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let end = find(&raw, b"\r\n\r\n").ok_or("bad HTTP response")?;
    let head = String::from_utf8_lossy(&raw[..end]).to_lowercase();
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("bad HTTP status")?;
    let body = &raw[end + 4..];
    let body = if head.contains("transfer-encoding: chunked") { dechunk(body)? } else { body.to_vec() };
    Ok((status, body))
}

/// Undo HTTP "chunked" transfer encoding: "<hex size>\r\n<data>\r\n" … "0\r\n".
pub fn dechunk(mut data: &[u8]) -> Res<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = find(data, b"\r\n").ok_or("bad chunk")?;
        let size_text = String::from_utf8_lossy(&data[..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)?;
        if size == 0 {
            return Ok(out);
        }
        let start = line_end + 2;
        out.extend_from_slice(data.get(start..start + size).ok_or("short chunk")?);
        data = data.get(start + size + 2..).ok_or("short chunk")?;
    }
}

pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// `docker stop` sends SIGTERM. As PID 1 in a container nothing handles it by
/// default, so Docker would wait 10 s and then kill us. Exit right away instead.
pub fn exit_on_sigterm() {
    extern "C" fn on_signal(_: libc::c_int) {
        unsafe { libc::_exit(0) }
    }
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    unsafe {
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dechunks_http_body() {
        assert_eq!(dechunk(b"4\r\nWiki\r\n6\r\npedia \r\n0\r\n\r\n").unwrap(), b"Wikipedia ");
    }

    #[test]
    fn safe_turns_errors_and_panics_into_not_ok() {
        assert_eq!(safe("t", || Ok(json!({"a": 1}))), json!({"a": 1, "ok": true}));
        assert_eq!(safe("t", || Err("broken".into())), json!({"ok": false, "error": "broken"}));
        assert_eq!(safe("t", || panic!("boom"))["ok"], false);
    }
}
