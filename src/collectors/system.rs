//! CPU, memory, network, load and host info from /proc.

use crate::config::Config;
use crate::util::{read, read_num, round1, Rate, Res};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::net::UdpSocket;

/// Interfaces that are not real network cards.
const SKIP_IFACES: &[&str] = &["lo", "docker", "br-", "veth", "virbr", "tun", "tap", "wg", "tailscale"];

#[derive(Default)]
pub struct System {
    previous: Vec<(u64, u64)>,     // per thread: (busy, total) jiffies
    core_threads: Vec<Vec<usize>>, // e.g. [[0, 8], [1, 9], …]
    model: Option<String>,
    network: Rate,
}

impl System {
    pub fn cpu(&mut self) -> Res<Value> {
        let times = parse_cpu_times(&fs::read_to_string("/proc/stat")?);
        if times.is_empty() {
            return Err("no CPU data".into());
        }
        // Usage = share of "busy" time since the previous second.
        let threads: Vec<f64> = times
            .iter()
            .enumerate()
            .map(|(i, &(busy, total))| match self.previous.get(i) {
                Some(&(b, t)) if total > t => busy.saturating_sub(b) as f64 / (total - t) as f64 * 100.0,
                _ => 0.0,
            })
            .collect();
        self.previous = times;

        if self.core_threads.iter().map(Vec::len).sum::<usize>() != threads.len() {
            self.core_threads = core_threads(threads.len()); // first run (topology never changes)
            self.model = cpu_model();
        }
        let percent = threads.iter().sum::<f64>() / threads.len() as f64;
        Ok(json!({
            "percent": round1(percent),
            "threads": threads.iter().map(|t| t.round() as u32).collect::<Vec<_>>(),
            "freq_mhz": freq_mhz(threads.len()),
            "model": self.model,
            "cores": self.core_threads.len(),
            "load": load(),
            "core_threads": self.core_threads,
        }))
    }

    pub fn network(&mut self, config: &Config) -> Res<Value> {
        // Lines look like "  eth0: <rx bytes> … (8 values) <tx bytes> …"
        let text = fs::read_to_string("/proc/net/dev")?;
        let counters: Vec<(&str, u64, u64)> = text
            .lines()
            .skip(2)
            .filter_map(|line| {
                let (name, values) = line.split_once(':')?;
                let v: Vec<u64> = values.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                Some((name.trim(), *v.first()?, *v.get(8)?))
            })
            .collect();
        let chosen = if config.net_iface.is_empty() {
            counters
                .iter()
                .filter(|(name, ..)| !SKIP_IFACES.iter().any(|skip| name.starts_with(skip)))
                .max_by_key(|(_, rx, tx)| rx + tx)
        } else {
            counters.iter().find(|(name, ..)| *name == config.net_iface)
        };
        let &(iface, rx, tx) = chosen.ok_or("network interface not found")?;
        Ok(json!({
            "iface": iface,
            "up": self.network.per_second(&format!("{iface}:up"), tx).round(),
            "down": self.network.per_second(&format!("{iface}:down"), rx).round(),
        }))
    }
}

/// Every "cpuN" line of /proc/stat as (busy, total) jiffies.
pub fn parse_cpu_times(stat: &str) -> Vec<(u64, u64)> {
    stat.lines()
        .filter(|line| line.starts_with("cpu") && line.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .map(|line| {
            // user nice system idle iowait irq softirq steal
            let v: Vec<u64> = line.split_whitespace().skip(1).take(8).filter_map(|x| x.parse().ok()).collect();
            let total: u64 = v.iter().sum();
            let idle = v.get(3).unwrap_or(&0) + v.get(4).unwrap_or(&0);
            (total.saturating_sub(idle), total)
        })
        .collect()
}

/// Threads grouped by physical core, from the kernel's CPU topology.
fn core_threads(count: usize) -> Vec<Vec<usize>> {
    let mut cores: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for i in 0..count {
        let topology = format!("/sys/devices/system/cpu/cpu{i}/topology");
        match (read_num(format!("{topology}/physical_package_id")), read_num(format!("{topology}/core_id"))) {
            (Some(package), Some(core)) => cores.entry((package, core)).or_default().push(i),
            _ => return (0..count).map(|i| vec![i]).collect(), // unknown: one thread per row
        }
    }
    let mut groups: Vec<Vec<usize>> = cores.into_values().collect();
    groups.sort_by_key(|g| g[0]);
    groups
}

fn cpu_model() -> Option<String> {
    let info = fs::read_to_string("/proc/cpuinfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("model name"))?;
    Some(line.split_once(':')?.1.trim().to_string())
}

/// Average current clock of all threads.
fn freq_mhz(count: usize) -> Option<u64> {
    let khz: Vec<u64> = (0..count)
        .filter_map(|i| read_num(format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq")))
        .collect();
    if khz.is_empty() {
        return None;
    }
    Some(khz.iter().sum::<u64>() / khz.len() as u64 / 1000)
}

/// Load average over 1, 5 and 15 minutes.
fn load() -> Option<Vec<f64>> {
    let text = read("/proc/loadavg")?;
    let values: Vec<f64> = text.split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
    (values.len() == 3).then_some(values)
}

pub fn memory() -> Res<Value> {
    let info = fs::read_to_string("/proc/meminfo")?;
    let bytes = |key: &str| -> u64 {
        info.lines()
            .find(|l| l.strip_prefix(key).is_some_and(|rest| rest.starts_with(':')))
            .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
            .map_or(0, |kib: u64| kib * 1024)
    };
    let total = bytes("MemTotal");
    if total == 0 {
        return Err("no memory data".into());
    }
    let cached = bytes("Cached") + bytes("SReclaimable");
    let buffers = bytes("Buffers");
    let used = total.saturating_sub(bytes("MemFree") + buffers + cached);
    let available = bytes("MemAvailable");
    Ok(json!({
        "total": total,
        "used": used,
        "cached": cached + buffers,
        "percent": round1(total.saturating_sub(available) as f64 / total as f64 * 100.0),
        "swap_total": bytes("SwapTotal"),
        "swap_used": bytes("SwapTotal").saturating_sub(bytes("SwapFree")),
    }))
}

pub fn host(config: &Config) -> Value {
    let uptime = read("/proc/uptime").and_then(|t| t.split_whitespace().next()?.parse::<f64>().ok());
    let ip = if config.host_ip.is_empty() { lan_ip() } else { Some(config.host_ip.clone()) };
    json!({
        "hostname": read("/proc/sys/kernel/hostname"),
        "ip": ip,
        "uptime": uptime.map(|u| u as u64),
    })
}

/// "Connecting" a UDP socket sends nothing, but tells us which local IP
/// the system would use to reach the LAN.
fn lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("10.255.255.255:1").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_per_thread_cpu_times() {
        let stat = "cpu  10 0 10 80 0 0 0 0\ncpu0 5 0 5 40 0 0 0 0 0 0\ncpu1 5 0 5 30 10 0 0 0\nintr 1\n";
        assert_eq!(parse_cpu_times(stat), vec![(10, 50), (10, 50)]);
    }
}
