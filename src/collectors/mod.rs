//! Each collector reads one area of the server. They read /proc and /sys
//! directly (no extra libraries) and return JSON for the page.
//!
//! Inside the container these files belong to the host, so they describe the
//! NAS, not the container (network needs `network_mode: host`, processes need
//! `pid: host`).

pub mod docker;
pub mod jellyfin;
pub mod minecraft;
pub mod processes;
pub mod sensors;
pub mod smart;
pub mod storage;
pub mod system;
