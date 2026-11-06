//! Minecraft (Java edition) online state and player count, using the
//! "Server List Ping" the game's server browser uses. No RCON, no passwords.
//! https://minecraft.wiki/w/Java_Edition_protocol/Server_List_Ping

use crate::config::Config;
use crate::util::Res;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Minecraft's variable-length integer: 7 bits per byte, high bit = "more follows".
fn varint(value: i32) -> Vec<u8> {
    let mut v = value as u32;
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn read_varint(reader: &mut impl Read) -> Res<u32> {
    let (mut value, mut shift) = (0u32, 0);
    loop {
        let mut byte = [0u8];
        reader.read_exact(&mut byte)?;
        value |= ((byte[0] & 0x7F) as u32) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift > 28 {
            return Err("VarInt too long".into());
        }
    }
}

fn packet(id: i32, payload: &[u8]) -> Vec<u8> {
    let mut body = varint(id);
    body.extend_from_slice(payload);
    let mut out = varint(body.len() as i32);
    out.extend(body);
    out
}

/// The server's status JSON (version, players, description).
pub fn ping(host: &str, port: u16) -> Res<Value> {
    let address = (host, port).to_socket_addrs()?.next().ok_or("cannot resolve MC_HOST")?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;

    let mut handshake = varint(-1); // protocol version: "unknown"
    handshake.extend(varint(host.len() as i32));
    handshake.extend(host.as_bytes());
    handshake.extend(port.to_be_bytes());
    handshake.extend(varint(1)); // next state: status
    let mut request = packet(0x00, &handshake);
    request.extend(packet(0x00, &[])); // status request
    stream.write_all(&request)?;

    read_varint(&mut stream)?; // packet length
    read_varint(&mut stream)?; // packet id
    let length = read_varint(&mut stream)? as usize;
    if length > 1 << 20 {
        return Err("status too large".into());
    }
    let mut text = vec![0; length];
    stream.read_exact(&mut text)?;
    Ok(serde_json::from_slice(&text)?)
}

pub fn collect(config: &Config) -> Res<Value> {
    if config.mc_host.is_empty() {
        return Err("MC_HOST not set".into());
    }
    Ok(match ping(&config.mc_host, config.mc_port) {
        Ok(status) => json!({
            "online": true,
            "players": status["players"]["online"].as_u64().unwrap_or(0),
            "names": status["players"]["sample"].as_array().map(|sample| {
                sample.iter().filter_map(|p| p["name"].as_str()).collect::<Vec<_>>()
            }),
            "max": status["players"]["max"],
            "version": status["version"]["name"],
        }),
        Err(_) => json!({ "online": false, "players": 0, "max": null, "version": null }), // offline is a state, not an error
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn varint_round_trip() {
        for n in [0, 1, 127, 128, 25565, -1] {
            assert_eq!(read_varint(&mut &varint(n)[..]).unwrap(), n as u32);
        }
    }

    #[test]
    fn pings_a_fake_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 256];
            let _ = s.read(&mut buf);
            let body = br#"{"version":{"name":"1.21.4"},"players":{"online":3,"max":20}}"#;
            let mut payload = varint(0);
            payload.extend(varint(body.len() as i32));
            payload.extend(body);
            let mut msg = varint(payload.len() as i32);
            msg.extend(payload);
            s.write_all(&msg).unwrap();
        });
        let status = ping("127.0.0.1", port).unwrap();
        assert_eq!(status["players"]["online"], 3);
        assert_eq!(status["version"]["name"], "1.21.4");
    }
}
