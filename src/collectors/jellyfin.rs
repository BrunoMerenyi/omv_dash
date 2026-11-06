//! Active Jellyfin streams from GET /Sessions (needs an API key).

use crate::config::Config;
use crate::util::{http_get, Res};
use serde_json::{json, Value};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub fn collect(config: &Config) -> Res<Value> {
    if config.jellyfin_api_key.is_empty() {
        return Err("JELLYFIN_API_KEY not set".into());
    }
    let rest = config.jellyfin_url.strip_prefix("http://").ok_or("JELLYFIN_URL must start with http://")?;
    let (host, base) = rest.split_once('/').map_or((rest, ""), |(h, b)| (h, b.trim_end_matches('/')));
    let address = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    let address = address.to_socket_addrs()?.next().ok_or("cannot resolve JELLYFIN_URL")?;

    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let path = if base.is_empty() { "/Sessions?activeWithinSeconds=120".to_string() } else { format!("/{base}/Sessions?activeWithinSeconds=120") };
    let auth = format!("Authorization: MediaBrowser Token=\"{}\"\r\n", config.jellyfin_api_key);
    let (status, body) = http_get(&mut stream, host, &path, &auth)?;
    if status != 200 {
        return Err(format!("Jellyfin returned {status}").into());
    }
    Ok(json!({ "streams": streams(&serde_json::from_slice(&body)?) }))
}

/// Sessions that are actually playing something.
pub fn streams(sessions: &Value) -> Vec<Value> {
    sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["NowPlayingItem"].is_object())
        .map(|s| {
            let item = &s["NowPlayingItem"];
            let transcoding = &s["TranscodingInfo"];
            let method = s["PlayState"]["PlayMethod"].as_str().unwrap_or(if transcoding.is_object() { "Transcode" } else { "DirectPlay" });
            json!({
                "user": s["UserName"].as_str().unwrap_or("?"),
                "device": s["DeviceName"].as_str().or(s["Client"].as_str()).unwrap_or("?"),
                "title": title(item),
                "method": method,
                "bitrate": transcoding["Bitrate"].as_u64().or_else(|| source_bitrate(item)),
                "paused": s["PlayState"]["IsPaused"].as_bool().unwrap_or(false),
                "progress": progress(s),
                "reason": transcoding["TranscodeReasons"].as_array().map(|reasons| {
                    reasons.iter().filter_map(Value::as_str).map(words).collect::<Vec<_>>().join(", ")
                }),
            })
        })
        .collect()
}

/// How far into the item the stream is, in percent.
fn progress(session: &Value) -> Option<f64> {
    let position = session["PlayState"]["PositionTicks"].as_f64()?;
    let length = session["NowPlayingItem"]["RunTimeTicks"].as_f64().filter(|&l| l > 0.0)?;
    Some((position / length * 1000.0).round().clamp(0.0, 1000.0) / 10.0)
}

/// "AudioCodecNotSupported" -> "audio codec not supported".
fn words(code: &str) -> String {
    let mut out = String::new();
    for (i, c) in code.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn title(item: &Value) -> String {
    match item["SeriesName"].as_str() {
        Some(series) => format!(
            "{series} · S{:02}E{:02}",
            item["ParentIndexNumber"].as_u64().unwrap_or(0),
            item["IndexNumber"].as_u64().unwrap_or(0)
        ),
        None => item["Name"].as_str().unwrap_or("?").to_string(),
    }
}

/// Bitrate of the original file when nothing is transcoded.
fn source_bitrate(item: &Value) -> Option<u64> {
    item["MediaStreams"].as_array()?.iter().find(|s| s["Type"] == "Video")?["BitRate"].as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_sessions_into_streams() {
        let sessions = json!([
            {"UserName": "anna", "DeviceName": "Apple TV",
             "NowPlayingItem": {"SeriesName": "Severance", "ParentIndexNumber": 2, "IndexNumber": 3,
                                "MediaStreams": [{"Type": "Video", "BitRate": 18000000}]},
             "PlayState": {"PlayMethod": "DirectPlay"}},
            {"UserName": "luca", "DeviceName": "Firefox", "NowPlayingItem": {"Name": "Dune", "RunTimeTicks": 1000},
             "PlayState": {"PlayMethod": "Transcode", "PositionTicks": 250},
             "TranscodingInfo": {"Bitrate": 8000000, "TranscodeReasons": ["AudioCodecNotSupported"]}},
            {"UserName": "idle", "DeviceName": "Phone"}
        ]);
        let s = streams(&sessions);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0]["title"], "Severance · S02E03");
        assert_eq!(s[0]["bitrate"], 18000000);
        assert_eq!(s[1]["method"], "Transcode");
        assert_eq!(s[1]["bitrate"], 8000000);
        assert_eq!(s[1]["progress"], 25.0);
        assert_eq!(s[1]["reason"], "audio codec not supported");
    }
}
