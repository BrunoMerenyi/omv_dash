//! Every setting comes from an environment variable (see .env.example).
//! The defaults match the recommended compose.yml, so most need no setting.

use serde_json::{json, Value};
use std::env;
use std::str::FromStr;

pub struct Config {
    pub demo: bool,
    pub title: String, // empty = the server's hostname
    pub host: String,
    pub port: u16,

    pub host_ip: String,   // empty = detect
    pub net_iface: String, // empty = busiest physical interface
    pub top_processes: usize,
    pub process_interval: u64,

    pub cpu_chip: String,
    pub board_chip: String,
    pub board_label: String,

    pub pool_path: String,
    pub disks: Vec<String>, // empty = all physical disks
    pub smart_dir: String,
    pub smart_interval: u64,

    pub docker_host: String, // tcp://… (read-only proxy) or unix://… (socket)
    pub docker_interval: u64,

    pub jellyfin_url: String,
    pub jellyfin_api_key: String,
    pub jellyfin_interval: u64,

    pub mc_host: String, // empty = Minecraft card off
    pub mc_port: u16,
    pub mc_interval: u64,

    pub limits: Limits,
}

/// (warning, critical) thresholds. Also sent to the browser.
pub struct Limits {
    pub cpu_temp: (f64, f64),
    pub disk_temp: (f64, f64),
    pub usage: (f64, f64),
}

fn text(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn num<T: FromStr>(name: &str, default: T) -> T {
    env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn flag(name: &str) -> bool {
    matches!(text(name, "").trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            demo: flag("DEMO_MODE"),
            title: text("DASH_TITLE", ""),
            host: text("HOST", "0.0.0.0"),
            port: num("PORT", 6767),

            host_ip: text("HOST_IP", ""),
            net_iface: text("NET_IFACE", ""),
            top_processes: num("TOP_PROCESSES", 8),
            process_interval: num("PROCESS_INTERVAL", 3),

            cpu_chip: text("SENSOR_CPU_CHIP", "k10temp"),
            board_chip: text("SENSOR_BOARD_CHIP", "nct6798"),
            board_label: text("SENSOR_BOARD_LABEL", "SYSTIN"),

            pool_path: text("POOL_PATH", "/srv/mergerfs/pool"),
            disks: text("DISKS", "").split(',').map(str::trim).filter(|d| !d.is_empty()).map(String::from).collect(),
            smart_dir: text("SMART_DIR", "/smart"),
            smart_interval: num("SMART_INTERVAL", 1800),

            docker_host: text("DOCKER_HOST", "tcp://127.0.0.1:2375"),
            docker_interval: num("DOCKER_INTERVAL", 10),

            jellyfin_url: text("JELLYFIN_URL", "http://127.0.0.1:8096"),
            jellyfin_api_key: text("JELLYFIN_API_KEY", ""),
            jellyfin_interval: num("JELLYFIN_INTERVAL", 5),

            mc_host: text("MC_HOST", ""),
            mc_port: num("MC_PORT", 25565),
            mc_interval: num("MC_INTERVAL", 10),

            limits: Limits {
                cpu_temp: (num("LIMIT_CPU_TEMP_WARN", 80.0), num("LIMIT_CPU_TEMP_CRIT", 90.0)),
                disk_temp: (num("LIMIT_DISK_TEMP_WARN", 50.0), num("LIMIT_DISK_TEMP_CRIT", 55.0)),
                usage: (num("LIMIT_USAGE_WARN", 85.0), num("LIMIT_USAGE_CRIT", 95.0)),
            },
        }
    }

    pub fn limits_json(&self) -> Value {
        let l = &self.limits;
        json!({
            "cpu_temp": [l.cpu_temp.0, l.cpu_temp.1],
            "disk_temp": [l.disk_temp.0, l.disk_temp.1],
            "usage": [l.usage.0, l.usage.1],
        })
    }
}
