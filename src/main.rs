mod audio_modules;
mod overlay;
mod game_load_checker;
mod types;
mod registry;
mod data_directory;

use std::sync::{Arc, Mutex};
use std::thread;
use types::threat::ThreatState;
use types::overlays::OverlayConfig;

fn main() {
    data_directory::create_all();

    let registry = Arc::new(registry::build());

    game_load_checker::wait_for_game();

    let config = OverlayConfig {
        x:      1,
        y:      1000,
        width:  500,
        height: 500,
    };

    let threat_state = Arc::new(Mutex::new(ThreatState::default()));

    game_load_checker::watch_for_exit(|| {
        std::process::exit(0);
    });

    let audio_state = Arc::clone(&threat_state);
    let audio_registry = Arc::clone(&registry);
    thread::spawn(move || {
        audio_modules::audio::run(audio_state, audio_registry);
    });

    overlay::run(threat_state, config);
}