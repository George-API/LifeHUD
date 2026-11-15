//! UI glue for the music player's volume slider (worker: `services::volume`).

use std::cell::RefCell;
use std::sync::mpsc::Sender;

use slint::ComponentHandle;

use crate::services::volume::{self, Msg};
use crate::{HudState, HudWindow};

thread_local! {
    static TX: RefCell<Option<Sender<Msg>>> = const { RefCell::new(None) };
    static SOURCE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Starts the volume worker and connects the slider. Call once, on the UI thread.
pub fn attach(hud: &HudWindow) {
    let weak = hud.as_weak();
    let tx = volume::spawn(move |level| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(hud) = weak.upgrade() else { return };
            let st = hud.global::<HudState>();
            st.set_volume_available(level.is_some());
            if let Some(v) = level {
                st.set_volume(v);
            }
        });
    });
    hud.global::<HudState>().on_set_volume(|v| send(Msg::Set(v)));
    TX.with(|t| *t.borrow_mut() = Some(tx));
}

/// The music player changed (or stopped): its SMTC app id, from each media update. Cheap to call
/// often; only a change reaches the worker.
pub fn source(app_id: &str) {
    let changed = SOURCE.with(|s| {
        let mut s = s.borrow_mut();
        if *s == app_id {
            return false;
        }
        *s = app_id.to_string();
        true
    });
    if changed {
        send(Msg::Source(app_id.to_string()));
    }
}

fn send(msg: Msg) {
    TX.with(|t| {
        if let Some(tx) = t.borrow().as_ref() {
            let _ = tx.send(msg);
        }
    });
}
