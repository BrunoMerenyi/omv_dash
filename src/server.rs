//! A tiny HTTP server: serves the page (built into the binary), the latest
//! snapshot as JSON, and a WebSocket that receives every new snapshot.
//!
//! Read-only on purpose: only GET/HEAD exist, and messages from browsers are
//! never read. One short-lived thread per page request; WebSocket clients
//! cost no thread at all, they are just kept in a list and written to.

use crate::config::Config;
use crate::util::{lock, round1};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const HISTORY: usize = 121; // 2 minutes of seconds, plus one point hidden left of the graph
const MAX_CLIENTS: usize = 32;

/// Everything the collectors share with the web server.
#[derive(Default)]
pub struct State {
    latest: Mutex<Arc<String>>,
    history: Mutex<History>,
    clients: Mutex<Vec<TcpStream>>,
}

#[derive(Default)]
struct History {
    cpu: VecDeque<f64>,
    down: VecDeque<f64>,
    up: VecDeque<f64>,
    disks: BTreeMap<String, VecDeque<f64>>, // IO busy % per disk
    temps: BTreeMap<String, VecDeque<f64>>, // "cpu", "board", "disk", "fan:<label>"
}

fn push(list: &mut VecDeque<f64>, value: f64) {
    list.push_back(round1(value));
    if list.len() > HISTORY {
        list.pop_front();
    }
}

impl State {
    /// Store a new snapshot and send it to every open page.
    pub fn publish(&self, snapshot: &Value) {
        {
            let mut h = lock(&self.history);
            push(&mut h.cpu, snapshot["cpu"]["percent"].as_f64().unwrap_or(0.0));
            push(&mut h.down, snapshot["network"]["down"].as_f64().unwrap_or(0.0));
            push(&mut h.up, snapshot["network"]["up"].as_f64().unwrap_or(0.0));
            for disk in snapshot["disks"]["items"].as_array().into_iter().flatten() {
                if let Some(name) = disk["name"].as_str() {
                    push(h.disks.entry(name.to_string()).or_default(), disk["busy"].as_f64().unwrap_or(0.0));
                }
            }
            let t = &snapshot["temps"];
            let mut readings = vec![("cpu".to_string(), t["cpu"].as_f64()), ("board".to_string(), t["board"].as_f64()), ("disk".to_string(), t["disk"]["temp"].as_f64())];
            for fan in t["fans"].as_array().into_iter().flatten() {
                readings.push((format!("fan:{}", fan["label"].as_str().unwrap_or("?")), fan["rpm"].as_f64()));
            }
            for (key, value) in readings {
                if let Some(value) = value {
                    push(h.temps.entry(key).or_default(), value);
                }
            }
        }
        let text = Arc::new(snapshot.to_string());
        *lock(&self.latest) = text.clone();
        let message = frame(text.as_bytes());
        // A failed write means the tab was closed: drop that client.
        lock(&self.clients).retain_mut(|client| client.write_all(&message).is_ok());
    }

    /// Graph history for a page that just opened (without the newest point,
    /// which arrives right after as a normal snapshot).
    fn history_message(&self) -> String {
        let h = lock(&self.history);
        let older = |d: &VecDeque<f64>| d.iter().take(d.len().saturating_sub(1)).copied().collect::<Vec<_>>();
        let map = |m: &BTreeMap<String, VecDeque<f64>>| -> serde_json::Map<String, Value> {
            m.iter().map(|(key, d)| (key.clone(), json!(older(d)))).collect()
        };
        json!({ "history": {
            "cpu": older(&h.cpu), "down": older(&h.down), "up": older(&h.up),
            "disks": map(&h.disks), "temps": map(&h.temps),
        } })
        .to_string()
    }
}

pub fn run(config: &Config, state: Arc<State>) {
    let listener = TcpListener::bind((config.host.as_str(), config.port)).unwrap_or_else(|e| {
        eprintln!("cannot listen on {}:{}: {e}", config.host, config.port);
        std::process::exit(1);
    });
    eprintln!("OMV Dashboard on port {} (demo mode: {})", config.port, config.demo);
    for stream in listener.incoming().flatten() {
        let state = state.clone();
        let _ = thread::Builder::new().stack_size(128 * 1024).spawn(move || {
            let _ = handle(stream, &state);
        });
    }
}

/// Static files, compiled into the binary.
struct Asset {
    path: &'static str,
    mime: &'static str,
    body: &'static [u8],
}

macro_rules! asset {
    ($path:expr, $mime:expr, $file:expr) => {
        Asset { path: $path, mime: $mime, body: include_bytes!(concat!("../static/", $file)) }
    };
}

const ASSETS: &[Asset] = &[
    asset!("/", "text/html; charset=utf-8", "index.html"),
    asset!("/index.html", "text/html; charset=utf-8", "index.html"),
    asset!("/style.css", "text/css; charset=utf-8", "style.css"),
    asset!("/app.js", "text/javascript; charset=utf-8", "app.js"),
    asset!("/fonts/JetBrainsMono.woff2", "font/woff2", "fonts/JetBrainsMono.woff2"),
    asset!("/fonts/OFL.txt", "text/plain; charset=utf-8", "fonts/OFL.txt"),
    asset!("/favicon.svg", "image/svg+xml", "favicon.svg"),
];

fn handle(mut stream: TcpStream, state: &State) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let request = read_head(&mut stream)?;
    let mut lines = request.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or("/"));
    let path = target.split('?').next().unwrap_or("/");
    let header = |name: &str| {
        lines.clone().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
        })
    };
    if method == "GET" && path == "/ws" {
        if let Some(key) = header("sec-websocket-key") {
            return websocket(stream, state, &key); // the connection stays open
        }
    }
    let head_only = method == "HEAD";
    let mut reply = |status: &str, mime: &str, body: &[u8], etag: Option<String>| {
        respond(&mut stream, status, mime, body, etag, head_only)
    };

    if method != "GET" && method != "HEAD" {
        return reply("405 Method Not Allowed", "text/plain", b"read-only\n", None);
    }
    match path {
        "/ws" => reply("400 Bad Request", "text/plain", b"expected a WebSocket\n", None),
        "/api/snapshot" => {
            let latest = lock(&state.latest).clone();
            reply("200 OK", "application/json", latest.as_bytes(), None)
        }
        "/health" => reply("200 OK", "application/json", b"{\"ok\":true}", None),
        _ => match ASSETS.iter().find(|a| a.path == path) {
            Some(asset) => {
                let etag = format!("\"{:x}\"", fnv(asset.body));
                if header("if-none-match").as_deref() == Some(etag.as_str()) {
                    reply("304 Not Modified", asset.mime, b"", Some(etag))
                } else {
                    reply("200 OK", asset.mime, asset.body, Some(etag))
                }
            }
            None => reply("404 Not Found", "text/plain", b"not found\n", None),
        },
    }
}

/// Read the request line and headers (everything up to the blank line).
fn read_head(stream: &mut TcpStream) -> io::Result<String> {
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..n]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

fn respond(stream: &mut TcpStream, status: &str, mime: &str, body: &[u8], etag: Option<String>, head_only: bool) -> io::Result<()> {
    let etag = etag.map(|e| format!("ETag: {e}\r\n")).unwrap_or_default();
    let body = if status.starts_with("304") { b"" } else { body };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{etag}\
         Cache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\n\
         Content-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )?;
    if !head_only {
        stream.write_all(body)?;
    }
    Ok(())
}

/// Finish the WebSocket handshake, send the graph history and the latest
/// snapshot, then hand the connection to `publish` for all future updates.
fn websocket(mut stream: TcpStream, state: &State, key: &str) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    )?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?; // a stuck client must not stall updates
    stream.write_all(&frame(state.history_message().as_bytes()))?;
    let latest = lock(&state.latest).clone();
    if !latest.is_empty() {
        stream.write_all(&frame(latest.as_bytes()))?;
    }
    let mut clients = lock(&state.clients);
    if clients.len() >= MAX_CLIENTS {
        clients.remove(0);
    }
    clients.push(stream);
    Ok(())
}

/// Sec-WebSocket-Accept = base64(sha1(key + fixed GUID)), see RFC 6455.
fn accept_key(key: &str) -> String {
    let digest = sha1_smol::Sha1::from(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11")).digest().bytes();
    base64(&digest)
}

/// One unmasked WebSocket text frame (servers never mask).
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x81]; // FIN + "text"
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n if n <= 0xFFFF => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    out
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() { TABLE[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Cheap content hash for ETags, so unchanged files are not sent again.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, &b| (hash ^ b as u64).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_accept_key_matches_rfc_example() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn frames_use_the_right_length_encoding() {
        assert_eq!(&frame(b"hi")[..2], [0x81, 2]);
        assert_eq!(&frame(&[0; 300])[..4], [0x81, 126, 1, 44]);
    }

    /// Start the real server on a random port and talk to it.
    fn request(state: Arc<State>, raw: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = handle(stream, &state);
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(raw.as_bytes()).unwrap();
        client.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let mut response = Vec::new();
        let _ = client.read_to_end(&mut response);
        String::from_utf8_lossy(&response).into_owned()
    }

    #[test]
    fn serves_the_page_and_refuses_writes() {
        let state = Arc::new(State::default());
        let page = request(state.clone(), "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(page.starts_with("HTTP/1.1 200 OK") && page.contains("<html"));
        assert!(request(state.clone(), "POST /api/snapshot HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 405"));
        assert!(request(state, "GET /nope HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn websocket_sends_history_then_updates() {
        let state = Arc::new(State::default());
        let disks = |busy: f64| json!({"items": [{"name": "sda", "busy": busy}]});
        state.publish(&json!({"cpu": {"percent": 10.0}, "network": {"down": 1.0, "up": 2.0}, "disks": disks(30.0), "temps": {"cpu": 50.0}}));
        state.publish(&json!({"cpu": {"percent": 20.0}, "network": {"down": 1.0, "up": 2.0}, "disks": disks(40.0), "temps": {"cpu": 55.0}}));
        let reply = request(state, "GET /ws HTTP/1.1\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n");
        assert!(reply.starts_with("HTTP/1.1 101"));
        assert!(reply.contains(r#"{"history":{"cpu":[10.0]"#));
        assert!(reply.contains(r#""disks":{"sda":[30.0]}"#));
        assert!(reply.contains(r#""temps":{"cpu":[50.0]}"#));
        assert!(reply.contains(r#""percent":20.0"#));
    }
}
