//! Collects everything and publishes one snapshot per second, all the time
//! (not only while a page is open), so graphs already have history.
//!
//! Fast, local readings run every second. Anything slow or network-bound
//! (Docker, Jellyfin, Minecraft, SMART) and the process list run on their own
//! threads at their own interval, so they never delay the 1-second update.

use crate::collectors::{docker::Docker, jellyfin, minecraft, processes::Processes, sensors::Sensors, smart::Smart, storage, system};
use crate::config::Config;
use crate::demo::Demo;
use crate::server::State;
use crate::util::{lock, pending, safe};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Latest results of the slower collectors.
struct Slow {
    docker: Mutex<Value>,
    jellyfin: Mutex<Value>,
    minecraft: Mutex<Value>,
    smart: Mutex<Value>,
    processes: Mutex<Value>,
}

/// Run `job` forever, every `period`.
fn every(period: Duration, mut job: impl FnMut() + Send + 'static) {
    thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || loop {
            let started = Instant::now();
            job();
            thread::sleep(period.saturating_sub(started.elapsed()));
        })
        .expect("cannot start collector thread");
}

fn seconds(n: u64) -> Duration {
    Duration::from_secs(n.max(1))
}

pub fn start(config: Arc<Config>, state: Arc<State>) {
    if config.demo {
        let mut demo = Demo::default();
        every(seconds(1), move || {
            let mut snapshot = demo.build();
            finish(&mut snapshot, &config);
            state.publish(&snapshot);
        });
        return;
    }

    let slow = Arc::new(Slow {
        docker: Mutex::new(pending()),
        jellyfin: Mutex::new(pending()),
        minecraft: Mutex::new(pending()),
        smart: Mutex::new(pending()),
        processes: Mutex::new(pending()),
    });

    let (c, s) = (config.clone(), slow.clone());
    let mut docker = Docker::default();
    every(seconds(c.docker_interval), move || *lock(&s.docker) = safe("docker", || docker.collect(&c)));

    let (c, s) = (config.clone(), slow.clone());
    every(seconds(c.jellyfin_interval), move || *lock(&s.jellyfin) = safe("jellyfin", || jellyfin::collect(&c)));

    let (c, s) = (config.clone(), slow.clone());
    every(seconds(c.mc_interval), move || *lock(&s.minecraft) = safe("minecraft", || minecraft::collect(&c)));

    let (c, s) = (config.clone(), slow.clone());
    let mut smart = Smart::default();
    every(seconds(c.smart_interval), move || {
        let disks = storage::physical_disks(&c);
        *lock(&s.smart) = safe("smart", || smart.collect(&c, &disks));
    });

    let (c, s) = (config.clone(), slow.clone());
    let mut processes = Processes::default();
    every(seconds(c.process_interval), move || *lock(&s.processes) = safe("processes", || processes.collect(c.top_processes)));

    let (mut sys, mut sensors, mut disks) = (system::System::default(), Sensors::default(), storage::Storage::default());
    every(seconds(1), move || {
        let smart = lock(&slow.smart)["disks"].clone(); // null until SMART data exists
        let disk_temps = sensors.disk_temps(&config);
        let mut snapshot = json!({
            "host": system::host(&config),
            "cpu": safe("cpu", || sys.cpu()),
            "temps": safe("sensors", || sensors.collect(&config)),
            "memory": safe("memory", system::memory),
            "network": safe("network", || sys.network(&config)),
            "pool": safe("pool", || storage::pool(&config)),
            "disks": safe("disks", || disks.disks(&config, &smart, &disk_temps)),
            "docker": lock(&slow.docker).clone(),
            "processes": lock(&slow.processes).clone(),
            "jellyfin": lock(&slow.jellyfin).clone(),
            "minecraft": lock(&slow.minecraft).clone(),
        });
        finish(&mut snapshot, &config);
        state.publish(&snapshot);
    });
}

/// Add what both real and demo snapshots need: title, hottest disk, status.
pub fn finish(s: &mut Value, config: &Config) {
    let hottest = s["disks"]["items"]
        .as_array()
        .and_then(|items| {
            items.iter().filter(|d| d["temp"].is_number()).max_by(|a, b| {
                a["temp"].as_f64().unwrap_or(0.0).total_cmp(&b["temp"].as_f64().unwrap_or(0.0))
            })
        })
        .map(|d| json!({ "name": d["name"], "temp": d["temp"] }));
    s["temps"]["disk"] = hottest.unwrap_or(Value::Null);
    s["title"] = if config.title.is_empty() {
        s["host"]["hostname"].as_str().unwrap_or("OMV").into()
    } else {
        config.title.clone().into()
    };
    s["demo"] = config.demo.into();
    s["ts"] = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()).into();
    s["limits"] = config.limits_json();
    s["status"] = status(s, config);
}

/// "ok", "warn" or "crit", plus a short reason for every problem found.
fn status(s: &Value, config: &Config) -> Value {
    let mut reasons: Vec<(&str, String)> = Vec::new();
    let mut check = |value: Option<f64>, (warn, crit): (f64, f64), text: String| match value {
        Some(v) if v >= crit => reasons.push(("crit", text)),
        Some(v) if v >= warn => reasons.push(("warn", text)),
        _ => {}
    };
    let l = &config.limits;
    check(s["temps"]["cpu"].as_f64(), l.cpu_temp, "CPU temperature".into());
    check(s["memory"]["percent"].as_f64(), l.usage, "RAM almost full".into());
    check(s["pool"]["percent"].as_f64(), l.usage, "Pool almost full".into());
    for disk in s["disks"]["items"].as_array().into_iter().flatten() {
        let name = disk["name"].as_str().unwrap_or("?");
        check(disk["temp"].as_f64(), l.disk_temp, format!("{name} too hot"));
    }
    for disk in s["disks"]["items"].as_array().into_iter().flatten() {
        let name = disk["name"].as_str().unwrap_or("?");
        if disk["smart"] == false {
            reasons.push(("crit", format!("{name} SMART failed")));
        } else if disk["bad"].as_u64().unwrap_or(0) > 0 {
            reasons.push(("warn", format!("{name} has bad sectors")));
        }
    }
    for c in s["docker"]["containers"].as_array().into_iter().flatten() {
        if c["status"].as_str().unwrap_or("").contains("unhealthy") {
            reasons.push(("warn", format!("{} unhealthy", c["name"].as_str().unwrap_or("?"))));
        }
    }
    let level = if reasons.iter().any(|(l, _)| *l == "crit") {
        "crit"
    } else if reasons.is_empty() {
        "ok"
    } else {
        "warn"
    };
    json!({ "level": level, "reasons": reasons.iter().map(|(_, text)| text).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_snapshot_is_complete_and_titled_by_hostname() {
        let config = Config::from_env();
        let mut snapshot = Demo::default().build();
        finish(&mut snapshot, &config);
        for section in ["cpu", "temps", "memory", "network", "pool", "disks", "docker", "processes", "jellyfin", "minecraft"] {
            assert_eq!(snapshot[section]["ok"], true, "{section}");
        }
        assert_eq!(snapshot["title"], "openmediavault");
        assert!(["ok", "warn", "crit"].contains(&snapshot["status"]["level"].as_str().unwrap()));
    }

    #[test]
    fn status_reports_problems() {
        let config = Config::from_env();
        let s = json!({
            "temps": {"cpu": 95.0}, "memory": {"percent": 50.0}, "pool": {"percent": 88.0},
            "disks": {"items": [{"name": "sda", "temp": 40, "smart": false}]},
            "docker": {"containers": [{"name": "web", "status": "Up 2 days (unhealthy)"}]},
        });
        let result = status(&s, &config);
        assert_eq!(result["level"], "crit");
        assert_eq!(result["reasons"], json!(["CPU temperature", "Pool almost full", "sda SMART failed", "web unhealthy"]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_collectors_never_crash() {
        let config = Config::from_env();
        let (mut sys, mut sensors, mut disks) = (system::System::default(), Sensors::default(), storage::Storage::default());
        let temps = sensors.disk_temps(&config);
        let values = [
            safe("cpu", || sys.cpu()),
            safe("memory", system::memory),
            safe("network", || sys.network(&config)),
            safe("sensors", || sensors.collect(&config)),
            safe("pool", || storage::pool(&config)),
            safe("disks", || disks.disks(&config, &Value::Null, &temps)),
            safe("processes", || Processes::default().collect(5)),
        ];
        assert!(values.iter().all(|v| v.get("ok").is_some()));
        assert_eq!(values[0]["ok"], true);
        assert_eq!(values[1]["ok"], true);
    }
}
