//! Developer tool (debug builds only): renders the HUD offscreen with live data and writes PNGs, so
//! widget layouts can be checked without docking a real AppBar or touching other windows.
//!
//! `lifehud --snapshot <out-dir> [widget] [light]` writes `<widget>-top.png` (the widget in a Top HUD
//! slot) and `<widget>-side.png` (the widget in the Sidebar) for each requested widget.

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

use crate::settings::{Appearance, Layout, Settings};
use crate::{HudState, HudWindow, Theme, theme};

struct Offscreen(Rc<MinimalSoftwareWindow>);

thread_local! {
    /// The theme's acrylic tint (0xAARRGGBB). In the app, Windows paints it under the window;
    /// offscreen we composite it over the stand-in wallpaper ourselves.
    static TINT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

pub fn run(args: &[String]) {
    let out = std::path::PathBuf::from(args.first().cloned().unwrap_or_else(|| ".".into()));
    // Options: "light", "empty" (no data: unconfigured/loading states), "h=<px>" (Top HUD height,
    // default 96) and "w=<px>" (Sidebar width, default 340).
    let option = |prefix: &str| args.iter().find_map(|a| a.strip_prefix(prefix)?.parse::<u32>().ok());
    let (top_h, side_w) = (option("h=").unwrap_or(96), option("w=").unwrap_or(340));
    let light = args.iter().any(|a| a == "light");
    // "theme=<id>" renders with that palette (default Classic).
    let theme_id = args.iter().find_map(|a| a.strip_prefix("theme=")).unwrap_or(theme::CLASSIC).to_string();
    let empty = args.iter().any(|a| a == "empty");
    let widgets: Vec<&str> = args
        .iter()
        .skip(1)
        .map(|s| s.as_str())
        .filter(|s| {
            !matches!(*s, "light" | "empty") && !s.starts_with("h=") && !s.starts_with("w=") && !s.starts_with("theme=")
        })
        .collect();
    let suffix = format!(
        "{}{}",
        if empty { "-empty" } else { "" },
        if top_h != 96 || side_w != 340 { format!("-{top_h}x{side_w}") } else { String::new() }
    );
    let _ = std::fs::create_dir_all(&out);
    // Keep caches and logs away from the real app's data folder. Still single-threaded here.
    unsafe { std::env::set_var("LOCALAPPDATA", out.join("data")) };

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).expect("platform");
    crate::platform::init_worker_thread();
    if widgets.first() == Some(&"settings") {
        let pages: Vec<i32> = widgets[1..].iter().filter_map(|p| p.parse().ok()).collect();
        return settings_pages(&window, &out, &pages, light, &theme_id);
    }

    let hud = HudWindow::new().expect("hud");
    let palette = theme::resolve(&theme_id, if light { Appearance::Light } else { Appearance::Dark });
    TINT.with(|t| t.set(palette.tint));
    theme::apply(&hud.global::<Theme>(), &palette);
    let feeds = crate::feeds::Hub::new(&hud);
    hud.show().expect("show");

    let widgets = if widgets.is_empty() { vec!["sports"] } else { widgets };
    for id in widgets {
        // The widget second, left of the clock (normalize drops it if it's already listed).
        let mut settings = Settings {
            sidebar_order: ["weather", id, "news", "music"].map(String::from).to_vec(),
            ..Default::default()
        };
        settings.normalize();
        if empty {
            // Nothing followed and nothing fetched yet.
            settings.sports.leagues.clear();
            settings.youtube.channels.clear();
            settings.reddit.subreddits.clear();
            settings.stocks.watchlist.clear();
            feeds.configure(&settings);
        } else {
            load(&feeds, id, &settings);
        }

        let state = hud.global::<HudState>();
        if id == "music" {
            // Something playing, with a player whose volume we can control.
            state.set_has_media(true);
            state.set_has_timeline(true);
            state.set_is_playing(true);
            state.set_is_spotify(true);
            state.set_track_title("Midnight City".into());
            state.set_track_artist("M83".into());
            state.set_media_progress(0.42);
            state.set_elapsed("1:45".into());
            state.set_remaining("-2:19".into());
            state.set_volume_available(true);
            state.set_volume(0.62);
        }
        // Top HUD: 1920×96, the widget in the left slot.
        settings.layout = Layout::Top;
        feeds.apply_layout(&settings);
        state.set_vertical(false);
        for tab in [0, 1] {
            set_tab(&hud, id, tab);
            render(&window, PhysicalSize::new(1920, top_h), &out.join(format!("{id}-top-{tab}{suffix}.png")), light);
        }
        if id == "music" {
            state.set_volume_open(true);
            render(&window, PhysicalSize::new(1920, top_h), &out.join(format!("{id}-top-volume{suffix}.png")), light);
            state.set_volume_open(false);
        }
        // Sidebar: 340×1080 with the widget under the weather card.
        settings.layout = Layout::Left;
        let kind = crate::settings::widget_kind(id).unwrap_or(0) as i32;
        state.set_sidebar_order(ModelRc::new(VecModel::from(vec![0, kind, 2])));
        state.set_vertical(true);
        for tab in [0, 1] {
            set_tab(&hud, id, tab);
            render(&window, PhysicalSize::new(side_w, 1080), &out.join(format!("{id}-side-{tab}{suffix}.png")), light);
        }
        if id == "youtube" || id == "netflix" {
            // A video playing in a browser tab: the Now Watching card, in both layouts.
            state.set_watching(true);
            state.set_watching_site(id.into());
            state.set_watching_browser("Chrome".into());
            state.set_watching_playing(true);
            state.set_watching_can_seek(true);
            state.set_watching_progress(0.37);
            if id == "youtube" {
                state.set_watching_title("The Fermi Paradox — Where Are All The Aliens?".into());
                state.set_watching_subtitle("Kurzgesagt – In a Nutshell".into());
            } else {
                state.set_watching_title("Stranger Things".into());
                state.set_watching_subtitle("S5:E1 · The Crawl".into());
            }
            render(
                &window,
                PhysicalSize::new(side_w, 1080),
                &out.join(format!("{id}-side-watching{suffix}.png")),
                light,
            );
            settings.layout = Layout::Top;
            feeds.apply_layout(&settings);
            state.set_vertical(false);
            render(&window, PhysicalSize::new(1920, top_h), &out.join(format!("{id}-top-watching{suffix}.png")), light);
            state.set_watching(false);
            // A video docked into the widget (the black area is where Edge's window sits).
            let player = hud.global::<crate::PlayerState>();
            player.set_active(true);
            player.set_site(id.into());
            player.set_loading(true);
            player.set_title(
                if id == "youtube" { "The Fermi Paradox — Where Are All The Aliens?" } else { "Stranger Things" }
                    .into(),
            );
            render(&window, PhysicalSize::new(1920, top_h), &out.join(format!("{id}-top-docked{suffix}.png")), light);
            settings.layout = Layout::Left;
            feeds.apply_layout(&settings);
            state.set_vertical(true);
            render(&window, PhysicalSize::new(side_w, 1080), &out.join(format!("{id}-side-docked{suffix}.png")), light);
            eprintln!(
                "player area: {}x{} at ({}, {})",
                player.get_width(),
                player.get_height(),
                player.get_x(),
                player.get_y()
            );
            player.set_active(false);
        }
    }
}

/// `--snapshot <dir> settings 1 7`: Settings pages, 780×(tall enough for the page).
fn settings_pages(window: &MinimalSoftwareWindow, out: &std::path::Path, pages: &[i32], light: bool, theme_id: &str) {
    let sw = crate::SettingsWindow::new().expect("settings");
    let palette = theme::resolve(theme_id, if light { Appearance::Light } else { Appearance::Dark });
    TINT.with(|t| t.set(palette.tint));
    theme::apply(&sw.global::<Theme>(), &palette);
    let mut settings = Settings::default();
    settings.sports.favorites.push(crate::services::sports::Favorite {
        league: settings.sports.leagues[0].clone(),
        id: "1".into(),
        abbr: "TOR".into(),
        name: "Toronto Maple Leafs".into(),
    });
    // Three widgets: room for one more, so drops in are allowed. Fixed so the coordinates are stable.
    settings.sidebar_order = ["weather", "music", "today"].map(String::from).to_vec();
    crate::feeds::settings::populate(&sw, &settings);
    sw.global::<crate::SettingsState>().set_widget_tiles(crate::app::widget_tiles(&settings));
    sw.show().expect("show");
    for page in pages {
        sw.global::<crate::SettingsState>().set_page(*page);
        let tall = if *page == 30 { 1700 } else { 1000 };
        render(window, PhysicalSize::new(780, tall), &out.join(format!("settings-{page}.png")), light);
        if *page == 30 {
            // The arrange editor's other mode.
            let fs = sw.global::<crate::FeedSettings>();
            fs.set_arrange_mode(1 - fs.get_arrange_mode());
            render(window, PhysicalSize::new(780, tall), &out.join(format!("settings-{page}-b.png")), light);
            if fs.get_arrange_mode() == 1 {
                drag_script(window, &sw, out, light);
            }
        }
    }
}

/// What a gesture is, where it starts and ends, the callback it should fire and an optional
/// mid-drag screenshot.
type Gesture = (&'static str, (f32, f32), (f32, f32), &'static str, Option<&'static str>);

/// Real pointer gestures on the Sidebar arrange editor (default layout, Sports added; coordinates
/// from the 780-wide render), printing which callback each one fires.
fn drag_script(window: &MinimalSoftwareWindow, sw: &crate::SettingsWindow, out: &std::path::Path, light: bool) {
    use slint::LogicalPosition;
    use slint::platform::{PointerEventButton, WindowEvent};
    crate::feeds::settings::wire(sw);
    let fs = sw.global::<crate::FeedSettings>();
    let log = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    fs.on_place_card(move |a, b| l.borrow_mut().push(format!("place-card({a}, {b})")));
    let l = log.clone();
    fs.on_insert_card(move |id, at| l.borrow_mut().push(format!("insert-card({id}, {at})")));
    let l = log.clone();
    fs.on_remove_card(move |i| l.borrow_mut().push(format!("remove-card({i})")));
    let l = log.clone();
    fs.on_add_card(move |id| l.borrow_mut().push(format!("add-card({id})")));

    let w = window.window();
    let at = |x: f32, y: f32| LogicalPosition::new(x, y);
    let gesture = |from: (f32, f32), to: (f32, f32), shot: Option<&str>| {
        w.dispatch_event(WindowEvent::PointerMoved { position: at(from.0, from.1) });
        w.dispatch_event(WindowEvent::PointerPressed {
            position: at(from.0, from.1),
            button: PointerEventButton::Left,
        });
        for step in 1..=8 {
            let t = step as f32 / 8.0;
            let p = at(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            w.dispatch_event(WindowEvent::PointerMoved { position: p });
        }
        if let Some(name) = shot {
            render(window, PhysicalSize::new(780, 700), &out.join(name), light);
        }
        w.dispatch_event(WindowEvent::PointerReleased { position: at(to.0, to.1), button: PointerEventButton::Left });
        w.dispatch_event(WindowEvent::PointerExited);
    };
    // Sidebar view (Weather, Music, Tasks & Notes; coordinates from the 780-wide render).
    let side: [Gesture; 6] = [
        (
            "drag Stocks between Weather and Music",
            (544.0, 406.0),
            (320.0, 275.0),
            "insert-card(stocks, 1)",
            Some("drag-in.png"),
        ),
        ("drag Tasks & Notes to the top", (320.0, 450.0), (320.0, 200.0), "place-card(2, 0)", Some("drag-reorder.png")),
        ("drag Weather out to the palette", (320.0, 230.0), (544.0, 450.0), "remove-card(0)", Some("drag-out.png")),
        ("drop Music where it is", (320.0, 300.0), (322.0, 296.0), "", None),
        ("click Reddit", (544.0, 322.0), (544.0, 322.0), "add-card(reddit)", None),
        ("click Music's remove button", (369.0, 289.0), (369.0, 289.0), "remove-card(1)", None),
    ];
    // Top HUD view of the same list: Weather, Music | clock | Tasks & Notes.
    let top: [Gesture; 5] = [
        (
            "drag Stocks in just right of Music",
            (430.0, 316.0),
            (450.0, 188.0),
            "insert-card(stocks, 2)",
            Some("top-drag-in.png"),
        ),
        (
            "drag Weather past the clock to the end",
            (301.0, 188.0),
            (725.0, 188.0),
            "place-card(0, 3)",
            Some("top-reorder.png"),
        ),
        ("drag Tasks & Notes off the bar", (635.0, 188.0), (430.0, 300.0), "remove-card(2)", None),
        ("click Reddit", (678.0, 272.0), (678.0, 272.0), "add-card(reddit)", None),
        ("click Music's remove button", (438.0, 173.0), (438.0, 173.0), "remove-card(1)", None),
    ];
    let mut failed = 0;
    for (mode, cases) in [(1, &side[..]), (0, &top[..])] {
        fs.set_arrange_mode(mode);
        for &(what, from, to, want, shot) in cases {
            log.borrow_mut().clear();
            gesture(from, to, shot);
            let got = log.borrow().join("; ");
            let ok = got == want;
            failed += usize::from(!ok);
            eprintln!(
                "{} {what}: {}",
                if ok { "PASS" } else { "FAIL" },
                if got.is_empty() { "(nothing)" } else { &got }
            );
        }
    }
    eprintln!("drag script: {failed} failed");
}

fn set_tab(hud: &HudWindow, id: &str, tab: i32) {
    match id {
        "sports" => hud.global::<crate::SportsState>().set_tab(tab),
        "youtube" => hud.global::<crate::YouTubeState>().set_tab(tab),
        "reddit" => hud.global::<crate::RedditState>().set_tab(tab),
        "netflix" => hud.global::<crate::NetflixState>().set_tab(tab),
        "stocks" => hud.global::<crate::StocksState>().set_tab(tab),
        _ => {}
    }
}

/// Runs a widget's real fetch synchronously and feeds the results to its controller.
fn load(feeds: &crate::feeds::Hub, id: &str, settings: &Settings) {
    let started = Instant::now();
    if id == "sports" {
        feeds.sports.configure(settings);
        let updates = std::cell::RefCell::new(Vec::new());
        crate::services::sports::Fetcher::new().run(&settings.sports, &|u| updates.borrow_mut().push(u));
        for u in updates.into_inner() {
            feeds.sports.on_update(u);
        }
    }
    if id == "youtube" {
        let mut s = settings.clone();
        for handle in ["@mkbhd", "@Fireship", "@veritasium"] {
            if let Ok(ch) = crate::services::youtube::resolve_channel(handle) {
                s.youtube.channels.push(ch);
            }
        }
        s.youtube.saved.push(crate::services::youtube::resolve_video("dQw4w9WgXcQ").expect("oembed"));
        feeds.youtube.configure(&s);
        let updates = std::cell::RefCell::new(Vec::new());
        crate::services::youtube::Fetcher::new().run(&s.youtube, &|u| updates.borrow_mut().push(u));
        for u in updates.into_inner() {
            feeds.youtube.on_update(u);
        }
    }
    if id == "reddit" {
        feeds.reddit.configure(settings);
        let updates = std::cell::RefCell::new(Vec::new());
        crate::services::reddit::Fetcher::new().run(&settings.reddit, &|u| updates.borrow_mut().push(u));
        for u in updates.into_inner() {
            feeds.reddit.on_update(u);
        }
    }
    if id == "netflix" {
        let updates = std::cell::RefCell::new(Vec::new());
        crate::services::netflix::fetch(&|u| updates.borrow_mut().push(u));
        for u in updates.into_inner() {
            feeds.netflix.on_update(u);
        }
    }
    if id == "stocks" {
        let updates = std::cell::RefCell::new(Vec::new());
        crate::services::stocks::fetch(&settings.stocks, &|u| updates.borrow_mut().push(u));
        for u in updates.into_inner() {
            feeds.stocks.on_update(u);
        }
    }
    eprintln!("{id}: loaded in {:?}", started.elapsed());
}

fn render(window: &MinimalSoftwareWindow, size: PhysicalSize, path: &std::path::Path, light: bool) {
    window.set_size(size);
    // Let property animations (tab underline, hover fades) settle before drawing.
    for _ in 0..12 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let _ = window.draw_if_needed(|_| {});
        std::thread::sleep(Duration::from_millis(50));
    }
    let (w, h) = (size.width as usize, size.height as usize);
    let mut buf = vec![PremultipliedRgbaColor::default(); w * h];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, w);
    });
    // Stand-in for the acrylic backdrop: a soft wallpaper-like gradient under the glass.
    let mut rgb = Vec::with_capacity(w * h * 3);
    for (i, px) in buf.iter().enumerate() {
        let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
        let bg = if light {
            [228.0 - 20.0 * y, 232.0 - 10.0 * x, 240.0]
        } else {
            [26.0 + 18.0 * x, 30.0 + 8.0 * y, 44.0 + 20.0 * (1.0 - x)]
        };
        let tint = TINT.with(|t| t.get());
        let ta = (tint >> 24) as f32 / 255.0;
        let tc = [(tint >> 16) & 255, (tint >> 8) & 255, tint & 255].map(|c| c as f32);
        let bg = [0, 1, 2].map(|i| tc[i] * ta + bg[i] * (1.0 - ta));
        let a = px.alpha as f32 / 255.0;
        for (c, b) in [px.red, px.green, px.blue].into_iter().zip(bg) {
            rgb.push((c as f32 + b * (1.0 - a)).round().clamp(0.0, 255.0) as u8);
        }
    }
    std::fs::write(path, png(size.width, size.height, &rgb)).expect("write png");
    eprintln!("wrote {}", path.display());
}

/// A minimal PNG encoder (RGB8, stored deflate blocks): no image crate for a debug tool.
fn png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc(&body).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for row in rgb.chunks(width as usize * 3) {
        raw.push(0);
        raw.extend(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push((i == blocks.len() - 1) as u8);
        z.extend((block.len() as u16).to_le_bytes());
        z.extend((!(block.len() as u16)).to_le_bytes());
        z.extend(*block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
