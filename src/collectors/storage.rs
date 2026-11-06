//! mergerfs pool usage and per-disk details.
//!
//! OMV mounts every data disk under /srv, so the container needs /srv mounted
//! read-only. The disk list, models and mount points change rarely and are
//! looked up once a minute; usage and read speed are read every second.

use crate::config::Config;
use crate::util::{read, round1, Rate, Res};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

const SKIP_PREFIXES: &[&str] = &["loop", "ram", "zram", "sr", "dm-", "md", "fd", "nbd", "mmcblk"];

struct Disk {
    name: String,
    model: Option<String>,
    size: u64,
    mount: Option<String>,
}

#[derive(Default)]
pub struct Storage {
    disks: Vec<Disk>,
    found_at: Option<Instant>,
    rates: Rate,
}

pub fn pool(config: &Config) -> Res<Value> {
    if !Path::new(&config.pool_path).is_dir() {
        return Err(format!("{} not found", config.pool_path).into());
    }
    let (total, used, free, percent) = usage(&config.pool_path)?;
    Ok(json!({ "path": config.pool_path, "total": total, "used": used, "free": free, "percent": percent }))
}

/// Physical disks like ["nvme0n1", "sda"], filtered by DISKS if set.
pub fn physical_disks(config: &Config) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir("/sys/block")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !SKIP_PREFIXES.iter().any(|p| n.starts_with(p)))
        .filter(|n| config.disks.is_empty() || config.disks.contains(n))
        .collect();
    names.sort();
    names
}

impl Storage {
    /// `smart` and `temps` come from the SMART and sensor collectors.
    pub fn disks(&mut self, config: &Config, smart: &Value, temps: &HashMap<String, i64>) -> Res<Value> {
        if self.found_at.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
            let mounts = mount_per_disk();
            self.disks = physical_disks(config)
                .into_iter()
                .map(|name| Disk {
                    model: read(format!("/sys/block/{name}/device/model")),
                    size: read(format!("/sys/block/{name}/size")).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) * 512,
                    mount: mounts.get(&name).cloned(),
                    name,
                })
                .collect();
            self.found_at = Some(Instant::now());
        }
        let stats = disk_stats();
        let items: Vec<Value> = self
            .disks
            .iter()
            .map(|d| {
                let info = &smart[&d.name];
                let mut item = json!({
                    "name": d.name,
                    "model": d.model,
                    "size": d.size,
                    "total": null, "used": null, "percent": null,
                    "temp": temps.get(&d.name).copied().map(Value::from).unwrap_or_else(|| info["temp"].clone()),
                    "read": stats.get(&d.name).map(|s| self.rates.per_second(&format!("{}:r", d.name), s.read * 512).round()),
                    "write": stats.get(&d.name).map(|s| self.rates.per_second(&format!("{}:w", d.name), s.written * 512).round()),
                    // Like btop's IO%: share of the last second the disk was busy (100 % = it can't do more).
                    "busy": stats.get(&d.name).map(|s| round1((self.rates.per_second(&format!("{}:b", d.name), s.busy_ms) / 10.0).min(100.0))),
                    "hours": info["hours"],
                    "smart": info["passed"],
                    "bad": info["bad"],
                    "bad_detail": info["bad_detail"],
                });
                if let Some((total, used, _, percent)) = d.mount.as_deref().and_then(|m| usage(m).ok()) {
                    item["total"] = total.into();
                    item["used"] = used.into();
                    item["percent"] = percent.into();
                }
                item
            })
            .collect();
        Ok(json!({ "items": items }))
    }
}

/// Size, used, free and percent of the filesystem at `path` (like `df`).
pub fn usage(path: &str) -> Res<(u64, u64, u64, f64)> {
    let c_path = CString::new(path)?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut s) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    #[allow(clippy::unnecessary_cast)] // field types differ between Linux and macOS
    let (blocks, bfree, bavail, unit) = (s.f_blocks as u64, s.f_bfree as u64, s.f_bavail as u64, s.f_frsize as u64);
    let used = blocks.saturating_sub(bfree) * unit;
    let free = bavail * unit;
    let percent = if used + free > 0 { round1(used as f64 / (used + free) as f64 * 100.0) } else { 0.0 };
    Ok((blocks * unit, used, free, percent))
}

/// Ever-growing counters per disk from /proc/diskstats.
struct DiskStats {
    read: u64,    // sectors read
    written: u64, // sectors written
    busy_ms: u64, // milliseconds spent doing I/O
}

fn disk_stats() -> HashMap<String, DiskStats> {
    read("/proc/diskstats")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            // major minor name reads merged SECTORS_READ ms writes merged SECTORS_WRITTEN ms in_flight BUSY_MS …
            let f: Vec<u64> = l.split_whitespace().map(|x| x.parse().unwrap_or(0)).collect();
            let name = l.split_whitespace().nth(2)?.to_string();
            Some((name, DiskStats { read: *f.get(5)?, written: *f.get(9)?, busy_ms: *f.get(12)? }))
        })
        .collect()
}

/// One mount point per disk, preferring OMV's /srv/… mounts.
fn mount_per_disk() -> HashMap<String, String> {
    let mut result: HashMap<String, String> = HashMap::new();
    for line in read("/proc/self/mountinfo").unwrap_or_default().lines() {
        // "36 35 98:0 / /srv/dev-disk-by-uuid-… rw,… - ext4 /dev/sdb1 rw"
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(dash) = fields.iter().position(|&f| f == "-") else { continue };
        let (Some(mount), Some(source)) = (fields.get(4), fields.get(dash + 2)) else { continue };
        let Some(device) = source.strip_prefix("/dev/") else { continue };
        let disk = parent_disk(device);
        let mount = mount.replace("\\040", " ");
        if !result.contains_key(&disk) || mount.starts_with("/srv/") {
            result.insert(disk, mount);
        }
    }
    result
}

/// "sdb1" -> "sdb", "nvme0n1p2" -> "nvme0n1" (sysfs knows the parent).
fn parent_disk(name: &str) -> String {
    let path = format!("/sys/class/block/{name}");
    if Path::new(&format!("{path}/partition")).exists() {
        if let Some(parent) = fs::canonicalize(&path).ok().and_then(|p| Some(p.parent()?.file_name()?.to_str()?.to_string())) {
            return parent;
        }
    }
    name.to_string()
}
