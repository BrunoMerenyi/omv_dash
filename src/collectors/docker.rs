//! Container state, CPU and RAM from the Docker Engine API.
//!
//! DOCKER_HOST picks how to reach it:
//!   tcp://127.0.0.1:2375         the read-only socket proxy (default, see compose.yml)
//!   unix:///var/run/docker.sock  the socket itself (full API access, not recommended)
//!
//! Only two GET requests are ever sent: the container list and container stats.

use crate::config::Config;
use crate::util::{http_get, round1, Res};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Docker {
    previous: HashMap<String, (u64, u64)>, // container id -> (container CPU, system CPU)
}

fn get(config: &Config, path: &str) -> Res<Value> {
    let (status, body) = if let Some(address) = config.docker_host.strip_prefix("tcp://") {
        let address = address.to_socket_addrs()?.next().ok_or("bad DOCKER_HOST")?;
        let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        http_get(&mut stream, "docker", path, "")?
    } else {
        let socket = config.docker_host.strip_prefix("unix://").unwrap_or(&config.docker_host);
        if !Path::new(socket).exists() {
            return Err("Docker socket not found".into());
        }
        let mut stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        http_get(&mut stream, "docker", path, "")?
    };
    if status != 200 {
        return Err(format!("Docker API returned {status}").into());
    }
    Ok(serde_json::from_slice(&body)?)
}

impl Docker {
    pub fn collect(&mut self, config: &Config) -> Res<Value> {
        let list = get(config, "/containers/json?all=1")?;
        let mut containers = Vec::new();
        for c in list.as_array().ok_or("unexpected Docker response")? {
            let id = c["Id"].as_str().unwrap_or_default();
            let state = c["State"].as_str().unwrap_or_default();
            let (mut cpu, mut mem) = (None, None);
            if state == "running" {
                // one-shot: answer immediately instead of waiting for a second sample
                let stats = get(config, &format!("/containers/{id}/stats?stream=false&one-shot=true"))?;
                cpu = self.cpu_percent(id, &stats);
                mem = memory_used(&stats);
            }
            containers.push(json!({
                "name": c["Names"][0].as_str().unwrap_or("?").trim_start_matches('/'),
                "state": state,
                "status": c["Status"],
                "cpu": cpu,
                "mem": mem,
            }));
        }
        containers.sort_by_key(|c| (c["state"] != "running", c["name"].as_str().unwrap_or_default().to_string()));
        Ok(json!({ "containers": containers }))
    }

    /// Same formula as `docker stats`, using our own previous sample.
    fn cpu_percent(&mut self, id: &str, stats: &Value) -> Option<f64> {
        let cpu = &stats["cpu_stats"];
        let total = cpu["cpu_usage"]["total_usage"].as_u64()?;
        let system = cpu["system_cpu_usage"].as_u64()?;
        let previous = self.previous.insert(id.to_string(), (total, system));
        let (old_total, old_system) = previous.filter(|&(_, s)| system > s)?; // first sample: no difference yet
        let cpus = cpu["online_cpus"].as_u64().unwrap_or(1) as f64;
        Some(round1(total.saturating_sub(old_total) as f64 / (system - old_system) as f64 * cpus * 100.0))
    }
}

/// Usage minus page cache, like `docker stats` (cgroup v2 and v1).
fn memory_used(stats: &Value) -> Option<u64> {
    let memory = &stats["memory_stats"];
    let usage = memory["usage"].as_u64()?;
    let cache = memory["stats"]["inactive_file"].as_u64().or(memory["stats"]["total_inactive_file"].as_u64()).unwrap_or(0);
    Some(usage.saturating_sub(cache))
}
