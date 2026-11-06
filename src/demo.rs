//! Realistic fake data for DEMO_MODE=true, so the page can be designed on a
//! Mac. Every section has exactly the shape the real collectors produce.

use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

const GB: f64 = 1e9;
const TB: f64 = 1e12;
const GIB: f64 = 1073741824.0;

/// Tiny random number generator (xorshift), good enough for wiggly demo values.
struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

/// A value that drifts randomly between lo and hi, like a live sensor.
struct Walk {
    value: f64,
    lo: f64,
    hi: f64,
    step: f64,
}

fn walk(value: f64, lo: f64, hi: f64, step: f64) -> Walk {
    Walk { value, lo, hi, step }
}

impl Walk {
    fn next(&mut self, rng: &mut Rng) -> f64 {
        self.value = (self.value + rng.range(-self.step, self.step)).clamp(self.lo, self.hi);
        self.value
    }
}

pub struct Demo {
    rng: Rng,
    boot: u64,
    threads: Vec<Walk>,
    cpu_temp: Walk,
    board_temp: Walk,
    fan_cpu: Walk,
    fan_case: Walk,
    mem_used: Walk,
    net_down: Walk,
    net_up: Walk,
    disk_read: Walk,
    players: Walk,
    watched: f64, // demo stream progress
    load: [Walk; 3],
    containers: Vec<(&'static str, &'static str, &'static str, Option<Walk>, f64)>,
    processes: Vec<(u32, &'static str, &'static str, Walk, f64)>,
}

// name, model, size, used, temperature, power-on hours
const DISKS: &[(&str, &str, f64, f64, i64, u64)] = &[
    ("nvme0n1", "Samsung SSD 980 500GB", 500.0 * GB, 61.0 * GB, 41, 9120),
    ("sda", "WDC WD80EFZZ-68BTXN0", 8.0 * TB, 6.71 * TB, 36, 21430),
    ("sdb", "WDC WD80EFZZ-68BTXN0", 8.0 * TB, 6.64 * TB, 37, 21428),
    ("sdc", "ST8000VN004-3CP101", 8.0 * TB, 6.92 * TB, 39, 14077),
    ("sdd", "TOSHIBA MG08ADA800E", 8.0 * TB, 4.05 * TB, 34, 3312),
];

impl Default for Demo {
    fn default() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let mut rng = Rng(now.as_nanos() as u64 | 1);
        Demo {
            boot: now.as_secs() - (23 * 86400 + 5 * 3600 + 17 * 60),
            threads: (0..16).map(|_| walk(rng.range(3.0, 30.0), 0.0, 100.0, 9.0)).collect(),
            cpu_temp: walk(52.0, 44.0, 71.0, 0.6),
            board_temp: walk(36.0, 33.0, 40.0, 0.2),
            fan_cpu: walk(910.0, 780.0, 1250.0, 25.0),
            fan_case: walk(640.0, 560.0, 760.0, 10.0),
            mem_used: walk(9.8 * GIB, 8.5 * GIB, 12.0 * GIB, 0.05 * GIB),
            net_down: walk(4e6, 0.2e6, 42e6, 3e6),
            net_up: walk(1.8e6, 0.1e6, 9e6, 0.6e6),
            disk_read: walk(2.4e6, 0.8e6, 6e6, 0.4e6),
            players: walk(3.0, 0.0, 8.0, 0.15),
            watched: 0.0,
            load: [walk(3.2, 1.5, 9.0, 0.25), walk(2.9, 1.8, 6.0, 0.08), walk(2.6, 2.0, 4.0, 0.03)],
            containers: vec![
                ("jellyfin", "running", "Up 6 days", Some(walk(38.0, 12.0, 160.0, 15.0)), 1.42 * GIB),
                ("minecraft", "running", "Up 6 days (healthy)", Some(walk(14.0, 4.0, 45.0, 4.0)), 3.10 * GIB),
                ("immich_server", "running", "Up 6 days", Some(walk(2.0, 0.2, 18.0, 2.0)), 0.88 * GIB),
                ("immich_postgres", "running", "Up 6 days", Some(walk(0.6, 0.0, 4.0, 0.5)), 0.31 * GIB),
                ("nextcloud", "running", "Up 6 days", Some(walk(1.0, 0.1, 9.0, 1.0)), 0.42 * GIB),
                ("qbittorrent", "running", "Up 2 days", Some(walk(4.0, 0.5, 22.0, 3.0)), 0.27 * GIB),
                ("nas-dashboard", "running", "Up 3 hours", Some(walk(0.2, 0.1, 0.4, 0.05)), 0.004 * GIB),
                ("paperless", "exited", "Exited (0) 3 days ago", None, 0.0),
            ],
            processes: vec![
                (412233, "ffmpeg", "jellyfin", walk(140.0, 60.0, 220.0, 18.0), 2.1),
                (1883, "java", "minecraft", walk(14.0, 4.0, 45.0, 4.0), 9.6),
                (1120, "jellyfin", "jellyfin", walk(9.0, 2.0, 25.0, 3.0), 2.4),
                (911, "mergerfs", "root", walk(6.0, 1.0, 18.0, 2.0), 0.3),
                (2290, "qbittorrent-nox", "1000", walk(4.0, 0.5, 22.0, 3.0), 0.8),
                (733, "dockerd", "root", walk(1.5, 0.2, 6.0, 0.6), 0.4),
                (8814, "immich", "1000", walk(1.2, 0.0, 16.0, 2.0), 2.2),
                (1301, "smbd", "root", walk(0.8, 0.0, 9.0, 1.0), 0.1),
            ],
            rng,
        }
    }
}

impl Demo {
    pub fn build(&mut self) -> Value {
        let rng = &mut self.rng;
        let threads: Vec<u32> = self.threads.iter_mut().map(|t| t.next(rng).round() as u32).collect();
        let used = self.mem_used.next(rng);
        let busy_read = self.disk_read.next(rng);

        let disks: Vec<Value> = DISKS
            .iter()
            .map(|&(name, model, size, used, temp, hours)| {
                let busy = name == "sdc"; // the disk Jellyfin is reading from
                let idle_read = if rng.unit() < 0.25 { rng.range(5e4, 4e5) } else { 0.0 };
                let write = if rng.unit() < 0.3 { rng.range(2e4, 9e5) } else { 0.0 };
                let busy_pct = if busy { rng.range(18.0, 34.0) } else { (idle_read + write) / 2e5 };
                json!({
                    "name": name, "model": model, "size": size, "total": size, "used": used,
                    "percent": (used / size * 1000.0).round() / 10.0,
                    "temp": temp + if busy { 2 } else { 0 } + (rng.unit() < 0.25) as i64,
                    "read": if name == "sdc" { busy_read } else { idle_read },
                    "write": write.round(),
                    "busy": (busy_pct * 10.0).round() / 10.0,
                    "hours": hours, "smart": true, "bad": 0, "bad_detail": "",
                })
            })
            .collect();
        let pool_total: f64 = DISKS.iter().filter(|d| d.0.starts_with("sd")).map(|d| d.2).sum();
        let pool_used: f64 = DISKS.iter().filter(|d| d.0.starts_with("sd")).map(|d| d.3).sum();

        let containers: Vec<Value> = self
            .containers
            .iter_mut()
            .map(|(name, state, status, cpu, mem)| {
                let running = cpu.is_some();
                json!({
                    "name": name, "state": state, "status": status,
                    "cpu": cpu.as_mut().map(|w| (w.next(rng) * 10.0).round() / 10.0),
                    "mem": if running { Some(*mem as u64) } else { None },
                })
            })
            .collect();
        let mut processes: Vec<Value> = self
            .processes
            .iter_mut()
            .map(|(pid, name, user, cpu, mem)| {
                json!({ "pid": pid, "name": name, "user": user, "cpu": (cpu.next(rng) * 10.0).round() / 10.0, "mem": mem })
            })
            .collect();
        processes.sort_by(|a, b| b["cpu"].as_f64().unwrap_or(0.0).total_cmp(&a["cpu"].as_f64().unwrap_or(0.0)));

        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.watched = (self.watched + 0.05) % 100.0;
        let players = self.players.next(rng).round() as usize;
        let names = ["Steve", "Alex", "bruno", "anna", "luca", "Notch_fan", "creeper99", "mia"];
        json!({
            "host": { "hostname": "openmediavault", "ip": "192.168.1.20", "uptime": now - self.boot },
            "cpu": {
                "ok": true,
                "percent": (threads.iter().sum::<u32>() as f64 / threads.len() as f64 * 10.0).round() / 10.0,
                "threads": threads,
                "freq_mhz": rng.range(3400.0, 4550.0).round() as u64,
                "model": "AMD Ryzen 7 5700G with Radeon Graphics",
                "cores": 8,
                "load": self.load.iter_mut().map(|w| (w.next(rng) * 100.0).round() / 100.0).collect::<Vec<_>>(),
                "core_threads": (0..8).map(|i| [i, i + 8]).collect::<Vec<_>>(),
            },
            "temps": {
                "ok": true,
                "cpu": (self.cpu_temp.next(rng) * 10.0).round() / 10.0,
                "board": (self.board_temp.next(rng) * 10.0).round() / 10.0,
                "fans": [
                    { "label": "CPU fan", "rpm": self.fan_cpu.next(rng).round() },
                    { "label": "Case", "rpm": self.fan_case.next(rng).round() },
                ],
            },
            "memory": {
                "ok": true, "total": 32.0 * GIB, "used": used, "cached": 17.2 * GIB,
                "percent": (used / (32.0 * GIB) * 1000.0).round() / 10.0,
                "swap_total": 4.0 * GIB, "swap_used": 0.13 * GIB,
            },
            "network": { "ok": true, "iface": "enp5s0", "up": self.net_up.next(rng), "down": self.net_down.next(rng) },
            "pool": {
                "ok": true, "path": "/srv/mergerfs/pool", "total": pool_total, "used": pool_used,
                "free": pool_total - pool_used, "percent": (pool_used / pool_total * 1000.0).round() / 10.0,
            },
            "disks": { "ok": true, "items": disks },
            "docker": { "ok": true, "containers": containers },
            "processes": { "ok": true, "items": processes },
            "jellyfin": { "ok": true, "streams": [
                { "user": "anna", "title": "Severance · S02E03", "device": "Apple TV", "method": "DirectPlay", "bitrate": 18_400_000, "paused": false,
                  "progress": ((42.0 + self.watched) % 100.0 * 10.0).round() / 10.0, "reason": null },
                { "user": "luca", "title": "Dune: Part Two", "device": "Firefox", "method": "Transcode", "bitrate": 8_000_000, "paused": false,
                  "progress": ((71.0 + self.watched) % 100.0 * 10.0).round() / 10.0, "reason": "audio codec not supported" },
            ]},
            "minecraft": { "ok": true, "online": true, "players": players, "names": &names[..players], "max": 20, "version": "1.21.4" },
        })
    }
}
