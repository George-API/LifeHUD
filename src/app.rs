//! The application controller: owns state, wires UI callbacks, and bridges the platform and
//! background services into the Slint windows. Everything here runs on the UI thread.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::mpsc::Sender;
use std::time::Duration;

use slint::{
    ComponentHandle, Image, ModelRc, PhysicalPosition, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode,
    VecModel,
};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

use crate::platform::appbar::{AppBar, DockRequest, ShellEvent};
use crate::platform::tray::{MenuState, Tray, TrayEvent};
use crate::platform::{effects, hwnd_of, monitors, reflow, shell_open, startup};
use crate::services::{media, news, weather};
use crate::settings::{Appearance, Layout, Settings};
use crate::tasks::TaskList;
use crate::{DayData, HourData, HudState, HudWindow, NewsData, SettingsState, SettingsWindow, Theme};
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
    tasks: RefCell<TaskList>,
    weather_tx: RefCell<Option<Sender<weather::Command>>>,
    media_tx: RefCell<Option<Sender<media::Command>>>,
    news_tx: RefCell<Option<Sender<(String, String)>>>,
    news_curated: RefCell<Vec<news::Article>>,
    news_top: RefCell<Vec<news::Article>>,
    weather: RefCell<Option<(weather::Snapshot, bool)>>,
    weather_status: RefCell<String>,
    today: RefCell<String>,
    last_minute: Cell<u16>,
    clock_timer: Timer,
    save_timer: Timer,
    notes_timer: Timer,
    notes_status_timer: Timer,
    display_timer: Timer,
    morph_timer: Timer,
}

impl App {
    pub fn new() -> Result<Rc<Self>, slint::PlatformError> {
        let (settings, first_run) = Settings::load();
        let app = Rc::new(Self {
            hud: HudWindow::new()?,
            settings: RefCell::new(settings),
            first_run,
            hwnd: Cell::new(None),
            appbar: RefCell::new(None),
            settings_window: RefCell::new(None),
            tray: RefCell::new(None),
            tasks: RefCell::new(TaskList::load()),
            weather_tx: RefCell::new(None),
            media_tx: RefCell::new(None),
            news_tx: RefCell::new(None),
            news_curated: RefCell::new(Vec::new()),
            news_top: RefCell::new(Vec::new()),
            weather: RefCell::new(None),
            weather_status: RefCell::new(String::new()),
            today: RefCell::new(today_key()),
            last_minute: Cell::new(u16::MAX),
            clock_timer: Timer::default(),
            save_timer: Timer::default(),
            notes_timer: Timer::default(),
            notes_status_timer: Timer::default(),
            display_timer: Timer::default(),
            morph_timer: Timer::default(),
        });
        APP.with(|a| *a.borrow_mut() = Rc::downgrade(&app));
        Ok(app)
    }

    pub fn start(self: &Rc<Self>) {
        self.apply_theme();
        self.wire_hud();
        self.start_clock();
        self.load_notes();
        self.refresh_tasks();
        self.start_services();
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
        state.on_weather_clicked(with_app!(|a: &Rc<App>| a.open_settings(Some(3))));
        state.on_hide_hud(with_app!(|a: &Rc<App>| a.set_hud_visible(false)));
        state.on_toggle_layout(with_app!(|a: &Rc<App>| {
            let next = if a.settings.borrow().layout == Layout::Top { Layout::Left } else { Layout::Top };
            a.set_layout(next);
        }));

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
        state.on_open_spotify(open_spotify);

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
                if self.settings.borrow().theme == theme::CLASSIC
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
    }

    pub fn set_layout(self: &Rc<Self>, layout: Layout) {
        if self.settings.borrow().layout == layout || self.morph_timer.running() {
            return;
        }
        let animate = self.settings.borrow().hud_visible && self.hwnd.get().is_some();
        if !animate {
            self.commit_layout(layout);
            self.redock();
            return;
        }

        // 1) Fade the widgets out. 2) Fold the glass from one edge to the other.
        // 3) Claim the new edge and fade the widgets back in.
        self.hud.global::<HudState>().set_transitioning(true);
        let w = Rc::downgrade(self);
        Timer::single_shot(Duration::from_millis(190), move || {
            let Some(a) = w.upgrade() else { return };
            let Some(hwnd) = a.hwnd.get() else { return };
            let mut from = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut from);
            }
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
            // The corner where the bar folds: keep one dimension, take the other from the target.
            let corner = match layout {
                Layout::Left => RECT {
                    left: from.left,
                    top: from.top,
                    right: from.left + (target.right - target.left),
                    bottom: from.bottom,
                },
                Layout::Top => RECT {
                    left: from.left,
                    top: from.top,
                    right: from.right,
                    bottom: from.top + (target.bottom - target.top),
                },
            };
            a.run_morph(vec![(from, corner, 170.0), (corner, target, 230.0)]);
        });
    }

    fn commit_layout(&self, layout: Layout) {
        self.settings.borrow_mut().layout = layout;
        self.hud.global::<HudState>().set_vertical(layout == Layout::Left);
        self.schedule_save();
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            self.populate_layout(&sw.global::<SettingsState>());
        }
    }

    fn run_morph(self: &Rc<Self>, phases: Vec<(RECT, RECT, f32)>) {
        let start = std::time::Instant::now();
        let total: f32 = phases.iter().map(|p| p.2).sum();
        let w = Rc::downgrade(self);
        self.morph_timer.start(TimerMode::Repeated, Duration::from_millis(12), move || {
            let Some(a) = w.upgrade() else { return };
            let mut t = start.elapsed().as_secs_f32() * 1000.0;
            if t >= total {
                a.morph_timer.stop();
                a.finish_morph();
                return;
            }
            for (from, to, dur) in &phases {
                if t <= *dur {
                    let k = ease_in_out(t / dur);
                    let lerp = |x: i32, y: i32| x + ((y - x) as f32 * k).round() as i32;
                    let rc = RECT {
                        left: lerp(from.left, to.left),
                        top: lerp(from.top, to.top),
                        right: lerp(from.right, to.right),
                        bottom: lerp(from.bottom, to.bottom),
                    };
                    if let Some(appbar) = a.appbar.borrow().as_ref() {
                        appbar.move_freely(rc);
                    }
                    return;
                }
                t -= dur;
            }
        });
    }

    fn finish_morph(self: &Rc<Self>) {
        self.redock();
        let w = Rc::downgrade(self);
        Timer::single_shot(Duration::from_millis(80), move || {
            if let Some(a) = w.upgrade() {
                a.hud.global::<HudState>().set_transitioning(false);
            }
        });
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
        if let Some(sw) = self.settings_window.borrow().as_ref() {
            theme::apply(&sw.global::<Theme>(), &palette);
            sw.global::<SettingsState>().set_themes(theme::swatches(&id, appearance));
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

        let prefer = self.settings.borrow().prefer_spotify;
        let tx = media::spawn(prefer, |update| {
            post(move |app| app.on_media(update));
        });
        *self.media_tx.borrow_mut() = Some(tx);
    }

    fn send_media(&self, cmd: media::Command) {
        if let Some(tx) = self.media_tx.borrow().as_ref() {
            let _ = tx.send(cmd);
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

        state.set_upcoming(ModelRc::new(VecModel::from(daily.iter().skip(1).take(3).cloned().collect::<Vec<_>>())));
        state.set_daily(ModelRc::new(VecModel::from(daily)));
        state.set_hourly(ModelRc::new(VecModel::from(hours)));
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
        let rows = |list: &[news::Article]| -> ModelRc<NewsData> {
            let rows: Vec<NewsData> = list
                .iter()
                .map(|a| NewsData {
                    title: a.title.as_str().into(),
                    age: news::age_label(a.published, now).into(),
                    publisher: a.publisher.as_deref().unwrap_or("").into(),
                    url: a.url.as_str().into(),
                })
                .collect();
            ModelRc::new(VecModel::from(rows))
        };
        state.set_news_curated(rows(&self.news_curated.borrow()));
        state.set_news_top(rows(&self.news_top.borrow()));
    }
    fn on_media(&self, u: media::Update) {
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
        st.set_themes(theme::swatches(&s.theme, s.appearance));
        st.set_appearance(match s.appearance {
            Appearance::Auto => 0,
            Appearance::Light => 1,
            Appearance::Dark => 2,
        });
        st.set_appearance_enabled(s.theme == theme::CLASSIC);
        st.set_use_24h(s.use_24h);
        st.set_show_seconds(s.show_seconds);
        st.set_city(s.weather_city.as_str().into());
        st.set_weather_status(self.weather_status.borrow().as_str().into());
        st.set_units(if s.celsius { 0 } else { 1 });
        st.set_prefer_spotify(s.prefer_spotify);
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

        let w = Rc::downgrade(self);
        st.on_changed(move |key| {
            if let Some(a) = w.upgrade() {
                a.on_settings_changed(&key);
            }
        });
        let w = Rc::downgrade(self);
        st.on_select_theme(move |id| {
            let Some(a) = w.upgrade() else { return };
            a.settings.borrow_mut().theme = id.to_string();
            a.schedule_save();
            a.apply_theme();
            if let Some(sw) = a.settings_window.borrow().as_ref() {
                sw.global::<SettingsState>().set_appearance_enabled(id == theme::CLASSIC);
            }
        });
        let w = Rc::downgrade(self);
        st.on_move_card(move |index, delta| {
            let Some(a) = w.upgrade() else { return };
            a.settings.borrow_mut().move_card(index.max(0) as usize, delta);
            a.apply_sidebar_order();
            if let Some(sw) = a.settings_window.borrow().as_ref() {
                sw.global::<SettingsState>().set_sidebar_names(a.sidebar_names());
            }
            a.schedule_save();
        });
        st.on_open_data_folder(|| {
            shell_open(&storage::data_dir().to_string_lossy());
        });

        // Closing frees the window entirely; it's rebuilt on demand.
        let w = Rc::downgrade(self);
        sw.window().on_close_requested(move || {
            if let Some(a) = w.upgrade() {
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

    fn apply_sidebar_order(&self) {
        let kinds = self.settings.borrow().sidebar_kinds();
        self.hud.global::<HudState>().set_sidebar_order(ModelRc::new(VecModel::from(kinds)));
    }

    fn sidebar_names(&self) -> ModelRc<SharedString> {
        let s = self.settings.borrow();
        let names: Vec<SharedString> = s
            .sidebar_order
            .iter()
            .filter_map(|id| {
                crate::settings::SIDEBAR_CARDS.iter().find(|(c, _)| c == id).map(|(_, n)| SharedString::from(*n))
            })
            .collect();
        ModelRc::new(VecModel::from(names))
    }

    fn populate_layout(&self, st: &SettingsState) {
        st.set_sidebar_names(self.sidebar_names());
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
            "startup" => startup::set_enabled(st.get_open_at_login()),
            "fit" => self.settings.borrow_mut().fit_windows = st.get_fit_windows(),
            "layout" => self.set_layout(if st.get_top_layout() { Layout::Top } else { Layout::Left }),
            "size" => {
                let v = (st.get_size_percent() * 2.0).round() / 2.0;
                {
                    let mut s = self.settings.borrow_mut();
                    match s.layout {
                        Layout::Top => s.top_size_percent = v,
                        Layout::Left => s.side_size_percent = v,
                    }
                    s.normalize();
                }
                let before = self.work_areas();
                self.reposition();
                self.reflow_after(before);
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
                    let _ = tx.send((curated.into(), top.into()));
                }
            }
            "media" => {
                let v = st.get_prefer_spotify();
                self.settings.borrow_mut().prefer_spotify = v;
                self.send_media(media::Command::PreferSpotify(v));
            }
            _ => {}
        }
        self.schedule_save();
    }
}

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
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
}
