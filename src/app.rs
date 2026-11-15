//! The application controller: owns state, wires UI callbacks, and bridges the platform and
//! background services into the Slint windows. Everything here runs on the UI thread.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use slint::{
    ComponentHandle, Image, Model, ModelRc, PhysicalPosition, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer,
    TimerMode, VecModel,
};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

use crate::platform::appbar::{AppBar, DockRequest, ShellEvent};
use crate::platform::menu::Item;
use crate::platform::tray::{MenuState, Tray, TrayEvent};
use crate::platform::{effects, hwnd_of, monitors, reflow, shell_open, startup};
use crate::playlists::{self, Playlist};
use crate::services::{http, media, news, weather};
use crate::settings::{Appearance, Layout, MAX_WIDGETS, Settings, WIDGETS, widget_name};
use crate::tasks::TaskList;
use crate::{
    DayData, EditChip, EditState, HourData, HudState, HudWindow, NewsData, PlaylistRow, SettingsState, SettingsWindow,
    Theme, WidgetTile,
};
use crate::{locale, storage, theme};

const NOTES_FILE: &str = "notes.md";
const MIN_TOP_DIP: f32 = 76.0;
const MAX_TOP_DIP: f32 = 200.0;
const MIN_SIDE_DIP: f32 = 248.0;
const MAX_SIDE_DIP: f32 = 560.0;

thread_local! {
    static APP: RefCell<Weak<App>> = const { RefCell::new(Weak::new()) };
}

/// Runs `f` with the app on the UI thread. Safe to call from any thread.
pub fn post(f: impl FnOnce(&Rc<App>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = APP.with(|a| a.borrow().upgrade()) {
            f(&app);
        }
    });
}

pub struct App {
    hud: HudWindow,
    settings: RefCell<Settings>,
    first_run: bool,
    hwnd: Cell<Option<HWND>>,
    appbar: RefCell<Option<AppBar>>,
    settings_window: RefCell<Option<SettingsWindow>>,
    tray: RefCell<Option<Tray>>,
    /// The optional AI chat panel; costs nothing until it's opened.
    chat: Rc<crate::chat::Chat>,
    tasks: RefCell<TaskList>,
    weather_tx: RefCell<Option<Sender<weather::Command>>>,
    media_tx: RefCell<Option<Sender<media::Command>>>,
    news_tx: RefCell<Option<Sender<news::Command>>>,
    news_curated: RefCell<Vec<news::Article>>,
    news_top: RefCell<Vec<news::Article>>,
    /// Bound to the UI once; rows are updated in place so the list isn't rebuilt every minute.
    news_curated_rows: Rc<VecModel<NewsData>>,
    news_top_rows: Rc<VecModel<NewsData>>,
    weather: RefCell<Option<(weather::Snapshot, bool)>>,
    weather_status: RefCell<String>,
    today: RefCell<String>,
    last_minute: Cell<u16>,
    clock_timer: Timer,
    save_timer: Timer,
    /// Refits windows once the Size slider settles, against the work areas from before the drag.
    size_timer: Timer,
    size_before: RefCell<Option<Vec<(String, RECT)>>>,
    notes_timer: Timer,
    /// Edit Widgets: the widget list before each change, newest last.
    edit_undo: RefCell<Vec<Vec<String>>>,
    notes_status_timer: Timer,
    display_timer: Timer,
    /// The layout fold in progress, if any.
    morph: RefCell<Option<Morph>>,
    /// Sports, YouTube, Reddit, Netflix and Stocks.
    pub feeds: crate::feeds::Hub,
}

impl App {
    pub fn new() -> Result<Rc<Self>, slint::PlatformError> {
        let (settings, first_run) = Settings::load();
        let hud = HudWindow::new()?;
        let feeds = crate::feeds::Hub::new(&hud);
        let app = Rc::new(Self {
            hud,
            settings: RefCell::new(settings),
            first_run,
            hwnd: Cell::new(None),
            appbar: RefCell::new(None),
            settings_window: RefCell::new(None),
            tray: RefCell::new(None),
            chat: crate::chat::Chat::new(Default::default()),
            tasks: RefCell::new(TaskList::load()),
            weather_tx: RefCell::new(None),
            media_tx: RefCell::new(None),
            news_tx: RefCell::new(None),
            news_curated: RefCell::new(Vec::new()),
            news_top: RefCell::new(Vec::new()),
            news_curated_rows: Rc::new(VecModel::default()),
            news_top_rows: Rc::new(VecModel::default()),
            weather: RefCell::new(None),
            weather_status: RefCell::new(String::new()),
            today: RefCell::new(today_key()),
            last_minute: Cell::new(u16::MAX),
            clock_timer: Timer::default(),
            save_timer: Timer::default(),
            size_timer: Timer::default(),
            size_before: RefCell::new(None),
            notes_timer: Timer::default(),
            edit_undo: RefCell::new(Vec::new()),
            notes_status_timer: Timer::default(),
            display_timer: Timer::default(),
            morph: RefCell::new(None),
            feeds,
        });
        APP.with(|a| *a.borrow_mut() = Rc::downgrade(&app));
        let state = app.hud.global::<HudState>();
        state.set_news_curated(ModelRc::from(app.news_curated_rows.clone()));
        state.set_news_top(ModelRc::from(app.news_top_rows.clone()));
        Ok(app)
    }

    pub fn start(self: &Rc<Self>) {
        self.apply_theme();
        self.wire_hud();
        self.apply_ai_prefs();
        self.start_clock();
        self.load_notes();
        self.refresh_tasks();
        self.start_services();
        self.feeds.start(&self.settings.borrow());
        self.create_tray();

        self.hud.global::<HudState>().set_vertical(self.settings.borrow().layout == Layout::Left);
        self.apply_sidebar_order();
        if self.settings.borrow().hud_visible {
            self.show_hud();
        }
        if self.first_run {
            self.settings.borrow().save();
            let w = Rc::downgrade(self);
            Timer::single_shot(Duration::from_millis(600), move || {
                if let Some(a) = w.upgrade() {
                    a.open_settings(None);
                }
            });
        }
    }

    pub fn shutdown(&self) {
        self.flush_notes();
        self.tasks.borrow().save();
        self.settings.borrow().save();
        self.appbar.borrow_mut().take(); // unregisters
        self.tray.borrow_mut().take();
    }

    pub fn quit(&self) {
        // A video docked into a widget is another program's window: close it, don't orphan it.
        self.feeds.player.close();
        // Give the space back and let ordinary windows grow into it again.
        let before = self.work_areas();
        self.shutdown();
        if self.settings.borrow().fit_windows {
            std::thread::sleep(Duration::from_millis(250));
            for m in monitors::all() {
                if let Some((_, old)) = before.iter().find(|(d, _)| *d == m.device)
                    && *old != m.work_area
                {
                    reflow::reflow(*old, m.work_area);
                }
            }
        }
        let _ = slint::quit_event_loop();
    }

    // ------------------------------------------------------------------ wiring

    fn wire_hud(self: &Rc<Self>) {
        let state = self.hud.global::<HudState>();
        let w = Rc::downgrade(self);
        macro_rules! with_app {
            ($body:expr) => {{
                let w = w.clone();
                move || {
                    if let Some(a) = w.upgrade() {
                        #[allow(clippy::redundant_closure_call)]
                        ($body)(&a);
                    }
                }
            }};
        }

        state.on_open_settings(with_app!(|a: &Rc<App>| a.open_settings(None)));
        state.on_open_chat(with_app!(|a: &Rc<App>| a.toggle_chat()));
        state.on_weather_clicked(with_app!(|a: &Rc<App>| a.open_settings(Some(3))));
        state.on_hide_hud(with_app!(|a: &Rc<App>| a.set_hud_visible(false)));
        // Deferred out of the pointer handler, like the playlist menu.
        state.on_open_themes(with_app!(|a: &Rc<App>| {
            let w = Rc::downgrade(a);
            Timer::single_shot(Duration::ZERO, move || {
                if let Some(a) = w.upgrade() {
                    a.show_theme_menu();
                }
            });
        }));
        state.on_toggle_layout(with_app!(|a: &Rc<App>| {
            let next = if a.settings.borrow().layout == Layout::Top { Layout::Left } else { Layout::Top };
            a.set_layout(next);
        }));
        self.wire_edit();

        // Tasks
        {
            let w = w.clone();
            state.on_add_task(move |text| {
                let Some(a) = w.upgrade() else { return };
                if a.tasks.borrow_mut().add(&text) {
                    a.hud.global::<HudState>().set_new_task(SharedString::new());
                    a.tasks_changed();
                }
            });
        }
        {
            let w = w.clone();
            state.on_toggle_task(move |id| {
                let Some(a) = w.upgrade() else { return };
                let today = a.today.borrow().clone();
                let done = a.tasks.borrow_mut().toggle(&id, &today);
                a.tasks_changed();
                if done == Some(true) {
                    // Let the check animation play in place, then settle it below open tasks.
                    let w = Rc::downgrade(&a);
                    Timer::single_shot(Duration::from_millis(750), move || {
                        if let Some(a) = w.upgrade() {
                            a.tasks.borrow_mut().settle(&id);
                            a.tasks.borrow().save();
                        }
                    });
                }
            });
        }
        {
            let w = w.clone();
            state.on_remove_task(move |id| {
                let Some(a) = w.upgrade() else { return };
                a.tasks.borrow_mut().remove(&id);
                a.tasks_changed();
            });
        }
        state.on_clear_completed(with_app!(|a: &Rc<App>| {
            a.tasks.borrow_mut().clear_completed();
            a.tasks_changed();
        }));

        // Notes: auto-save shortly after typing stops.
        {
            let w = w.clone();
            state.on_notes_edited(move |_| {
                let Some(a) = w.upgrade() else { return };
                let w2 = Rc::downgrade(&a);
                a.notes_timer.start(TimerMode::SingleShot, Duration::from_millis(600), move || {
                    if let Some(a) = w2.upgrade() {
                        a.flush_notes();
                    }
                });
                a.update_word_count();
            });
        }

        // Media
        state.on_play_pause(with_app!(|a: &Rc<App>| {
            let s = a.hud.global::<HudState>();
            if !s.get_has_media() {
                open_spotify();
                return;
            }
            s.set_is_playing(!s.get_is_playing()); // optimistic; the real state follows
            a.send_media(media::Command::PlayPause);
        }));
        state.on_next_track(with_app!(|a: &Rc<App>| a.send_media(media::Command::Next)));
        state.on_previous_track(with_app!(|a: &Rc<App>| a.send_media(media::Command::Previous)));
        state.on_watching_play_pause(with_app!(|a: &Rc<App>| {
            let s = a.hud.global::<HudState>();
            s.set_watching_playing(!s.get_watching_playing()); // optimistic; the real state follows
            a.send_media(media::Command::WatchPlayPause);
        }));
        {
            let w = w.clone();
            state.on_watching_seek(move |fraction| {
                let Some(a) = w.upgrade() else { return };
                let fraction = fraction.clamp(0.0, 1.0);
                a.hud.global::<HudState>().set_watching_progress(fraction);
                a.send_media(media::Command::WatchSeek(fraction));
            });
        }
        state.on_watching_open(with_app!(|a: &Rc<App>| {
            let s = a.hud.global::<HudState>();
            let (site, title) = (s.get_watching_site(), s.get_watching_title());
            // The tab showing it: its window title carries the video title (YouTube) or "Netflix".
            let windows = crate::platform::browsers::windows();
            let target = windows
                .iter()
                .find(|w| !title.is_empty() && w.title.contains(title.as_str()))
                .or_else(|| windows.iter().find(|w| site == "netflix" && w.title.contains("Netflix")));
            if let Some(w) = target {
                crate::platform::browsers::activate(w.hwnd);
            }
        }));
        {
            let w = w.clone();
            state.on_seek(move |fraction| {
                let Some(a) = w.upgrade() else { return };
                let fraction = fraction.clamp(0.0, 1.0);
                // Optimistic: move the bar now; the player's real position follows on the next poll.
                a.hud.global::<HudState>().set_media_progress(fraction);
                a.send_media(media::Command::Seek(fraction));
            });
        }
        state.on_open_spotify(open_spotify);
        // Deferred out of the pointer handler, so the menu window opens after this click is done.
        state.on_show_playlists(with_app!(|a: &Rc<App>| {
            let w = Rc::downgrade(a);
            Timer::single_shot(Duration::ZERO, move || {
                if let Some(a) = w.upgrade() {
                    a.show_playlist_menu();
                }
            });
        }));

        // News: only ever hand web links to the shell.
        state.on_open_article(|url| {
            if url.starts_with("https://") || url.starts_with("http://") {
                shell_open(&url);
            }
        });
    }

    fn create_tray(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        let w2 = w.clone();
        let tray = Tray::create(
            move |event| {
                let Some(a) = w.upgrade() else { return };
                match event {
                    TrayEvent::Click => {
                        if a.settings.borrow().hud_visible {
                            a.open_settings(None)
                        } else {
                            a.set_hud_visible(true)
                        }
                    }
                    TrayEvent::ToggleHud => {
                        let v = !a.settings.borrow().hud_visible;
                        a.set_hud_visible(v);
                    }
                    TrayEvent::TopLayout => a.set_layout(Layout::Top),
                    TrayEvent::SidebarLayout => a.set_layout(Layout::Left),
                    TrayEvent::Settings => a.open_settings(None),
                    TrayEvent::Chat => a.toggle_chat(),
                    TrayEvent::EditWidgets => a.set_editing(true),
                    TrayEvent::Quit => a.quit(),
                }
            },
            move || {
                let s =
                    w2.upgrade().map(|a| (a.settings.borrow().hud_visible, a.settings.borrow().layout == Layout::Top));
                let (hud_visible, top_layout) = s.unwrap_or((true, true));
                MenuState { hud_visible, top_layout }
            },
        );
        *self.tray.borrow_mut() = tray;
    }

    // ------------------------------------------------------------------ HUD window & docking

    fn show_hud(self: &Rc<Self>) {
        if self.hwnd.get().is_none() {
            // First show: park off-screen until the shell assigns our slot, to avoid a flash.
            self.hud.window().set_position(PhysicalPosition::new(-32000, -32000));
        }
        if let Err(e) = self.hud.show() {
            storage::log(format!("could not show HUD: {e}"));
            return;
        }
        self.attach_platform(0);
    }

    /// The native window is created lazily by the event loop; retry briefly until it exists.
    fn attach_platform(self: &Rc<Self>, attempt: u32) {
        let Some(hwnd) = hwnd_of(self.hud.window()) else {
            if attempt < 100 {
                let w = Rc::downgrade(self);
                Timer::single_shot(Duration::from_millis(20), move || {
                    if let Some(a) = w.upgrade() {
                        a.attach_platform(attempt + 1);
                    }
                });
            }
            return;
        };

        if self.appbar.borrow().is_none() {
            self.hwnd.set(Some(hwnd));
            effects::hide_from_switcher(hwnd);
            effects::make_flush(hwnd);
            effects::set_topmost(hwnd, true);
            self.apply_backdrop();
            let w = Rc::downgrade(self);
            let appbar = AppBar::attach(hwnd, move |event| {
                if let Some(a) = w.upgrade() {
                    a.on_shell_event(event);
                }
            });
            *self.appbar.borrow_mut() = Some(appbar);
        }
        let before = self.work_areas();
        self.dock();
        self.reflow_after(before);
    }

    /// Work areas of every monitor, to compare against after the HUD changes the layout.
    fn work_areas(&self) -> Vec<(String, RECT)> {
        monitors::all().into_iter().map(|m| (m.device, m.work_area)).collect()
    }

    /// Once the shell has settled, refit ordinary windows on any monitor whose usable area changed.
    fn reflow_after(self: &Rc<Self>, before: Vec<(String, RECT)>) {
        if !self.settings.borrow().fit_windows {
            return;
        }
        Timer::single_shot(Duration::from_millis(650), move || {
            for m in monitors::all() {
                if let Some((_, old)) = before.iter().find(|(d, _)| *d == m.device)
                    && *old != m.work_area
                {
                    reflow::reflow(*old, m.work_area);
                }
            }
        });
    }

    fn dock(&self) {
        let req = self.dock_request();
        if let (Some(appbar), Some(req)) = (self.appbar.borrow().as_ref(), req) {
            appbar.register();
            appbar.reposition(req);
        }
    }

    fn reposition(&self) {
        let req = self.dock_request();
        if let (Some(appbar), Some(req)) = (self.appbar.borrow().as_ref(), req) {
            appbar.reposition(req);
        }
    }

    /// Release the old reservation, let the shell recompute the work area, then claim the new one.
    fn redock(self: &Rc<Self>) {
        let before = self.work_areas();
        self.reflow_after(before);
        if let Some(appbar) = self.appbar.borrow().as_ref() {
            appbar.unregister();
        }
        if !self.settings.borrow().hud_visible {
            return;
        }
        let w = Rc::downgrade(self);
        Timer::single_shot(Duration::from_millis(30), move || {
            let Some(a) = w.upgrade() else { return };
            a.dock();
            let w = Rc::downgrade(&a);
            // A second pass picks up any work-area change the shell applied asynchronously.
            Timer::single_shot(Duration::from_millis(400), move || {
                if let Some(a) = w.upgrade() {
                    a.reposition();
                }
            });
        });
    }

    fn dock_request(&self) -> Option<DockRequest> {
        let s = self.settings.borrow();
        let m = monitors::find(s.monitor.as_deref())?;
        let height = (m.bounds.bottom - m.bounds.top) as f32;
        let width = (m.bounds.right - m.bounds.left) as f32;
        let thickness = match s.layout {
            Layout::Top => (height * s.top_size_percent / 100.0).clamp(MIN_TOP_DIP * m.scale, MAX_TOP_DIP * m.scale),
            Layout::Left => (width * s.side_size_percent / 100.0).clamp(MIN_SIDE_DIP * m.scale, MAX_SIDE_DIP * m.scale),
        };
        Some(DockRequest {
            edge: s.layout,
            monitor_bounds: m.bounds,
            work_area: m.work_area,
            thickness: thickness.round() as i32,
        })
    }

    fn on_shell_event(self: &Rc<Self>, event: ShellEvent) {
        match event {
            ShellEvent::PositionChanged => self.reposition(),
            ShellEvent::TaskbarCreated => self.dock(),
            ShellEvent::Fullscreen(on) => {
                // Games, video and presentations get the whole screen.
                if let Some(hwnd) = self.hwnd.get() {
                    effects::set_topmost(hwnd, !on);
                }
            }
            ShellEvent::DisplayChanged => {
                let w = Rc::downgrade(self);
                self.display_timer.start(TimerMode::SingleShot, Duration::from_millis(500), move || {
                    if let Some(a) = w.upgrade() {
                        a.redock();
                    }
                });
            }
            ShellEvent::SettingChanged => {
                // Light/dark or accent colour may have changed.
                if theme::adaptive(&self.settings.borrow().theme)
                    || theme::find(&self.settings.borrow().theme).is_some_and(|p| p.accent.is_none())
                {
                    self.apply_theme();
                }
            }
        }
    }

    pub fn set_hud_visible(self: &Rc<Self>, visible: bool) {
        self.settings.borrow_mut().hud_visible = visible;
        self.schedule_save();
        self.send_media(media::Command::Visible(visible));
        if visible {
            self.show_hud();
        } else {
            let before = self.work_areas();
            if let Some(appbar) = self.appbar.borrow().as_ref() {
                appbar.unregister();
            }
            let _ = self.hud.hide();
            self.reflow_after(before);
        }
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            sw.global::<SettingsState>().set_hud_visible(visible);
        }
        self.apply_widget_activity();
    }

    pub fn set_layout(self: &Rc<Self>, layout: Layout) {
        if self.settings.borrow().layout == layout || self.morph.borrow().is_some() {
            // Ignored mid-animation: put the Settings tiles back in line with the HUD.
            if let Some(sw) = self.settings_window.borrow().as_ref() {
                self.populate_layout(&sw.global::<SettingsState>());
            }
            return;
        }
        let animate = self.settings.borrow().hud_visible && self.hwnd.get().is_some();
        if !animate {
            self.commit_layout(layout);
            self.show_layout();
            self.redock();
            return;
        }

        // 1) Fade the widgets out. 2) Fold the glass from one edge to the other in one motion.
        // 3) Claim the new edge and fade the widgets back in. The fold starts just after the 80 ms
        //    fade, once the widgets are out of the frame, so every fold frame is cheap to draw.
        self.hud.global::<HudState>().set_transitioning(true);
        let w = Rc::downgrade(self);
        Timer::single_shot(Duration::from_millis(90), move || {
            let Some(a) = w.upgrade() else { return };
            let Some(hwnd) = a.hwnd.get() else { return };
            let mut from = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut from);
            }
            // The widgets switch layout when the glass lands (finish_morph), not here: building the
            // new layout on the fold's first frame would cost a dropped frame.
            a.commit_layout(layout);
            let Some(req) = a.dock_request() else {
                a.finish_morph();
                return;
            };
            let (mb, wa, t) = (req.monitor_bounds, req.work_area, req.thickness);
            let target = match layout {
                Layout::Top => RECT { left: wa.left, top: mb.top, right: wa.right, bottom: mb.top + t },
                Layout::Left => RECT { left: mb.left, top: wa.top, right: mb.left + t, bottom: wa.bottom },
            };
            // The long side collapses first; the other grows in as it finishes.
            a.run_morph(from, target, layout == Layout::Left);
        });
    }

    /// Records the new layout. The HUD's widgets follow in [`Self::show_layout`].
    fn commit_layout(&self, layout: Layout) {
        self.settings.borrow_mut().layout = layout;
        self.schedule_save();
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            self.populate_layout(&sw.global::<SettingsState>());
        }
    }

    /// Lays the widgets out for the current layout.
    fn show_layout(&self) {
        self.hud.global::<HudState>().set_vertical(self.settings.borrow().layout == Layout::Left);
        self.apply_widget_activity();
    }

    /// Folds the window from `from` to `to`, one step per display refresh. A small thread waits on
    /// the compositor's vblank and asks the UI thread for the next frame; position comes from elapsed
    /// time, so a late frame catches up instead of slowing the animation down.
    fn run_morph(self: &Rc<Self>, from: RECT, to: RECT, lead_horizontal: bool) {
        let stop = Arc::new(AtomicBool::new(false));
        *self.morph.borrow_mut() = Some(Morph {
            start: Instant::now(),
            from,
            to,
            lead_horizontal,
            stop: stop.clone(),
            frames: 0,
            last: Instant::now(),
            worst_gap_ms: 0.0,
        });
        self.morph_frame();

        let pending = Arc::new(AtomicBool::new(false));
        let spawned = std::thread::Builder::new().name("morph-vsync".into()).spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                monitors::wait_for_vblank();
                // At most one frame queued: a slow UI thread is never flooded.
                if !pending.swap(true, Ordering::AcqRel) {
                    let pending = pending.clone();
                    post(move |a| {
                        pending.store(false, Ordering::Release);
                        a.morph_frame();
                    });
                }
            }
        });
        if spawned.is_err() {
            // No pacing thread: jump straight to the end rather than freeze mid-fold.
            if let Some(m) = self.morph.borrow_mut().as_mut() {
                m.start -= Duration::from_secs(1);
            }
            self.morph_frame();
        }
    }

    fn morph_frame(self: &Rc<Self>) {
        let (rc, done) = {
            let mut morph = self.morph.borrow_mut();
            let Some(m) = morph.as_mut() else { return };
            m.frames += 1;
            m.worst_gap_ms = m.worst_gap_ms.max(m.last.elapsed().as_secs_f32() * 1000.0);
            m.last = Instant::now();
            let u = m.start.elapsed().as_secs_f32() * 1000.0 / MORPH_MS;
            (morph_rect(m.from, m.to, m.lead_horizontal, u), u >= 1.0)
        };
        if let Some(appbar) = self.appbar.borrow().as_ref() {
            appbar.move_freely(rc);
        }
        if done && let Some(m) = self.morph.borrow_mut().take() {
            m.stop.store(true, Ordering::Relaxed);
            let ms = m.start.elapsed().as_secs_f32() * 1000.0;
            storage::log(format!(
                "layout fold: {} frames in {ms:.0} ms ({:.0} fps, worst gap {:.1} ms)",
                m.frames,
                m.frames as f32 * 1000.0 / ms,
                m.worst_gap_ms
            ));
            self.finish_morph();
        }
    }

    fn finish_morph(self: &Rc<Self>) {
        // The glass has landed at its final size: lay the widgets out for it, then bring them back.
        self.show_layout();
        self.redock();
        self.hud.global::<HudState>().set_transitioning(false);
    }

    // ------------------------------------------------------------------ theme

    fn apply_theme(&self) {
        let (id, appearance) = {
            let s = self.settings.borrow();
            (s.theme.clone(), s.appearance)
        };
        let palette = theme::resolve(&id, appearance);
        theme::apply(&self.hud.global::<Theme>(), &palette);
        self.apply_backdrop();
        self.chat.apply_theme(&palette);
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            theme::apply(&sw.global::<Theme>(), &palette);
            theme::fill_gallery(&sw.global::<SettingsState>(), &id, appearance);
            if let Some(hwnd) = hwnd_of(sw.window()) {
                effects::set_dark_title_bar(hwnd, palette.dark);
            }
        }
    }

    fn apply_backdrop(&self) {
        if let Some(hwnd) = self.hwnd.get() {
            let s = self.settings.borrow();
            let palette = theme::resolve(&s.theme, s.appearance);
            if !effects::enable_acrylic(hwnd, palette.tint) {
                storage::log("acrylic unavailable; using a solid surface");
            }
        }
    }

    // ------------------------------------------------------------------ clock

    fn start_clock(self: &Rc<Self>) {
        self.tick();
        let w = Rc::downgrade(self);
        self.clock_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
            if let Some(a) = w.upgrade() {
                a.tick();
            }
        });
    }

    fn tick(&self) {
        let now = locale::now();
        let (use_24h, show_seconds) = {
            let s = self.settings.borrow();
            (s.use_24h, s.show_seconds)
        };
        let state = self.hud.global::<HudState>();
        let hour = now.wHour as u32;
        let hours = if use_24h {
            format!("{hour:02}")
        } else {
            format!("{}", if hour.is_multiple_of(12) { 12 } else { hour % 12 })
        };
        let minutes =
            if show_seconds { format!("{:02}:{:02}", now.wMinute, now.wSecond) } else { format!("{:02}", now.wMinute) };
        state.set_hours(hours.into());
        state.set_minutes(minutes.into());
        state.set_period(if use_24h { SharedString::new() } else { locale::am_pm(now.wHour).into() });

        if self.last_minute.replace(now.wMinute) != now.wMinute {
            state.set_weekday(locale::weekday(&now).into());
            state.set_month_day(locale::month_day(&now).into());
            state.set_greeting(
                match now.wHour {
                    5..=11 => "Good morning",
                    12..=16 => "Good afternoon",
                    17..=21 => "Good evening",
                    _ => "Good night",
                }
                .into(),
            );
        }

        let today = today_key();
        if *self.today.borrow() != today {
            *self.today.borrow_mut() = today.clone();
            if self.tasks.borrow_mut().rollover(&today) {
                self.tasks_changed();
            }
            // Hour labels and "Today" shift with the date.
            self.render_weather();
        }
        if now.wSecond == 0 {
            self.render_news(); // keep "12m" ages honest
            self.feeds.minute(&self.settings.borrow());
        }
    }

    // ------------------------------------------------------------------ tasks & notes

    fn refresh_tasks(&self) {
        let today = self.today.borrow().clone();
        self.tasks.borrow_mut().rollover(&today);
        let state = self.hud.global::<HudState>();
        state.set_tasks(ModelRc::from(self.tasks.borrow().model()));
        self.update_task_summary();
    }

    fn tasks_changed(&self) {
        self.tasks.borrow().save();
        self.update_task_summary();
    }

    fn update_task_summary(&self) {
        let tasks = self.tasks.borrow();
        let state = self.hud.global::<HudState>();
        let (total, done) = (tasks.total(), tasks.done());
        state.set_total_count(total as i32);
        state.set_done_count(done as i32);
        state.set_task_progress(if total == 0 { 0.0 } else { done as f32 / total as f32 });
        state.set_task_summary(tasks.summary().into());
    }

    fn load_notes(&self) {
        let text = storage::read_text(NOTES_FILE).unwrap_or_default();
        self.hud.global::<HudState>().set_notes(text.into());
        self.update_word_count();
    }

    fn update_word_count(&self) {
        let state = self.hud.global::<HudState>();
        let words = state.get_notes().split_whitespace().count();
        state.set_word_count(match words {
            0 => SharedString::new(),
            1 => "1 word".into(),
            n => format!("{n} words").into(),
        });
    }

    fn flush_notes(&self) {
        if !self.notes_timer.running() && self.hud.global::<HudState>().get_notes_status() == "Saved" {
            return;
        }
        self.notes_timer.stop();
        let state = self.hud.global::<HudState>();
        storage::write_atomic(NOTES_FILE, state.get_notes().as_bytes());
        state.set_notes_status("Saved".into());
        let w = APP.with(|a| a.borrow().clone());
        self.notes_status_timer.start(TimerMode::SingleShot, Duration::from_millis(2000), move || {
            if let Some(a) = w.upgrade() {
                a.hud.global::<HudState>().set_notes_status(SharedString::new());
            }
        });
    }

    // ------------------------------------------------------------------ services

    fn weather_config(&self) -> weather::Config {
        let s = self.settings.borrow();
        weather::Config { city: s.weather_city.clone(), celsius: s.celsius, resolved: s.resolved_city.clone() }
    }

    fn start_services(self: &Rc<Self>) {
        let config = self.weather_config();
        if let Some(snapshot) = weather::cached(&config) {
            let stale = weather::unix_now() - snapshot.fetched_at > 15 * 60;
            *self.weather.borrow_mut() = Some((snapshot, stale));
            self.render_weather();
        }
        let tx = weather::spawn(config, |update| {
            post(move |app| app.on_weather(update));
        });
        *self.weather_tx.borrow_mut() = Some(tx);

        let (curated, top) = {
            let s = self.settings.borrow();
            (s.news_curated.clone(), s.news_top.clone())
        };
        *self.news_curated.borrow_mut() = news::cached(&curated).unwrap_or_default();
        *self.news_top.borrow_mut() = news::cached(&top).unwrap_or_default();
        self.render_news();
        let tx = news::spawn(curated, top, |update| {
            post(move |app| app.on_news(update));
        });
        *self.news_tx.borrow_mut() = Some(tx);
        self.apply_widget_activity();

        let prefer = self.settings.borrow().prefer_spotify;
        crate::volume::attach(&self.hud);
        let tx = media::spawn(prefer, |update| {
            post(move |app| app.on_media(update));
        });
        if !self.settings.borrow().hud_visible {
            let _ = tx.send(media::Command::Visible(false)); // started hidden (e.g. from the tray)
        }
        *self.media_tx.borrow_mut() = Some(tx);
    }

    fn send_media(&self, cmd: media::Command) {
        if let Some(tx) = self.media_tx.borrow().as_ref() {
            let _ = tx.send(cmd);
        }
    }

    /// Tells every widget's fetcher whether it's on screen: off-screen widgets don't use the network.
    fn apply_widget_activity(&self) {
        let s = self.settings.borrow();
        self.feeds.apply_layout(&s);
        let visible = s.visible_widgets();
        self.send_weather(weather::Command::Active(visible.contains(&"weather")));
        if let Some(tx) = self.news_tx.borrow().as_ref() {
            let _ = tx.send(news::Command::Active(visible.contains(&"news")));
        }
    }

    fn send_weather(&self, cmd: weather::Command) {
        if let Some(tx) = self.weather_tx.borrow().as_ref() {
            let _ = tx.send(cmd);
        }
    }

    fn on_weather(&self, update: weather::Update) {
        match update {
            weather::Update::Snapshot { snapshot, stale } => {
                *self.weather.borrow_mut() = Some((snapshot, stale));
                self.render_weather();
            }
            weather::Update::Unavailable(reason) => {
                let state = self.hud.global::<HudState>();
                if self.weather.borrow().is_none() {
                    state.set_weather_ready(false);
                }
                state.set_weather_placeholder(reason.into());
            }
            weather::Update::Resolved(city) => {
                self.settings.borrow_mut().resolved_city = Some(city);
                self.schedule_save();
            }
            weather::Update::Status(text) => {
                *self.weather_status.borrow_mut() = text.clone();
                if let Some(sw) = self.settings_window.borrow().as_ref() {
                    sw.global::<SettingsState>().set_weather_status(text.into());
                }
            }
        }
    }

    fn render_weather(&self) {
        let data = self.weather.borrow();
        let Some((s, stale)) = data.as_ref() else { return };
        let use_24h = self.settings.borrow().use_24h;
        let state = self.hud.global::<HudState>();
        let deg = |v: f64| format!("{}°", v.round() as i64);

        state.set_location(s.location_name.as_str().into());
        state.set_temperature(deg(s.temperature).into());
        state.set_condition(weather::condition(s.code));
        state.set_is_day(s.is_day);
        state.set_description(weather::describe(s.code, s.is_day).into());
        // Calm by default: only speak up when something is off.
        state.set_weather_status(if *stale { "Offline".into() } else { SharedString::new() });

        let skip = ((weather::unix_now() - s.fetched_at).max(0) / 3600) as usize;
        let hours: Vec<HourData> = s
            .hours
            .iter()
            .skip(skip)
            .take(6)
            .enumerate()
            .map(|(i, h)| HourData {
                label: if i == 0 { "Now".into() } else { locale::hour_label(h.hour, use_24h).into() },
                temp: deg(if i == 0 { s.temperature } else { h.temperature }).into(),
                condition: weather::condition(h.code),
                day: h.is_day,
            })
            .collect();

        let today = today_key();
        let days: Vec<_> = s.days.iter().filter(|d| d.date >= today).collect();
        if let Some(first) = days.first() {
            state.set_high_low(format!("H:{}  L:{}", deg(first.high), deg(first.low)).into());
        }
        let min = days.iter().map(|d| d.low).fold(f64::INFINITY, f64::min);
        let max = days.iter().map(|d| d.high).fold(f64::NEG_INFINITY, f64::max);
        let span = (max - min).max(1.0);
        let to_c = |v: f64| if s.celsius { v } else { (v - 32.0) * 5.0 / 9.0 };
        let daily: Vec<DayData> = days
            .iter()
            .enumerate()
            .map(|(i, d)| DayData {
                label: if i == 0 {
                    "Today".into()
                } else {
                    locale::date_from_iso(&d.date).map(|st| locale::weekday_short(&st)).unwrap_or_default().into()
                },
                condition: weather::condition(d.code),
                high: deg(d.high).into(),
                low: deg(d.low).into(),
                low_pos: ((d.low - min) / span) as f32,
                high_pos: ((d.high - min) / span) as f32,
                marker_pos: ((s.temperature - min) / span) as f32,
                has_marker: i == 0,
                low_color: theme::temperature_color(to_c(d.low)),
                high_color: theme::temperature_color(to_c(d.high)),
            })
            .collect();

        // Update the rows in place: a refresh that changes one temperature redraws one cell, not
        // the whole forecast.
        let upcoming = daily.iter().skip(1).take(3).cloned().collect::<Vec<_>>();
        sync_model(state.get_upcoming(), |m| state.set_upcoming(m), upcoming);
        sync_model(state.get_daily(), |m| state.set_daily(m), daily);
        sync_model(state.get_hourly(), |m| state.set_hourly(m), hours);
        state.set_weather_ready(true);
    }

    fn on_news(&self, update: news::Update) {
        let (curated, top) = {
            let s = self.settings.borrow();
            (s.news_curated.clone(), s.news_top.clone())
        };
        let slot = |feed: news::Feed| if feed == news::Feed::Curated { &self.news_curated } else { &self.news_top };
        let current = |feed: news::Feed| if feed == news::Feed::Curated { curated.clone() } else { top.clone() };
        match update {
            news::Update::Articles { feed, source, articles } if source == current(feed) => {
                *slot(feed).borrow_mut() = articles;
                self.render_news();
            }
            news::Update::Failed { feed, source } if source == current(feed) && slot(feed).borrow().is_empty() => {
                self.hud.global::<HudState>().set_news_placeholder("Headlines will appear when you're online.".into());
            }
            _ => {}
        }
    }

    fn render_news(&self) {
        let state = self.hud.global::<HudState>();
        {
            let s = self.settings.borrow();
            state.set_curated_name(news::find(news::Feed::Curated, &s.news_curated).name.into());
            state.set_top_name(news::find(news::Feed::Top, &s.news_top).name.into());
        }
        let now = weather::unix_now();
        sync_news_rows(&self.news_curated_rows, &self.news_curated.borrow(), now);
        sync_news_rows(&self.news_top_rows, &self.news_top.borrow(), now);
    }

    fn on_media(&self, u: media::Update) {
        crate::volume::source(&u.source_app);
        let state = self.hud.global::<HudState>();
        match &u.track {
            Some(t) => {
                state.set_has_media(true);
                state.set_track_title(if t.title.is_empty() {
                    "Unknown Track".into()
                } else {
                    t.title.as_str().into()
                });
                state.set_track_artist(t.artist.as_str().into());
                state.set_media_source(t.source.to_uppercase().into());
                state.set_is_spotify(t.is_spotify);
            }
            None => {
                state.set_has_media(false);
                state.set_track_title("Not Playing".into());
                state.set_track_artist("Open Spotify to start listening".into());
                state.set_media_source("SPOTIFY".into());
                state.set_is_spotify(true);
            }
        }
        state.set_is_playing(u.track.is_some() && u.playing);
        state.set_can_previous(u.can_previous);
        state.set_can_next(u.can_next);
        state.set_can_seek(u.track.is_some() && u.can_seek && u.duration_ms > 1000);
        state.set_has_timeline(u.track.is_some() && u.duration_ms > 1000);
        if u.duration_ms > 0 {
            state.set_media_progress(u.position_ms as f32 / u.duration_ms as f32);
            state.set_elapsed(format_ms(u.position_ms).into());
            state.set_remaining(format!("-{}", format_ms(u.duration_ms.saturating_sub(u.position_ms))).into());
        }
        match u.art {
            media::ArtChange::Keep => {}
            media::ArtChange::Clear => {
                state.set_has_artwork(false);
                state.set_artwork(Image::default());
                state.set_glow(slint::Color::from_argb_u8(0, 0, 0, 0));
            }
            media::ArtChange::Set(art) => {
                let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&art.rgba, art.size, art.size);
                state.set_artwork(Image::from_rgba8(buffer));
                state.set_has_artwork(true);
                state.set_glow(match art.glow {
                    Some((r, g, b)) => slint::Color::from_argb_u8(0x3A, r, g, b),
                    None => slint::Color::from_argb_u8(0, 0, 0, 0),
                });
            }
        }
        self.show_watching(u.watching.as_ref(), u.watch_thumb);
    }

    /// "Now watching" for the YouTube and Netflix widgets.
    fn show_watching(&self, w: Option<&media::Watching>, thumb: media::ThumbChange) {
        let state = self.hud.global::<HudState>();
        state.set_watching(w.is_some());
        if let Some(w) = w {
            state.set_watching_site(if w.site == media::Site::YouTube { "youtube" } else { "netflix" }.into());
            state.set_watching_title(w.title.as_str().into());
            state.set_watching_subtitle(w.subtitle.as_str().into());
            state.set_watching_browser(w.browser.to_uppercase().into());
            state.set_watching_playing(w.playing);
            state.set_watching_can_seek(w.can_seek && w.duration_ms > 1000);
            state.set_watching_progress(if w.duration_ms > 0 {
                w.position_ms as f32 / w.duration_ms as f32
            } else {
                0.0
            });
        }
        match thumb {
            media::ThumbChange::Keep => {}
            media::ThumbChange::Clear => {
                state.set_watching_has_art(false);
                state.set_watching_art(Image::default());
            }
            media::ThumbChange::Set(t) => {
                let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&t.rgba, t.width, t.height);
                state.set_watching_art(Image::from_rgba8(buffer));
                state.set_watching_has_art(true);
            }
        }
    }

    // ------------------------------------------------------------------ AI chat

    fn toggle_chat(self: &Rc<Self>) {
        if self.chat.is_open() {
            self.settings.borrow_mut().ai.tab = self.chat.tab();
            self.schedule_save();
            self.chat.close();
            return;
        }
        let (palette, top_layout, tab) = {
            let s = self.settings.borrow();
            (theme::resolve(&s.theme, s.appearance), s.layout == Layout::Top, s.ai.tab)
        };
        let hud = self.settings.borrow().hud_visible.then(|| self.hwnd.get()).flatten().map(|hwnd| {
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut r);
            }
            r
        });
        self.chat.open(&palette, crate::chat::Anchor { hud, top_layout }, Some(tab));
    }

    /// Pushes the AI preferences to the HUD (button) and the chat panel (models).
    fn apply_ai_prefs(&self) {
        let ai = self.settings.borrow().ai.clone();
        self.hud.global::<HudState>().set_ai_button(ai.show_button);
        self.chat.set_models(ai.models);
    }

    fn wire_ai_settings(self: &Rc<Self>, st: &SettingsState) {
        st.set_ai_button(self.settings.borrow().ai.show_button);
        st.set_ai_providers(crate::chat::provider_rows(&self.settings.borrow().ai.models));
        // A message under one provider's group (empty clears it). Doesn't rebuild the rows, so a
        // field being typed in keeps its focus.
        let show_error = |a: &App, index: usize, error: &str| {
            if let Some(sw) = a.settings_window.borrow().as_ref() {
                let st = sw.global::<SettingsState>();
                let mut errors: Vec<SharedString> = st.get_ai_errors().iter().collect();
                errors.resize(3, SharedString::new());
                errors[index.min(2)] = error.into();
                st.set_ai_errors(ModelRc::new(VecModel::from(errors)));
            }
        };
        let refresh = move |a: &App, index: usize, error: String| {
            if let Some(sw) = a.settings_window.borrow().as_ref() {
                sw.global::<SettingsState>()
                    .set_ai_providers(crate::chat::provider_rows(&a.settings.borrow().ai.models));
            }
            show_error(a, index, &error);
            a.chat.refresh_keys();
        };
        let w = Rc::downgrade(self);
        st.on_ai_save_key(move |i, key| {
            let Some(a) = w.upgrade() else { return };
            let i = (i.max(0) as usize).min(2);
            let error = crate::chat::save_key(i, &key).err().unwrap_or_default();
            refresh(&a, i, error);
        });
        let w = Rc::downgrade(self);
        st.on_ai_remove_key(move |i| {
            let Some(a) = w.upgrade() else { return };
            let i = (i.max(0) as usize).min(2);
            crate::chat::remove_key(i);
            refresh(&a, i, String::new());
        });
        let w = Rc::downgrade(self);
        st.on_ai_set_model(move |i, model| {
            let Some(a) = w.upgrade() else { return };
            let i = (i.max(0) as usize).min(2);
            let model = match crate::chat::clean_model(i, &model) {
                Ok(m) => {
                    show_error(&a, i, "");
                    m
                }
                Err(message) => return show_error(&a, i, &message),
            };
            if a.settings.borrow().ai.models[i] != model {
                a.settings.borrow_mut().ai.models[i] = model;
                a.apply_ai_prefs();
                a.schedule_save();
            }
        });
        st.on_ai_get_key(|i| crate::chat::open_keys_page(i.max(0) as usize));
    }

    // ------------------------------------------------------------------ settings window

    fn schedule_save(&self) {
        let w = APP.with(|a| a.borrow().clone());
        self.save_timer.start(TimerMode::SingleShot, Duration::from_millis(400), move || {
            if let Some(a) = w.upgrade() {
                a.settings.borrow().save();
            }
        });
    }

    pub fn open_settings(self: &Rc<Self>, page: Option<i32>) {
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            if let Some(p) = page {
                sw.global::<SettingsState>().set_page(p);
            }
            let _ = sw.show();
            if let Some(hwnd) = hwnd_of(sw.window()) {
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
                }
            }
            return;
        }

        let sw = match SettingsWindow::new() {
            Ok(sw) => sw,
            Err(e) => {
                storage::log(format!("could not open settings: {e}"));
                return;
            }
        };
        let s = self.settings.borrow().clone();
        let palette = theme::resolve(&s.theme, s.appearance);
        theme::apply(&sw.global::<Theme>(), &palette);

        let st = sw.global::<SettingsState>();
        st.set_page(page.unwrap_or(0));
        st.set_hud_visible(s.hud_visible);
        st.set_fit_windows(s.fit_windows);
        st.set_open_at_login(startup::is_enabled());
        self.populate_layout(&st);
        theme::fill_gallery(&st, &s.theme, s.appearance);
        st.set_appearance(match s.appearance {
            Appearance::Auto => 0,
            Appearance::Light => 1,
            Appearance::Dark => 2,
        });
        st.set_appearance_enabled(theme::adaptive(&s.theme));
        st.set_use_24h(s.use_24h);
        st.set_show_seconds(s.show_seconds);
        st.set_city(s.weather_city.as_str().into());
        st.set_weather_status(self.weather_status.borrow().as_str().into());
        st.set_units(if s.celsius { 0 } else { 1 });
        st.set_prefer_spotify(s.prefer_spotify);
        st.set_playlists(playlist_rows(&s.playlists));
        let names = |feed| -> ModelRc<SharedString> {
            ModelRc::new(VecModel::from(
                news::sources(feed).iter().map(|n| SharedString::from(n.name)).collect::<Vec<_>>(),
            ))
        };
        st.set_curated_sources(names(news::Feed::Curated));
        st.set_top_sources(names(news::Feed::Top));
        st.set_curated_index(news::CURATED.iter().position(|n| n.id == s.news_curated).unwrap_or(0) as i32);
        st.set_top_index(news::TOP.iter().position(|n| n.id == s.news_top).unwrap_or(0) as i32);
        st.set_version(env!("CARGO_PKG_VERSION").into());
        crate::feeds::settings::wire(&sw);
        crate::feeds::settings::populate(&sw, &s);
        self.wire_ai_settings(&st);

        let w = Rc::downgrade(self);
        st.on_changed(move |key| {
            if let Some(a) = w.upgrade() {
                a.on_settings_changed(&key);
            }
        });
        let w = Rc::downgrade(self);
        st.on_select_theme(move |id| {
            if let Some(a) = w.upgrade() {
                a.set_theme(&id);
            }
        });
        let w = Rc::downgrade(self);
        st.on_reset_layout(move || {
            let Some(a) = w.upgrade() else { return };
            let before = a.work_areas();
            // Re-places cards and slots, reconfigures widgets, refreshes their settings and saves.
            a.update_settings(Settings::reset_layout);
            if let Some(sw) = a.settings_window() {
                a.populate_layout(&sw.global::<SettingsState>());
            }
            // The default size may differ from the current one.
            a.reposition();
            a.reflow_after(before);
        });
        let w = Rc::downgrade(self);
        st.on_add_playlist(move |text| {
            if let Some(a) = w.upgrade() {
                a.add_playlist(&text);
            }
        });
        let w = Rc::downgrade(self);
        st.on_toggle_pin(move |uri| {
            let Some(a) = w.upgrade() else { return };
            if let Some(p) = a.settings.borrow_mut().playlists.iter_mut().find(|p| p.uri == uri.as_str()) {
                p.pinned = !p.pinned;
            }
            a.playlists_changed();
        });
        let w = Rc::downgrade(self);
        st.on_remove_playlist(move |uri| {
            let Some(a) = w.upgrade() else { return };
            a.settings.borrow_mut().playlists.retain(|p| p.uri != uri.as_str());
            a.playlists_changed();
        });
        st.on_open_data_folder(|| {
            shell_open(&storage::data_dir().to_string_lossy());
        });
        // About page links: only ever the project's own pages.
        st.on_open_link(|url| {
            if url.starts_with("https://github.com/George-API/LifeHUD") {
                shell_open(&url);
            }
        });

        // Closing frees the window entirely; it's rebuilt on demand.
        let w = Rc::downgrade(self);
        sw.window().on_close_requested(move || {
            if let Some(a) = w.upgrade() {
                let typed = a.settings_window.borrow().as_ref().map(|sw| sw.global::<SettingsState>().get_city());
                if typed.is_some_and(|c| c.trim() != a.settings.borrow().weather_city) {
                    a.on_settings_changed("weather");
                }
                let w = Rc::downgrade(&a);
                Timer::single_shot(Duration::ZERO, move || {
                    if let Some(a) = w.upgrade() {
                        a.settings_window.borrow_mut().take();
                    }
                });
            }
            slint::CloseRequestResponse::HideWindow
        });

        if let Err(e) = sw.show() {
            storage::log(format!("could not show settings: {e}"));
            return;
        }
        *self.settings_window.borrow_mut() = Some(sw);
        let w = Rc::downgrade(self);
        Timer::single_shot(Duration::from_millis(30), move || {
            if let Some(a) = w.upgrade()
                && let Some(sw) = a.settings_window.borrow().as_ref()
                && let Some(hwnd) = hwnd_of(sw.window())
            {
                effects::set_dark_title_bar(hwnd, palette.dark);
            }
        });
    }

    // ------------------------------------------------------------------ playlists

    /// Switches theme, from Settings or the HUD's quick menu alike.
    fn set_theme(&self, id: &str) {
        self.settings.borrow_mut().theme = id.to_string();
        self.schedule_save();
        self.apply_theme();
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            sw.global::<SettingsState>().set_appearance_enabled(theme::adaptive(id));
        }
    }

    /// The paintbrush's quick-swap menu: every theme with a colour preview, the current one ticked.
    fn show_theme_menu(self: &Rc<Self>) {
        if crate::context_menu::is_open() {
            return;
        }
        let (current, appearance) = {
            let s = self.settings.borrow();
            (s.theme.clone(), s.appearance)
        };
        let to_u32 = |c: slint::Color| u32::from_be_bytes([c.alpha(), c.red(), c.green(), c.blue()]);
        let (mut signature, mut moods) = (vec![Item::Header("Signature".into())], vec![]);
        for (i, entry) in theme::themes().into_iter().enumerate() {
            let p = theme::resolve(entry.id, appearance);
            let [a, _, c] = theme::ambient_stops(&p);
            let item = Item::Swatch {
                id: i,
                text: p.name.into(),
                checked: p.id.eq_ignore_ascii_case(&current),
                surface: p.tint | 0xFF00_0000,
                glow_a: to_u32(a),
                glow_b: to_u32(c),
                card: p.card,
                accent: to_u32(theme::accent(&p)),
            };
            if p.stickers > 0 { moods.push(item) } else { signature.push(item) }
        }
        if !moods.is_empty() {
            signature.push(Item::Separator);
            signature.push(Item::Header("Mood".into()));
            signature.extend(moods);
        }
        let palette = theme::resolve(&current, appearance);
        let w = Rc::downgrade(self);
        crate::context_menu::show(&palette, signature, move |i| {
            if let (Some(a), Some(p)) = (w.upgrade(), theme::themes().get(i).copied()) {
                a.set_theme(p.id);
            }
        });
    }

    /// The music card's right-click menu: pinned first, then the rest, then Spotify shortcuts.
    fn show_playlist_menu(self: &Rc<Self>) {
        const LIKED: usize = 1;
        const LIBRARY: usize = 2;
        const MANAGE: usize = 3;
        const FIRST: usize = 100;
        let list = self.settings.borrow().playlists.clone();
        let ordered = playlists::ordered(&list);

        let mut items = Vec::new();
        let pinned = ordered.iter().take_while(|p| p.pinned).count();
        for (i, p) in ordered.iter().enumerate() {
            if i == 0 && pinned > 0 {
                items.push(Item::Header("Pinned".into()));
            }
            if i == pinned {
                if pinned > 0 {
                    items.push(Item::Separator);
                }
                items.push(Item::Header("Playlists".into()));
            }
            items.push(Item::Entry { id: FIRST + i, text: p.name.clone(), checked: false });
        }
        if !ordered.is_empty() {
            items.push(Item::Separator);
        }
        items.push(Item::Entry { id: LIKED, text: "Liked Songs".into(), checked: false });
        items.push(Item::Entry { id: LIBRARY, text: "Your Library".into(), checked: false });
        items.push(Item::Separator);
        let manage = if ordered.is_empty() { "Add Playlists…" } else { "Manage Playlists…" };
        items.push(Item::Entry { id: MANAGE, text: manage.into(), checked: false });

        // Only canonical URIs from the saved list are ever opened (see playlists::normalize).
        let uris: Vec<String> = ordered.iter().map(|p| p.uri.clone()).collect();
        let palette = {
            let s = self.settings.borrow();
            theme::resolve(&s.theme, s.appearance)
        };
        let w = Rc::downgrade(self);
        crate::context_menu::show(&palette, items, move |choice| match choice {
            LIKED => open_in_spotify("spotify:collection:tracks", "https://open.spotify.com/collection/tracks"),
            LIBRARY => open_in_spotify("spotify:collection", "https://open.spotify.com/collection/playlists"),
            MANAGE => {
                if let Some(a) = w.upgrade() {
                    a.open_settings(Some(4));
                }
            }
            id if id >= FIRST => {
                if let Some(uri) = uris.get(id - FIRST) {
                    open_in_spotify(uri, &playlists::web_url(uri));
                }
            }
            _ => {}
        });
    }

    fn add_playlist(&self, text: &str) {
        let error = match playlists::parse_link(text) {
            None => Some("That doesn't look like a Spotify playlist, album, artist or podcast link."),
            Some(uri) => {
                let mut s = self.settings.borrow_mut();
                if s.playlists.iter().any(|p| p.uri == uri) {
                    Some("That one is already saved.")
                } else if s.playlists.len() >= playlists::MAX {
                    Some("You've reached the limit. Remove one to add another.")
                } else {
                    s.playlists.push(Playlist {
                        name: playlists::placeholder_name(&uri),
                        uri: uri.clone(),
                        pinned: false,
                    });
                    fetch_playlist_title(uri);
                    None
                }
            }
        };
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            let st = sw.global::<SettingsState>();
            st.set_playlist_error(error.unwrap_or("").into());
            if error.is_none() {
                st.set_playlist_link(SharedString::new());
            }
        }
        if error.is_none() {
            self.playlists_changed();
        }
    }

    /// Fills in the real title once Spotify's public oEmbed endpoint answers.
    fn set_playlist_title(&self, uri: &str, title: String) {
        if let Some(p) = self.settings.borrow_mut().playlists.iter_mut().find(|p| p.uri == uri) {
            p.name = title;
        }
        self.playlists_changed();
    }

    fn playlists_changed(&self) {
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            sw.global::<SettingsState>().set_playlists(playlist_rows(&self.settings.borrow().playlists));
        }
        self.schedule_save();
    }

    fn apply_sidebar_order(&self) {
        let kinds = self.settings.borrow().sidebar_kinds();
        self.hud.global::<HudState>().set_sidebar_order(ModelRc::new(VecModel::from(kinds)));
        self.apply_widget_activity();
        self.refresh_edit();
    }

    // ------------------------------------------------------------------ Edit Widgets

    /// Connects Edit Widgets mode (ui/edit.slint). Every change goes through `update_settings`, so the
    /// HUD, an open Settings window and the saved file stay in step.
    fn wire_edit(self: &Rc<Self>) {
        let es = self.hud.global::<EditState>();
        let ids: Vec<SharedString> = WIDGETS.iter().map(|(id, _)| SharedString::from(*id)).collect();
        let names: Vec<SharedString> = WIDGETS.iter().map(|(_, name)| SharedString::from(*name)).collect();
        es.set_ids(ModelRc::new(VecModel::from(ids)));
        es.set_names(ModelRc::new(VecModel::from(names)));
        es.set_capacity(MAX_WIDGETS as i32);

        macro_rules! on {
            ($setter:ident, |$a:ident $(, $arg:ident)*| $body:expr) => {{
                let w = Rc::downgrade(self);
                es.$setter(move |$($arg),*| {
                    if let Some($a) = w.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(on_begin, |a| a.set_editing(true));
        on!(on_done, |a| a.set_editing(false));
        on!(on_place, |a, from, to| a.edit_change(|s| s.place_card(from.max(0) as usize, to.max(0) as usize)));
        on!(on_insert, |a, id, at| a.edit_change(|s| s.insert_card(&id, at.max(0) as usize)));
        on!(on_replace, |a, index, id| a.edit_change(|s| s.replace_card(index.max(0) as usize, &id)));
        on!(on_remove, |a, index| a.edit_change(|s| s.remove_card(index.max(0) as usize)));
        on!(on_nudge, |a, index, delta| a.edit_change(|s| s.move_card(index.max(0) as usize, delta.signum())));
        on!(on_undo, |a| a.edit_undo());
    }

    /// Enters or leaves Edit Widgets mode. Entering shows the HUD if it was hidden.
    pub fn set_editing(self: &Rc<Self>, on: bool) {
        if on && !self.settings.borrow().hud_visible {
            self.set_hud_visible(true);
        }
        // Each editing session starts with a clean history.
        self.edit_undo.borrow_mut().clear();
        self.refresh_edit();
        self.hud.global::<EditState>().set_editing(on);
    }

    /// Applies one edit and remembers the list as it was, for Undo.
    fn edit_change(&self, f: impl FnOnce(&mut Settings)) {
        const HISTORY: usize = 50;
        let before = self.settings.borrow().sidebar_order.clone();
        self.update_settings(f);
        if self.settings.borrow().sidebar_order != before {
            let mut undo = self.edit_undo.borrow_mut();
            undo.push(before);
            let extra = undo.len().saturating_sub(HISTORY);
            undo.drain(..extra);
        }
        self.refresh_edit();
    }

    fn edit_undo(&self) {
        let previous = self.edit_undo.borrow_mut().pop();
        if let Some(order) = previous {
            self.update_settings(|s| s.sidebar_order = order);
        }
        self.refresh_edit();
    }

    /// What the tray offers and how full the HUD is.
    fn refresh_edit(&self) {
        let es = self.hud.global::<EditState>();
        let s = self.settings.borrow();
        let extras: Vec<EditChip> = s
            .sidebar_available()
            .into_iter()
            .map(|id| EditChip { id: id.into(), name: widget_name(id).into() })
            .collect();
        es.set_extras(ModelRc::new(VecModel::from(extras)));
        es.set_full(s.hud_full());
        es.set_count(s.hud_widgets().len() as i32);
        es.set_can_undo(!self.edit_undo.borrow().is_empty());
    }

    // ------------------------------------------------------------------ feed widgets (src/feeds)

    pub fn settings(&self) -> std::cell::Ref<'_, Settings> {
        self.settings.borrow()
    }

    pub fn settings_window(&self) -> Option<SettingsWindow> {
        self.settings_window.borrow().as_ref().map(|sw| sw.clone_strong())
    }

    /// Applies a change made on a widget settings page: re-places cards, reconfigures the widgets,
    /// refreshes the open Settings window and saves.
    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) {
        {
            let mut s = self.settings.borrow_mut();
            f(&mut s);
            s.normalize();
        }
        self.apply_sidebar_order();
        self.feeds.configure(&self.settings.borrow());
        if let Some(sw) = self.settings_window() {
            crate::feeds::settings::populate(&sw, &self.settings.borrow());
            self.populate_widget_tiles(&sw.global::<SettingsState>());
        }
        self.schedule_save();
    }

    fn populate_widget_tiles(&self, st: &SettingsState) {
        st.set_widget_tiles(widget_tiles(&self.settings.borrow()));
    }

    fn populate_layout(&self, st: &SettingsState) {
        self.populate_widget_tiles(st);
        let s = self.settings.borrow();
        let (lo, hi) = s.size_range();
        st.set_top_layout(s.layout == Layout::Top);
        st.set_size_min(lo);
        st.set_size_max(hi);
        st.set_size_percent(s.size_percent());
        let list = monitors::all();
        let labels: Vec<SharedString> = list.iter().enumerate().map(|(i, m)| m.label(i).into()).collect();
        let index =
            s.monitor.as_deref().and_then(|d| list.iter().position(|m| m.device.eq_ignore_ascii_case(d))).unwrap_or(0);
        st.set_monitors(ModelRc::new(VecModel::from(labels)));
        st.set_monitor_index(index as i32);
    }

    fn on_settings_changed(self: &Rc<Self>, key: &str) {
        let Some(sw) = self.settings_window.borrow().as_ref().map(|sw| sw.clone_strong()) else { return };
        let st = sw.global::<SettingsState>();
        match key {
            "visibility" => self.set_hud_visible(st.get_hud_visible()),
            "startup" => {
                startup::set_enabled(st.get_open_at_login());
                // Show what Windows actually has, in case the change didn't take.
                st.set_open_at_login(startup::is_enabled());
            }
            "fit" => self.settings.borrow_mut().fit_windows = st.get_fit_windows(),
            "layout" => self.set_layout(if st.get_top_layout() { Layout::Top } else { Layout::Left }),
            "size" => {
                // The slider fires on every pixel of a drag; only a new half-percent step moves the HUD.
                let v = (st.get_size_percent() * 2.0).round() / 2.0;
                if v == self.settings.borrow().size_percent() {
                    return;
                }
                {
                    let mut s = self.settings.borrow_mut();
                    match s.layout {
                        Layout::Top => s.top_size_percent = v,
                        Layout::Left => s.side_size_percent = v,
                    }
                    s.normalize();
                }
                self.size_before.borrow_mut().get_or_insert_with(|| self.work_areas());
                self.reposition();
                // Refit other windows once, after the drag settles, rather than on every step.
                let w = Rc::downgrade(self);
                self.size_timer.start(TimerMode::SingleShot, Duration::from_millis(300), move || {
                    if let Some(a) = w.upgrade()
                        && let Some(before) = a.size_before.borrow_mut().take()
                    {
                        a.reflow_after(before);
                    }
                });
            }
            "monitor" => {
                let list = monitors::all();
                let device = list.get(st.get_monitor_index().max(0) as usize).map(|m| m.device.clone());
                self.settings.borrow_mut().monitor = device;
                self.redock();
            }
            "theme" => {
                self.settings.borrow_mut().appearance = match st.get_appearance() {
                    1 => Appearance::Light,
                    2 => Appearance::Dark,
                    _ => Appearance::Auto,
                };
                self.apply_theme();
            }
            "clock" => {
                {
                    let mut s = self.settings.borrow_mut();
                    s.use_24h = st.get_use_24h();
                    s.show_seconds = st.get_show_seconds();
                }
                self.tick();
                self.render_weather();
            }
            "weather" => {
                {
                    let mut s = self.settings.borrow_mut();
                    s.weather_city = st.get_city().trim().to_string();
                    s.celsius = st.get_units() == 0;
                }
                self.send_weather(weather::Command::Configure(self.weather_config()));
            }
            "news" => {
                let pick = |list: &'static [news::Source], i: i32| list[(i.max(0) as usize).min(list.len() - 1)].id;
                let (curated, top) = (pick(news::CURATED, st.get_curated_index()), pick(news::TOP, st.get_top_index()));
                {
                    let mut s = self.settings.borrow_mut();
                    if s.news_curated != curated {
                        self.news_curated.borrow_mut().clear();
                        // Show the tab that changed.
                        self.hud.global::<HudState>().set_news_tab(0);
                    }
                    if s.news_top != top {
                        self.news_top.borrow_mut().clear();
                        self.hud.global::<HudState>().set_news_tab(1);
                    }
                    s.news_curated = curated.into();
                    s.news_top = top.into();
                }
                self.hud.global::<HudState>().set_news_placeholder("Loading headlines…".into());
                self.render_news();
                if let Some(tx) = self.news_tx.borrow().as_ref() {
                    let _ = tx.send(news::Command::Sources(curated.into(), top.into()));
                }
            }
            "media" => {
                let v = st.get_prefer_spotify();
                self.settings.borrow_mut().prefer_spotify = v;
                self.send_media(media::Command::PreferSpotify(v));
            }
            "ai" => {
                self.settings.borrow_mut().ai.show_button = st.get_ai_button();
                self.apply_ai_prefs();
            }
            _ => {}
        }
        self.schedule_save();
    }
}

/// The Widgets overview (Settings page 30): every widget with its blurb and where it's placed.
pub fn widget_tiles(s: &Settings) -> ModelRc<WidgetTile> {
    let tiles: Vec<WidgetTile> = crate::settings::WIDGETS
        .iter()
        .map(|(id, name)| {
            let (placement, shown) = widget_placement(s, id);
            WidgetTile {
                id: (*id).into(),
                name: (*name).into(),
                detail: crate::feeds::settings::blurb(id).into(),
                page: widget_page(id),
                placement: placement.into(),
                shown,
            }
        })
        .collect();
    ModelRc::new(VecModel::from(tiles))
}

/// The Settings page for a widget (see `ui/settings.slint`), or -1 when it has none.
fn widget_page(id: &str) -> i32 {
    match id {
        "weather" => 3,
        "music" => 4,
        "news" => 5,
        "sports" => 7,
        "youtube" => 8,
        "reddit" => 9,
        "netflix" => 10,
        "stocks" => 11,
        _ => -1,
    }
}

/// Whether a widget is on the HUD, for its badge on the Widgets overview. Both layouts show the same
/// list, so there's one answer. Read from the arrangement (not what's on screen right now), so a hidden
/// HUD still shows what's placed.
fn widget_placement(s: &Settings, id: &str) -> (&'static str, bool) {
    if s.sidebar_order.iter().any(|w| w == id) { ("On HUD", true) } else { ("Not shown", false) }
}

/// Brings a headline model up to date. New articles replace the list; otherwise only rows whose
/// age label changed ("12m" → "13m") are touched, so unchanged rows aren't rebuilt or redrawn.
/// Diffs `rows` into `current` when it is already a `VecModel`; otherwise installs a new one.
fn sync_model<T: Clone + PartialEq + 'static>(current: ModelRc<T>, set: impl FnOnce(ModelRc<T>), rows: Vec<T>) {
    match current.as_any().downcast_ref::<VecModel<T>>() {
        Some(model) => crate::feeds::sync(model, rows),
        None => set(ModelRc::new(VecModel::from(rows))),
    }
}

fn sync_news_rows(model: &VecModel<NewsData>, articles: &[news::Article], now: i64) {
    let row = |a: &news::Article| NewsData {
        title: a.title.as_str().into(),
        age: news::age_label(a.published, now).into(),
        publisher: a.publisher.as_deref().unwrap_or("").into(),
        url: a.url.as_str().into(),
    };
    let same_articles = model.row_count() == articles.len()
        && articles.iter().enumerate().all(|(i, a)| model.row_data(i).is_some_and(|r| r.url == a.url.as_str()));
    if !same_articles {
        model.set_vec(articles.iter().map(row).collect::<Vec<_>>());
        return;
    }
    for (i, a) in articles.iter().enumerate() {
        let fresh = row(a);
        if model.row_data(i).is_some_and(|r| r != fresh) {
            model.set_row_data(i, fresh);
        }
    }
}

/// A layout fold in progress.
struct Morph {
    start: Instant,
    from: RECT,
    to: RECT,
    /// Whether the horizontal edges move first (folding into the Sidebar) or the vertical ones.
    lead_horizontal: bool,
    /// Tells the vsync thread to stop.
    stop: Arc<AtomicBool>,
    /// Frames drawn and the longest gap between two, for the one-line timing note in the log.
    frames: u32,
    last: Instant,
    worst_gap_ms: f32,
}

const MORPH_MS: f32 = 560.0;
/// The leading axis moves over the first 74% of the fold, the other over the last 78%. The long overlap
/// turns the corner into one arc instead of two moves. Tuned by simulating every frame at 164 Hz: this
/// is the gentlest jolt (largest change in speed between frames) of the curves and overlaps tried.
const LEAD_END: f32 = 0.74;
const FOLLOW_START: f32 = 0.22;

/// The window's rectangle at progress `u` (0..1) of a fold from `from` to `to`.
fn morph_rect(from: RECT, to: RECT, lead_horizontal: bool, u: f32) -> RECT {
    let lead = smootherstep(u / LEAD_END);
    let follow = smootherstep((u - FOLLOW_START) / (1.0 - FOLLOW_START));
    let (h, v) = if lead_horizontal { (lead, follow) } else { (follow, lead) };
    let lerp = |a: i32, b: i32, k: f32| a + ((b - a) as f32 * k).round() as i32;
    RECT {
        left: lerp(from.left, to.left, h),
        right: lerp(from.right, to.right, h),
        top: lerp(from.top, to.top, v),
        bottom: lerp(from.bottom, to.bottom, v),
    }
}

/// Ken Perlin's smootherstep: starts and stops with zero speed *and* zero acceleration, so the motion
/// has no jolt at either end, and peaks at 1.9× the average speed (an ease-in-out cubic peaks at 3×).
fn smootherstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (6.0 * t - 15.0) + 10.0)
}

fn today_key() -> String {
    let t = locale::now();
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

fn format_ms(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn playlist_rows(list: &[Playlist]) -> ModelRc<PlaylistRow> {
    let rows: Vec<PlaylistRow> = playlists::ordered(list)
        .into_iter()
        .map(|p| PlaylistRow { name: p.name.as_str().into(), uri: p.uri.as_str().into(), pinned: p.pinned })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// Opens a Spotify URI in the desktop app, or its web page if the app isn't installed.
fn open_in_spotify(uri: &str, web: &str) {
    if !shell_open(uri) {
        shell_open(web);
    }
}

fn fetch_playlist_title(uri: String) {
    let page = playlists::web_url(&uri).replace(':', "%3A").replace('/', "%2F");
    let url = format!("https://open.spotify.com/oembed?url={page}");
    let spawned = std::thread::Builder::new().name("playlist-title".into()).spawn(move || match http::get(&url) {
        Ok(body) => {
            if let Some(title) = playlists::title_from_oembed(&body) {
                post(move |a| a.set_playlist_title(&uri, title));
            }
        }
        Err(e) => storage::log(format!("could not fetch playlist title: {e}")),
    });
    if let Err(e) = spawned {
        storage::log(format!("could not start playlist title lookup: {e}"));
    }
}

fn open_spotify() {
    if !shell_open("spotify:") {
        shell_open("https://open.spotify.com");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_durations() {
        assert_eq!(super::format_ms(61_000), "1:01");
        assert_eq!(super::format_ms(3_725_000), "1:02:05");
    }

    #[test]
    fn layout_fold_is_one_continuous_motion() {
        use super::morph_rect;
        use windows::Win32::Foundation::RECT;
        let r = |l, t, rt, b| RECT { left: l, top: t, right: rt, bottom: b };
        // Top bar on a 2560×1440 screen folding into a 358px sidebar.
        let (from, to) = (r(0, 0, 2560, 127), r(0, 0, 358, 1440));

        // Starts and lands exactly; overshoot past the end is clamped.
        assert_eq!(morph_rect(from, to, true, 0.0), from);
        assert_eq!(morph_rect(from, to, true, 1.0), to);
        assert_eq!(morph_rect(from, to, true, 1.4), to);

        // Width only ever shrinks and height only ever grows: no wobble.
        let frames: Vec<RECT> = (0..=240).map(|i| morph_rect(from, to, true, i as f32 / 240.0)).collect();
        for w in frames.windows(2) {
            assert!(w[1].right <= w[0].right && w[1].bottom >= w[0].bottom);
        }
        // The old version stopped dead at the corner. Now both edges are moving mid-fold.
        let (a, b) = (morph_rect(from, to, true, 0.44), morph_rect(from, to, true, 0.48));
        assert!(b.right < a.right && b.bottom > a.bottom, "{a:?} → {b:?}");

        // Folding back to the top: the height leads instead.
        let early = morph_rect(to, from, false, 0.2);
        assert!(early.bottom < to.bottom && early.right == to.right);
    }

    #[test]
    fn widget_badges_follow_the_arrangement() {
        use super::{widget_page, widget_placement};
        let s = crate::settings::Settings {
            hud_visible: false, // placement is shown even while the HUD is hidden
            sidebar_order: vec!["weather".into(), "news".into()],
            ..Default::default()
        };
        // One list drives both layouts, so a widget is either on the HUD or not.
        assert_eq!(widget_placement(&s, "weather"), ("On HUD", true));
        assert_eq!(widget_placement(&s, "news"), ("On HUD", true));
        assert_eq!(widget_placement(&s, "music"), ("Not shown", false));
        assert_eq!(widget_placement(&s, "stocks"), ("Not shown", false));
        // Every widget but Tasks & Notes has a settings page, each a different one.
        let pages: Vec<i32> = crate::settings::WIDGETS.iter().map(|(id, _)| widget_page(id)).collect();
        assert_eq!(pages.iter().filter(|p| **p < 0).count(), 1);
        let mut unique = pages.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), pages.len());
    }

    #[test]
    fn news_rows_update_in_place() {
        use slint::{Model, VecModel};
        let article = |url: &str, published| crate::services::news::Article {
            title: url.into(),
            url: url.into(),
            published: Some(published),
            publisher: None,
        };
        let list = vec![article("https://a", 0), article("https://b", 600)];
        let model = VecModel::default();
        super::sync_news_rows(&model, &list, 660);
        assert_eq!(model.row_count(), 2);
        assert_eq!(model.row_data(0).unwrap().age, "11m");

        // A minute later only the ages change.
        super::sync_news_rows(&model, &list, 720);
        assert_eq!(model.row_data(0).unwrap().age, "12m");
        assert_eq!(model.row_data(1).unwrap().age, "2m");

        // New articles replace the list.
        super::sync_news_rows(&model, &[article("https://c", 720)], 720);
        assert_eq!(model.row_count(), 1);
        assert_eq!(model.row_data(0).unwrap().url, "https://c");
    }
}
