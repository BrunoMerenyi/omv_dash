//! SMART health, power-on hours and temperature.
//!
//! The container gets no access to the disks. Instead, a timer on the host
//! runs `smartctl -j -a -n standby /dev/<disk>` and writes <SMART_DIR>/<disk>.json
//! (see README, section SMART). This only reads those files.

use crate::config::Config;
use crate::util::Res;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

#[derive(Default)]
pub struct Smart {
    last: Map<String, Value>, // last good reading per disk (kept while a disk sleeps)
}

impl Smart {
    pub fn collect(&mut self, config: &Config, disks: &[String]) -> Res<Value> {
        let dir = Path::new(&config.smart_dir);
        for disk in disks {
            let parsed = fs::read_to_string(dir.join(format!("{disk}.json")))
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|data| parse(&data));
            if let Some(info) = parsed {
                self.last.insert(disk.clone(), info);
            }
        }
        if self.last.is_empty() {
            return Err("No SMART data (see README, section SMART)".into());
        }
        Ok(json!({ "disks": self.last }))
    }
}

/// smartctl's JSON -> {"passed", "hours", "temp"}. None if the disk was asleep.
pub fn parse(data: &Value) -> Option<Value> {
    let passed = data.get("smart_status")?.get("passed")?.as_bool()?;
    Some(json!({
        "passed": passed,
        "hours": data["power_on_time"]["hours"],
        "temp": data["temperature"]["current"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_smartctl_json_and_skips_sleeping_disks() {
        let awake = json!({"smart_status": {"passed": true}, "power_on_time": {"hours": 21430}, "temperature": {"current": 36}});
        assert_eq!(parse(&awake), Some(json!({"passed": true, "hours": 21430, "temp": 36})));
        assert_eq!(parse(&json!({"smartctl": {"exit_status": 2}})), None);
    }
}
