//! Life HUD: a calm, native Windows 11 desktop HUD for weather, time, tasks, notes and music.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod app;
mod chat;
mod context_menu;
mod feeds;
mod locale;
mod platform;
mod playlists;
mod services;
mod settings;
#[cfg(debug_assertions)]
mod snapshot;
mod stickers;
mod storage;
mod tasks;
mod theme;
mod volume;

use platform::instance::{self, Signal};

fn main() {
    // Developer tool: render widgets offscreen to PNGs (never docks, never touches other windows).
    #[cfg(debug_assertions)]
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--snapshot") {
            snapshot::run(&args[i + 1..]);
            return;
        }
    }
    // Settings → Apps → Uninstall, and install-or-update on a plain launch of a downloaded exe.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--uninstall") {
        platform::install::uninstall(args.iter().any(|a| a == "--quiet"));
        return;
    }
    if let platform::install::Next::Exit = platform::install::on_launch(&args) {
        return;
    }

    let exit_requested = args.iter().any(|a| a == "--exit");
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
