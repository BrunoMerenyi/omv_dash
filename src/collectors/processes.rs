//! Processes from /proc/<pid> (needs `pid: host`).
//!
//! Every scan sends the top and bottom N by CPU and by RAM, so the page can
//! sort either way. Details for one process (command line, threads, parent …)
//! are read on demand via `details()` when someone opens the info box.

use crate::util::{read, round1, Res};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Facts about the system that never change while we run.
struct System {
    users: HashMap<u32, String>, // uid -> name, from /etc/passwd if mounted
    ticks_per_second: f64,
    page_size: u64,
}

fn system() -> &'static System {
    static SYSTEM: OnceLock<System> = OnceLock::new();
    SYSTEM.get_or_init(|| System {
        users: fs::read_to_string("/etc/passwd")
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                Some((f.get(2)?.parse().ok()?, f[0].to_string()))
            })
            .collect(),
        ticks_per_second: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64,
        page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
    })
}

pub struct Processes {
    previous: HashMap<u32, u64>, // pid -> CPU ticks at the last scan
    last_scan: Instant,
}

impl Default for Processes {
    fn default() -> Self {
        Processes { previous: HashMap::new(), last_scan: Instant::now() }
    }
}

struct Proc {
    pid: u32,
    name: String,
    cpu: f64,
    rss: u64,
}

impl Processes {
    pub fn collect(&mut self, top: usize) -> Res<Value> {
        let sys = system();
        let elapsed = self.last_scan.elapsed().as_secs_f64().max(0.001);
        self.last_scan = Instant::now();
        let mut ticks_now = HashMap::new();
        let mut all = Vec::new();

        for entry in fs::read_dir("/proc")? {
            let Some(pid) = entry?.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else { continue }; // already gone
            let Some((name, fields)) = split_stat(&stat) else { continue };
            let num = |n: usize| field(&fields, n);
            let ticks = num(14) + num(15); // utime + stime
            let cpu = self.previous.get(&pid).map_or(0.0, |&before| {
                ticks.saturating_sub(before) as f64 / sys.ticks_per_second / elapsed * 100.0
            });
            ticks_now.insert(pid, ticks);
            all.push(Proc { pid, name: name.to_string(), cpu, rss: num(24) * sys.page_size });
        }
        self.previous = ticks_now;

        // The N highest and lowest by CPU and by RAM (ties: lower pid first).
        let ranked = |key: fn(&Proc) -> f64| -> (Vec<u32>, Vec<u32>) {
            let mut sorted: Vec<&Proc> = all.iter().collect();
            sorted.sort_by(|a, b| key(b).total_cmp(&key(a)).then(a.pid.cmp(&b.pid)));
            let desc = sorted.iter().take(top).map(|p| p.pid).collect();
            let asc = sorted.iter().rev().take(top).map(|p| p.pid).collect();
            (desc, asc)
        };
        let (cpu_desc, cpu_asc) = ranked(|p| p.cpu);
        let (mem_desc, mem_asc) = ranked(|p| p.rss as f64);

        let wanted: BTreeSet<u32> = [&cpu_desc, &cpu_asc, &mem_desc, &mem_asc].into_iter().flatten().copied().collect();
        let mem_total = mem_total_bytes().max(1) as f64;
        let items: Vec<Value> = all
            .iter()
            .filter(|p| wanted.contains(&p.pid))
            .map(|p| {
                json!({
                    "pid": p.pid,
                    "name": p.name,
                    "user": user(p.pid),
                    "cpu": round1(p.cpu),
                    "mem": round1(p.rss as f64 / mem_total * 100.0),
                })
            })
            .collect();
        Ok(json!({
            "items": items,
            "order": { "cpu": { "desc": cpu_desc, "asc": cpu_asc }, "mem": { "desc": mem_desc, "asc": mem_asc } },
        }))
    }
}

/// Everything the info box shows for one process, read right now.
pub fn details(pid: u32) -> Option<Value> {
    let sys = system();
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (name, fields) = split_stat(&stat)?;
    let num = |n: usize| field(&fields, n);
    let state = match fields.first().copied().unwrap_or("?") {
        "R" => "running",
        "S" => "sleeping",
        "D" => "waiting for disk",
        "Z" => "zombie",
        "T" | "t" => "stopped",
        "I" => "idle",
        other => other,
    };
    let ppid = num(4) as u32;
    let parent = fs::read_to_string(format!("/proc/{ppid}/stat")).ok().and_then(|s| split_stat(&s).map(|(n, _)| n.to_string()));
    // The command line is NUL-separated; kernel threads have none.
    let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok().map(|raw| {
        String::from_utf8_lossy(&raw).split('\0').filter(|a| !a.is_empty()).collect::<Vec<_>>().join(" ")
    });
    let boot = read("/proc/stat")
        .and_then(|s| s.lines().find(|l| l.starts_with("btime"))?.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or(0);
    let started = boot + (num(22) as f64 / sys.ticks_per_second) as u64;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Some(json!({
        "pid": pid,
        "name": name,
        "user": user(pid),
        "state": state,
        "ppid": ppid,
        "parent": parent,
        "nice": fields.get(16).and_then(|v| v.parse::<i64>().ok()),
        "threads": num(20),
        "rss": num(24) * sys.page_size,
        "virt": num(23),
        "started": started,
        "running_for": now.saturating_sub(started),
        "command": cmdline.filter(|c| !c.is_empty()).map(|c| truncate(&redact(&c), 600)),
    }))
}

/// Hide values that look like secrets: `--password hunter2` -> `--password ••••`,
/// `TOKEN=abc` -> `TOKEN=••••`. The page has no login, so better safe.
pub fn redact(command: &str) -> String {
    const SECRET: [&str; 6] = ["pass", "token", "secret", "key", "auth", "credential"];
    let mut out: Vec<String> = Vec::new();
    let mut hide_next = false;
    for arg in command.split(' ') {
        if hide_next {
            out.push("••••".into());
            hide_next = false;
            continue;
        }
        let looks_secret = SECRET.iter().any(|s| arg.to_lowercase().contains(s));
        match arg.split_once('=') {
            Some((name, _)) if looks_secret => out.push(format!("{name}=••••")),
            None if looks_secret && arg.starts_with('-') => {
                out.push(arg.into());
                hide_next = true; // the value follows as the next argument
            }
            _ => out.push(arg.into()),
        }
    }
    out.join(" ")
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

fn user(pid: u32) -> Option<String> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let uid: u32 = status.lines().find(|l| l.starts_with("Uid:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(match (system().users.get(&uid), uid) {
        (Some(name), _) => name.clone(),
        (None, 0) => "root".to_string(),
        (None, uid) => uid.to_string(),
    })
}

/// Field N (1-based, as in `man proc`) of a stat line split by `split_stat`.
fn field(fields: &[&str], n: usize) -> u64 {
    fields.get(n - 3).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
}

/// "123 (my name) S 1 …" -> ("my name", ["S", "1", …]).
/// The name may contain spaces and brackets, so split at the last ')'.
fn split_stat(stat: &str) -> Option<(&str, Vec<&str>)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    Some((&stat[open + 1..close], stat.get(close + 2..)?.split_whitespace().collect()))
}

fn mem_total_bytes() -> u64 {
    read("/proc/meminfo")
        .and_then(|m| m.lines().next()?.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |kib| kib * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_names_with_spaces_and_brackets() {
        let (name, fields) = split_stat("42 (Web Content (x)) S 1 2").unwrap();
        assert_eq!(name, "Web Content (x)");
        assert_eq!(fields, ["S", "1", "2"]);
    }

    #[test]
    fn hides_secrets_in_command_lines() {
        assert_eq!(redact("app --password hunter2 --port 80"), "app --password •••• --port 80");
        assert_eq!(redact("run TUNNEL_TOKEN=abc123 -v"), "run TUNNEL_TOKEN=•••• -v");
        assert_eq!(redact("ffmpeg -skip_frame nokey -i movie.mkv"), "ffmpeg -skip_frame nokey -i movie.mkv");
        assert_eq!(redact("java -Xmx10G -jar server.jar nogui"), "java -Xmx10G -jar server.jar nogui");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_details_of_our_own_process() {
        let me = details(std::process::id()).unwrap();
        assert_eq!(me["state"], "running");
        assert!(me["threads"].as_u64().unwrap() >= 1);
    }
}
