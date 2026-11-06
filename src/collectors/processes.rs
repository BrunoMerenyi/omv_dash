//! Top processes by CPU, from /proc/<pid>/stat (needs `pid: host`).

use crate::util::{read, round1, Res};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::time::Instant;

pub struct Processes {
    previous: HashMap<u32, u64>, // pid -> CPU ticks at the last scan
    last_scan: Instant,
    users: HashMap<u32, String>, // uid -> name, from /etc/passwd if mounted
    ticks_per_second: f64,
    page_size: u64,
}

impl Default for Processes {
    fn default() -> Self {
        let users = fs::read_to_string("/etc/passwd")
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                Some((f.get(2)?.parse().ok()?, f[0].to_string()))
            })
            .collect();
        Processes {
            previous: HashMap::new(),
            last_scan: Instant::now(),
            users,
            ticks_per_second: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64,
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
        }
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
        let elapsed = self.last_scan.elapsed().as_secs_f64().max(0.001);
        self.last_scan = Instant::now();
        let mut ticks_now = HashMap::new();
        let mut all = Vec::new();

        for entry in fs::read_dir("/proc")? {
            let Some(pid) = entry?.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else { continue }; // already gone
            let Some((name, fields)) = split_stat(&stat) else { continue };
            // fields[0] is field 3 of the stat line, so field N is fields[N - 3]
            let num = |n: usize| fields.get(n - 3).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            let ticks = num(14) + num(15); // utime + stime
            let cpu = self.previous.get(&pid).map_or(0.0, |&before| {
                ticks.saturating_sub(before) as f64 / self.ticks_per_second / elapsed * 100.0
            });
            ticks_now.insert(pid, ticks);
            all.push(Proc { pid, name: name.to_string(), cpu, rss: num(24) * self.page_size });
        }
        self.previous = ticks_now;

        all.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        let mem_total = mem_total_bytes().max(1) as f64;
        let items: Vec<Value> = all
            .iter()
            .take(top)
            .map(|p| {
                json!({
                    "pid": p.pid,
                    "name": p.name,
                    "user": self.user(p.pid),
                    "cpu": round1(p.cpu),
                    "mem": round1(p.rss as f64 / mem_total * 100.0),
                })
            })
            .collect();
        Ok(json!({ "items": items }))
    }

    fn user(&self, pid: u32) -> Option<String> {
        let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let uid: u32 = status.lines().find(|l| l.starts_with("Uid:"))?.split_whitespace().nth(1)?.parse().ok()?;
        Some(match (self.users.get(&uid), uid) {
            (Some(name), _) => name.clone(),
            (None, 0) => "root".to_string(),
            (None, uid) => uid.to_string(),
        })
    }
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
}
