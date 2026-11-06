//! Realistic fake data for DEMO_MODE=true, so the page can be designed on a
//! Mac. Every section has exactly the shape the real collectors produce.

use serde_json::{json, Value};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    started: Instant,
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
            started: Instant::now(),
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
                (1, "systemd", "root", walk(0.1, 0.0, 0.5, 0.1), 0.04),
                (612, "sshd", "root", walk(0.0, 0.0, 0.2, 0.05), 0.03),
                (655, "cron", "root", walk(0.0, 0.0, 0.1, 0.02), 0.01),
                (702, "tailscaled", "root", walk(0.3, 0.0, 2.0, 0.3), 0.15),
                (5120, "nas-dashboard", "10001", walk(0.2, 0.1, 0.4, 0.05), 0.0),
                (690, "containerd", "root", walk(0.6, 0.1, 3.0, 0.4), 0.2),
                (733001, "kworker/2:1", "root", walk(0.0, 0.0, 0.3, 0.05), 0.0),
                (1450, "php-fpm8.4", "999", walk(0.4, 0.0, 4.0, 0.6), 0.1),
            ],
            rng,
        }
    }
}

/// Blend from a normal value `a` towards the maxed-out value `b` (s = 0..1).
fn mix(a: f64, b: f64, s: f64) -> f64 {
    a + (b - a) * s
}

impl Demo {
    /// A load spike once a minute (ramp up 4 s, hold 12 s, ramp down 4 s), so
    /// the maxed-out look can be seen without being stuck in it.
    fn stress(&self) -> f64 {
        let t = self.started.elapsed().as_secs_f64() % 60.0;
        if t < 38.0 { 0.0 } else if t < 42.0 { (t - 38.0) / 4.0 } else if t < 54.0 { 1.0 } else if t < 58.0 { (58.0 - t) / 4.0 } else { 0.0 }
    }

    pub fn build(&mut self) -> Value {
        let s = self.stress();
        let rng = &mut self.rng;
        let threads: Vec<u32> = self.threads.iter_mut().map(|t| mix(t.next(rng), rng.range(88.0, 100.0), s).round() as u32).collect();
        let used = mix(self.mem_used.next(rng), 29.6 * GIB, s);
        let busy_read = mix(self.disk_read.next(rng), 180e6, s);

        let disks: Vec<Value> = DISKS
            .iter()
            .map(|&(name, model, size, used, temp, hours)| {
                let busy = name == "sdc"; // the disk Jellyfin is reading from
                let idle_read = if rng.unit() < 0.25 { rng.range(5e4, 4e5) } else { 0.0 };
                let write = if rng.unit() < 0.3 { rng.range(2e4, 9e5) } else { 0.0 };
                let busy_pct = mix(if busy { rng.range(18.0, 34.0) } else { (idle_read + write) / 2e5 }, if busy { 100.0 } else { rng.range(55.0, 95.0) }, s);
                let (idle_read, write) = (mix(idle_read, 60e6, s), mix(write, 35e6, s));
                json!({
                    "name": name, "model": model, "size": size, "total": size, "used": used,
                    "percent": (used / size * 1000.0).round() / 10.0,
                    "temp": temp + if busy { 2 } else { 0 } + (rng.unit() < 0.25) as i64 + mix(0.0, if busy { 18.0 } else { 10.0 }, s).round() as i64,
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
                    "cpu": cpu.as_mut().map(|w| (w.next(rng) * (1.0 + 3.0 * s) * 10.0).round() / 10.0),
                    "mem": if running { Some(*mem as u64) } else { None },
                })
            })
            .collect();
        let processes: Vec<Value> = self
            .processes
            .iter_mut()
            .map(|(pid, name, user, cpu, mem)| {
                json!({ "pid": pid, "name": name, "user": user, "cpu": (cpu.next(rng) * (1.0 + 2.5 * s) * 10.0).round() / 10.0, "mem": mem })
            })
            .collect();
        // Highest and lowest 8 by CPU and by RAM, like the real collector sends.
        let order = |key: &str| {
            let mut sorted: Vec<&Value> = processes.iter().collect();
            sorted.sort_by(|a, b| b[key].as_f64().unwrap_or(0.0).total_cmp(&a[key].as_f64().unwrap_or(0.0)));
            let pids = |list: Vec<&&Value>| list.iter().map(|p| p["pid"].clone()).collect::<Vec<_>>();
            json!({ "desc": pids(sorted.iter().take(8).collect()), "asc": pids(sorted.iter().rev().take(8).collect()) })
        };
        let process_order = json!({ "cpu": order("cpu"), "mem": order("mem") });

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
                "freq_mhz": mix(rng.range(3400.0, 4550.0), 4650.0, s).round() as u64,
                "model": "AMD Ryzen 7 5700G with Radeon Graphics",
                "cores": 8,
                "load": self.load.iter_mut().zip([15.6, 9.0, 5.0]).map(|(w, max)| (mix(w.next(rng), max, s) * 100.0).round() / 100.0).collect::<Vec<_>>(),
                "core_threads": (0..8).map(|i| [i, i + 8]).collect::<Vec<_>>(),
            },
            "temps": {
                "ok": true,
                "cpu": (mix(self.cpu_temp.next(rng), 93.0, s) * 10.0).round() / 10.0,
                "board": (mix(self.board_temp.next(rng), 49.0, s) * 10.0).round() / 10.0,
                "fans": [
                    { "label": "CPU fan", "rpm": mix(self.fan_cpu.next(rng), 2100.0, s).round() },
                    { "label": "Case", "rpm": mix(self.fan_case.next(rng), 1450.0, s).round() },
                ],
            },
            "memory": {
                "ok": true, "total": 32.0 * GIB, "used": used, "cached": mix(17.2, 1.8, s) * GIB,
                "percent": (used / (32.0 * GIB) * 1000.0).round() / 10.0,
                "swap_total": 4.0 * GIB, "swap_used": mix(0.13, 2.6, s) * GIB,
            },
            "network": { "ok": true, "iface": "enp5s0", "up": mix(self.net_up.next(rng), 38e6, s), "down": mix(self.net_down.next(rng), 117e6, s) },
            "pool": {
                "ok": true, "path": "/srv/mergerfs/pool", "total": pool_total, "used": pool_used,
                "free": pool_total - pool_used, "percent": (pool_used / pool_total * 1000.0).round() / 10.0,
            },
            "disks": { "ok": true, "items": disks },
            "docker": { "ok": true, "containers": containers },
            "processes": { "ok": true, "items": processes, "order": process_order },
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

/// Fake details for the demo's info box.
#[allow(clippy::type_complexity)] // a plain table is the clearest way to write demo data
pub fn process_details(pid: u32) -> Option<Value> {
    // pid, name, user, state, parent pid + name, threads, nice, RSS, virtual, running for, command
    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    let table: &[(u32, &str, &str, &str, u32, &str, u64, i64, u64, u64, u64, &str)] = &[
        (412233, "ffmpeg", "jellyfin", "running", 1120, "jellyfin", 14, 0, 312 * MIB, 2 * GIB, 1460,
         "/usr/lib/jellyfin-ffmpeg/ffmpeg -analyzeduration 200M -i file:/data/media/movies/Dune Part Two (2024)/Dune.mkv -map_metadata -1 -codec:v:0 libx264 -preset veryfast -crf 23 -maxrate 8000000 -codec:a:0 aac -ac 2 -f hls /cache/transcodes/7f3a.m3u8"),
        (1883, "java", "minecraft", "sleeping", 1850, "crafty", 87, 0, 3 * GIB, 13 * GIB, 518400,
         "java -Xms10G -Xmx10G -XX:+UseG1GC -XX:+ParallelRefProcEnabled -jar server.jar nogui"),
        (1120, "jellyfin", "jellyfin", "sleeping", 1098, "containerd-shim", 42, 0, 780 * MIB, 4 * GIB, 518400,
         "/jellyfin/jellyfin --datadir /config --cachedir /cache --ffmpeg /usr/lib/jellyfin-ffmpeg/ffmpeg"),
        (911, "mergerfs", "root", "sleeping", 1, "systemd", 6, 0, 90 * MIB, 600 * MIB, 2006000,
         "mergerfs pool:596a9dc6-f833-4973-a553 /srv/mergerfs/pool -o rw,branches=/srv/dev-disk-by-uuid-c632c2a4:/srv/dev-disk-by-uuid-8157dc73"),
        (2290, "qbittorrent-nox", "1000", "sleeping", 2250, "containerd-shim", 18, 0, 250 * MIB, MIB * 1800, 172800,
         "/app/qbittorrent-nox --webui-port=8080 --profile=/config"),
        (733, "dockerd", "root", "sleeping", 1, "systemd", 31, 0, 120 * MIB, 3 * GIB, 2006000,
         "/usr/bin/dockerd -H fd:// --containerd=/run/containerd/containerd.sock"),
        (8814, "immich", "1000", "sleeping", 8790, "containerd-shim", 23, 0, 700 * MIB, 3 * GIB, 518400, "immich"),
        (1301, "smbd", "root", "sleeping", 1, "systemd", 1, 0, 30 * MIB, 200 * MIB, 2006000, "/usr/sbin/smbd --foreground --no-process-group"),
        (1, "systemd", "root", "sleeping", 0, "", 1, 0, 12 * MIB, 160 * MIB, 2006000, "/sbin/init"),
        (612, "sshd", "root", "sleeping", 1, "systemd", 1, 0, 8 * MIB, 15 * MIB, 2006000, "sshd: /usr/sbin/sshd -D [listener] 0 of 10-100 startups"),
        (655, "cron", "root", "sleeping", 1, "systemd", 1, 0, 3 * MIB, 7 * MIB, 2006000, "/usr/sbin/cron -f"),
        (702, "tailscaled", "root", "sleeping", 680, "containerd-shim", 19, 0, 50 * MIB, MIB * 1300, 2006000,
         "tailscaled --socket=/tmp/tailscaled.sock --statedir=/var/lib/tailscale --authkey=tskey-auth-k1234 --tun=userspace-networking"),
        (5120, "nas-dashboard", "10001", "sleeping", 5100, "containerd-shim", 7, 0, MIB, 6 * MIB, 10800, "/nas-dashboard"),
        (690, "containerd", "root", "sleeping", 1, "systemd", 24, 0, 60 * MIB, 2 * GIB, 2006000, "/usr/bin/containerd"),
        (733001, "kworker/2:1", "root", "idle", 2, "kthreadd", 1, 0, 0, 0, 3400, ""),
        (1450, "php-fpm8.4", "999", "sleeping", 1440, "php-fpm8.4", 1, 0, 40 * MIB, 300 * MIB, 2006000, "php-fpm: pool www"),
    ];
    let &(pid, name, user, state, ppid, parent, threads, nice, rss, virt, running_for, command) = table.iter().find(|row| row.0 == pid)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Some(json!({
        "pid": pid, "name": name, "user": user, "state": state, "ppid": ppid,
        "parent": if parent.is_empty() { None } else { Some(parent) },
        "nice": nice, "threads": threads, "rss": rss, "virt": virt,
        "started": now - running_for, "running_for": running_for,
        "command": if command.is_empty() { None } else { Some(crate::collectors::processes::redact(command)) },
    }))
}
