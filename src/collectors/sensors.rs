//! Temperatures and fans from the Linux hwmon interface (/sys/class/hwmon).
//!
//! - CPU: k10temp ("Tdie"/"Tctl")
//! - Board temperature and fans: nct6798 (needs the nct6775 kernel module)
//! - Disks: the "drivetemp" (SATA) and "nvme" drivers
//!
//! Finding the right files means listing many directories, so that happens
//! once a minute. Every second only the few chosen files are read.

use crate::config::Config;
use crate::util::{read, read_num, round1, Res};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Sensors {
    cpu: Option<PathBuf>,
    board: Option<PathBuf>,
    fans: Vec<(String, PathBuf)>,
    disks: Vec<(String, PathBuf)>,
    found_at: Option<Instant>,
}

impl Sensors {
    /// Look up which hwmon files to read (at most once a minute).
    fn discover(&mut self, config: &Config) {
        if self.found_at.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        *self = Sensors { found_at: Some(Instant::now()), ..Default::default() };
        let Ok(entries) = fs::read_dir("/sys/class/hwmon") else { return };
        for hwmon in entries.flatten().map(|e| e.path()) {
            let chip = read(hwmon.join("name")).unwrap_or_default();
            if chip == config.cpu_chip {
                self.cpu = pick(&labelled(&hwmon, "temp"), &["Tdie", "Tctl"]);
            }
            if chip == config.board_chip {
                self.board = pick(&labelled(&hwmon, "temp"), &[config.board_label.as_str()]);
                self.fans = labelled(&hwmon, "fan")
                    .into_iter()
                    .map(|(label, n, path)| (label.unwrap_or_else(|| format!("Fan {n}")), path))
                    .collect();
            }
            for disk in disk_names(&hwmon, &chip) {
                self.disks.push((disk, hwmon.join("temp1_input")));
            }
        }
    }

    pub fn collect(&mut self, config: &Config) -> Res<Value> {
        self.discover(config);
        let celsius = |path: &Option<PathBuf>| path.as_ref().and_then(read_num::<f64>).map(|m| round1(m / 1000.0));
        let fans: Vec<Value> = self
            .fans
            .iter()
            .filter_map(|(label, path)| read_num::<u64>(path).filter(|&rpm| rpm > 0).map(|rpm| json!({"label": label, "rpm": rpm})))
            .collect(); // unconnected fan headers read 0
        Ok(json!({ "cpu": celsius(&self.cpu), "board": celsius(&self.board), "fans": fans }))
    }

    /// Disk temperatures by disk name, e.g. {"sda": 36, "nvme0n1": 41}.
    pub fn disk_temps(&mut self, config: &Config) -> HashMap<String, i64> {
        self.discover(config);
        self.disks
            .iter()
            .filter_map(|(disk, path)| Some((disk.clone(), (read_num::<f64>(path)? / 1000.0).round() as i64)))
            .collect()
    }
}

/// All "<kind>N_input" files of one chip, with their label (if any) and N, sorted by N.
fn labelled(hwmon: &Path, kind: &str) -> Vec<(Option<String>, u32, PathBuf)> {
    let mut found: Vec<_> = fs::read_dir(hwmon)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().into_string().ok()?;
            let n: u32 = file.strip_prefix(kind)?.strip_suffix("_input")?.parse().ok()?;
            Some((read(hwmon.join(format!("{kind}{n}_label"))), n, e.path()))
        })
        .collect();
    found.sort_by_key(|(_, n, _)| *n);
    found
}

/// The input with one of the preferred labels, otherwise the first one.
fn pick(inputs: &[(Option<String>, u32, PathBuf)], preferred: &[&str]) -> Option<PathBuf> {
    preferred
        .iter()
        .find_map(|want| inputs.iter().find(|(label, ..)| label.as_deref() == Some(*want)))
        .or(inputs.first())
        .map(|(_, _, path)| path.clone())
}

/// Which disk a disk-temperature hwmon belongs to.
fn disk_names(hwmon: &Path, chip: &str) -> Vec<String> {
    let list = |dir: PathBuf| -> Vec<String> {
        fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).collect()
    };
    match chip {
        "drivetemp" => list(hwmon.join("device/block")), // SATA: device/block/sdX
        "nvme" => list(hwmon.join("device"))              // NVMe: device/nvme0n1
            .into_iter()
            .filter(|n| n.strip_prefix("nvme").is_some_and(|rest| rest.contains('n'))) // nvme0n1, not nvme0
            .collect(),
        _ => Vec::new(),
    }
}
