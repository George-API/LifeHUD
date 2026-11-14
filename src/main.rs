//! Life HUD: a calm, native Windows 11 desktop HUD for weather, time, tasks, notes and music.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod app;
mod locale;
mod platform;
mod services;
mod settings;
mod storage;
mod tasks;
mod theme;

use platform::instance::{self, Signal};

fn main() {
    let exit_requested = std::env::args().any(|a| a == "--exit");
    // A second launch signals the running copy (open Settings, or quit with --exit) and leaves.
    let Some(_instance) = instance::acquire(exit_requested) else { return };

    platform::init_ui_thread();
    std::panic::set_hook(Box::new(|info| {
        storage::log(format!("panic: {info}"));
        // Never leave a dead, reserved strip of screen behind.
        platform::appbar::AppBar::emergency_remove();
    }));

    let app = match app::App::new() {
        Ok(app) => app,
        Err(e) => {
            storage::log(format!("could not start: {e}"));
            return;
        }
    };
    app.start();

    instance::listen(|signal| {
        app::post(move |app| match signal {
            Signal::Activate => app.open_settings(None),
            Signal::Exit => app.quit(),
        });
    });

    if let Err(e) = slint::run_event_loop_until_quit() {
        storage::log(format!("event loop error: {e}"));
    }
    app.shutdown();
}
