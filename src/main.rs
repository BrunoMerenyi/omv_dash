//! OMV Dashboard: one small binary that collects server metrics once per
//! second and serves the page plus a WebSocket that pushes every update.

mod collectors;
mod config;
mod demo;
mod server;
mod snapshot;
mod util;

use std::sync::Arc;

fn main() {
    util::exit_on_sigterm();
    let config = Arc::new(config::Config::from_env());
    let state = Arc::new(server::State::default());
    snapshot::start(config.clone(), state.clone()); // background threads
    server::run(&config, state); // blocks forever
}
