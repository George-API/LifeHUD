//! The Search & AI panel (UI thread). The first tab searches Google in the browser; the others chat
//! with ChatGPT, Claude or Gemini through `services::ai`. Conversations live in memory only and are
//! gone when Life HUD quits. API keys are read from Windows Credential Manager only when sending.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, SharedString, Timer, VecModel};
use windows::Win32::Foundation::RECT;

use crate::platform::{credentials, effects, hwnd_of, monitors};
use crate::services::ai::{self, Finish, Provider, Role, Turn, Update};
use crate::services::http;
use crate::theme::Palette;
use crate::{AiProviderRow, ChatMessage, ChatState, ChatWindow, Theme, storage};

/// Logical size of the panel (matches `ChatWindow` in ui/chat.slint).
const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 560.0;
/// Longest reply kept, in characters. Beyond this the reply is cut off and marked as such.
const MAX_REPLY_CHARS: usize = 120_000;

/// The panel's tabs, left to right: Google search, then the AI providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Search,
    Ai(Provider),
}

const TABS: [Tab; 4] = [Tab::Search, Tab::Ai(Provider::ChatGpt), Tab::Ai(Provider::Claude), Tab::Ai(Provider::Gemini)];

fn tab_at(i: usize) -> Tab {
    TABS[i.min(TABS.len() - 1)]
}

#[cfg(test)]
fn tab_of(provider: Provider) -> usize {
    TABS.iter().position(|t| *t == Tab::Ai(provider)).unwrap_or(0)
}

/// Google results for a query, in the browser.
fn search_url(query: &str) -> String {
    format!("https://www.google.com/search?q={}", http::encode(query))
}

thread_local! {
    static CHAT: RefCell<Weak<Chat>> = const { RefCell::new(Weak::new()) };
}

/// Where to place the panel: next to the docked HUD, or centred when the HUD is hidden.
pub struct Anchor {
    /// The HUD window's rectangle in physical pixels, if it's on screen.
    pub hud: Option<RECT>,
    pub top_layout: bool,
}

struct Pending {
    provider: Provider,
    id: u64,
    handle: ai::Handle,
}

pub struct Chat {
    window: RefCell<Option<ChatWindow>>,
    rows: Rc<VecModel<ChatMessage>>,
    conversations: RefCell<[Vec<Turn>; 3]>,
    notices: RefCell<[String; 3]>,
    has_key: Cell<[bool; 3]>,
    models: RefCell<[String; 3]>,
    tab: Cell<usize>,
    pending: RefCell<Option<Pending>>,
    next_id: Cell<u64>,
}

impl Chat {
    pub fn new(models: [String; 3]) -> Rc<Self> {
        let chat = Rc::new(Self {
            window: RefCell::new(None),
            rows: Rc::new(VecModel::default()),
            conversations: RefCell::new(Default::default()),
            notices: RefCell::new(Default::default()),
            has_key: Cell::new([false; 3]),
            models: RefCell::new(models),
            tab: Cell::new(0),
            pending: RefCell::new(None),
            next_id: Cell::new(1),
        });
        CHAT.with(|c| *c.borrow_mut() = Rc::downgrade(&chat));
        chat
    }

    // ------------------------------------------------------------------ window

    pub fn is_open(&self) -> bool {
        self.window.borrow().is_some()
    }

    pub fn open(self: &Rc<Self>, palette: &Palette, anchor: Anchor, preferred_tab: Option<usize>) {
        self.refresh_keys();
        if let Some(cw) = self.window.borrow().as_ref() {
            let _ = cw.show();
            return;
        }
        self.tab.set(preferred_tab.unwrap_or(0).min(TABS.len() - 1));

        let cw = match ChatWindow::new() {
            Ok(cw) => cw,
            Err(e) => {
                storage::log(format!("could not open chat: {e}"));
                return;
            }
        };
        theme_window(&cw, palette);
        let st = cw.global::<ChatState>();
        st.set_messages(ModelRc::from(self.rows.clone()));
        let ai_tabs: Vec<SharedString> =
            TABS.iter().filter_map(|t| if let Tab::Ai(p) = t { Some(p.name().into()) } else { None }).collect();
        st.set_ai_tabs(ModelRc::new(VecModel::from(ai_tabs)));
        self.wire(&st);
        position(&cw, &anchor);
        if let Err(e) = cw.show() {
            storage::log(format!("could not show chat: {e}"));
            return;
        }
        *self.window.borrow_mut() = Some(cw);
        self.render();
        self.style_native(*palette, 0);
    }

    pub fn close(&self) {
        // Dropped on the next turn of the event loop, never from inside its own callback.
        if let Some(cw) = self.window.borrow().as_ref() {
            let _ = cw.hide();
        }
        let w = CHAT.with(|c| c.borrow().clone());
        Timer::single_shot(Duration::ZERO, move || {
            if let Some(c) = w.upgrade() {
                c.window.borrow_mut().take();
            }
        });
    }

    pub fn apply_theme(&self, palette: &Palette) {
        if let Some(cw) = self.window.borrow().as_ref() {
            theme_window(cw, palette);
            if let Some(hwnd) = hwnd_of(cw.window()) {
                effects::enable_acrylic(hwnd, palette.tint);
            }
        }
    }

    /// The native window appears a moment after `show`; style it once it exists.
    fn style_native(self: &Rc<Self>, palette: Palette, attempt: u32) {
        let hwnd = self.window.borrow().as_ref().and_then(|cw| hwnd_of(cw.window()));
        let Some(hwnd) = hwnd else {
            if attempt < 50 && self.is_open() {
                let w = Rc::downgrade(self);
                Timer::single_shot(Duration::from_millis(20), move || {
                    if let Some(c) = w.upgrade() {
                        c.style_native(palette, attempt + 1);
                    }
                });
            }
            return;
        };
        effects::hide_from_switcher(hwnd);
        effects::round_corners(hwnd);
        effects::enable_acrylic(hwnd, palette.tint);
        effects::bring_to_front(hwnd);
    }

    fn wire(self: &Rc<Self>, st: &ChatState) {
        let w = Rc::downgrade(self);
        macro_rules! with_chat {
            ($body:expr) => {{
                let w = w.clone();
                move || {
                    if let Some(c) = w.upgrade() {
                        #[allow(clippy::redundant_closure_call)]
                        ($body)(&c);
                    }
                }
            }};
        }
        {
            let w = w.clone();
            st.on_select_tab(move |i| {
                if let Some(c) = w.upgrade() {
                    c.tab.set((i.max(0) as usize).min(TABS.len() - 1));
                    c.render();
                }
            });
        }
        {
            let w = w.clone();
            st.on_send(move |text| {
                if let Some(c) = w.upgrade() {
                    c.send(&text);
                }
            });
        }
        {
            let w = w.clone();
            st.on_ask(move |tab, text| {
                if let Some(c) = w.upgrade() {
                    c.tab.set((tab.max(0) as usize).min(TABS.len() - 1));
                    c.render();
                    c.send(&text);
                }
            });
        }
        st.on_stop(with_chat!(|c: &Rc<Chat>| c.stop("Stopped.")));
        st.on_new_chat(with_chat!(|c: &Rc<Chat>| c.new_chat()));
        st.on_close(with_chat!(|c: &Rc<Chat>| c.close()));
        st.on_open_settings(|| crate::app::post(|a| a.open_settings(Some(crate::chat::SETTINGS_PAGE))));
    }

    // ------------------------------------------------------------------ conversation

    pub fn set_models(&self, models: [String; 3]) {
        *self.models.borrow_mut() = models;
        self.render();
    }

    pub fn refresh_keys(&self) {
        self.has_key.set(Provider::ALL.map(|p| credentials::exists(&p.credential())));
        self.render();
    }

    /// Which tab the panel shows (remembered across launches by the caller).
    pub fn tab(&self) -> usize {
        self.tab.get()
    }

    /// The AI provider of the current tab, or `None` on the search tab.
    fn provider(&self) -> Option<Provider> {
        match tab_at(self.tab.get()) {
            Tab::Ai(p) => Some(p),
            Tab::Search => None,
        }
    }

    fn send(self: &Rc<Self>, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let Some(provider) = self.provider() else { return self.search(text) };
        if self.pending.borrow().is_some() {
            return;
        }
        let tab = provider.index();
        if text.chars().count() > ai::MAX_INPUT_CHARS {
            self.notices.borrow_mut()[tab] =
                format!("That message is too long. Keep it under {} characters.", ai::MAX_INPUT_CHARS);
            return self.render();
        }
        let Some(key) = credentials::read(&provider.credential()) else {
            return self.refresh_keys();
        };
        let model = self.models.borrow()[tab].clone();

        let history = {
            let mut convs = self.conversations.borrow_mut();
            convs[tab].push(Turn { role: Role::User, text: text.to_string() });
            let history = convs[tab].clone();
            convs[tab].push(Turn { role: Role::Assistant, text: String::new() });
            history
        };
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let handle = ai::start(ai::Request { provider, model, key, history }, move |update| {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(c) = CHAT.with(|c| c.borrow().upgrade()) {
                    c.on_update(id, update);
                }
            });
        });
        *self.pending.borrow_mut() = Some(Pending { provider, id, handle });
        self.notices.borrow_mut()[tab].clear();
        if let Some(cw) = self.window.borrow().as_ref() {
            cw.global::<ChatState>().set_draft(SharedString::new());
        }
        self.render();
    }

    /// Opens Google results in the browser and gets out of its way.
    fn search(&self, query: &str) {
        crate::platform::shell_open(&search_url(query));
        if let Some(cw) = self.window.borrow().as_ref() {
            cw.global::<ChatState>().set_draft(SharedString::new());
        }
        self.close();
    }

    fn on_update(&self, id: u64, update: Update) {
        let provider = match self.pending.borrow().as_ref() {
            Some(p) if p.id == id => p.provider,
            _ => return, // a stopped or superseded request
        };
        let tab = provider.index();
        match update {
            Update::Text(text) => {
                let overflow = {
                    let mut convs = self.conversations.borrow_mut();
                    let Some(reply) = convs[tab].last_mut() else { return };
                    reply.text.push_str(&text);
                    reply.text.chars().count() > MAX_REPLY_CHARS
                };
                if overflow {
                    self.stop("The reply was too long to show in full.");
                    return;
                }
                if self.provider() == Some(provider) {
                    self.sync_last_row();
                }
                self.update_status();
            }
            Update::Finished(finish) => {
                self.pending.borrow_mut().take();
                let name = provider.name();
                self.notices.borrow_mut()[tab] = match finish {
                    Finish::Complete => String::new(),
                    Finish::Truncated => "The reply hit its length limit.".into(),
                    Finish::Declined => format!("{name} declined to answer that."),
                };
                self.drop_empty_reply(tab);
                self.render();
            }
            Update::Failed(message) => {
                self.pending.borrow_mut().take();
                self.notices.borrow_mut()[tab] = message;
                self.drop_empty_reply(tab);
                self.render();
            }
        }
    }

    /// Ends the current request. Keeps any partial reply; an empty one is removed.
    fn stop(&self, notice: &str) {
        let Some(p) = self.pending.borrow_mut().take() else { return };
        p.handle.cancel();
        let tab = p.provider.index();
        self.notices.borrow_mut()[tab] = notice.to_string();
        self.drop_empty_reply(tab);
        self.render();
    }

    fn new_chat(&self) {
        let Some(provider) = self.provider() else { return };
        let tab = provider.index();
        if self.pending.borrow().as_ref().is_some_and(|p| p.provider == provider) {
            self.stop("");
        }
        self.conversations.borrow_mut()[tab].clear();
        self.notices.borrow_mut()[tab].clear();
        self.render();
    }

    /// If no text arrived, removes the empty reply and the question that went with it, and puts
    /// the question back in the composer so it can be retried or reworded. This also keeps the
    /// history alternating user/assistant, which every provider expects.
    fn drop_empty_reply(&self, tab: usize) {
        let restored = {
            let mut convs = self.conversations.borrow_mut();
            let conv = &mut convs[tab];
            if conv.last().is_some_and(|t| t.role == Role::Assistant && t.text.trim().is_empty()) {
                conv.pop();
                conv.pop().filter(|t| t.role == Role::User).map(|t| t.text)
            } else {
                None
            }
        };
        if let (Some(text), Some(cw)) = (restored, self.window.borrow().as_ref())
            && self.provider().map(Provider::index) == Some(tab)
        {
            let st = cw.global::<ChatState>();
            if st.get_draft().is_empty() {
                st.set_draft(text.into());
            }
        }
    }

    // ------------------------------------------------------------------ rendering

    fn rows_for(&self, provider: Option<Provider>) -> Vec<ChatMessage> {
        let Some(provider) = provider else { return Vec::new() };
        self.conversations.borrow()[provider.index()]
            .iter()
            .filter(|t| !t.text.is_empty())
            .map(|t| ChatMessage { from_user: t.role == Role::User, text: t.text.as_str().into() })
            .collect()
    }

    /// Streams touch only the last row, so earlier messages aren't re-laid out on every chunk.
    fn sync_last_row(&self) {
        let Some(provider) = self.provider() else { return };
        let convs = self.conversations.borrow();
        let Some(reply) = convs[provider.index()].last() else { return };
        let row = ChatMessage { from_user: false, text: reply.text.as_str().into() };
        let n = self.rows.row_count();
        if n > 0 && self.rows.row_data(n - 1).is_some_and(|r| !r.from_user) {
            self.rows.set_row_data(n - 1, row);
        } else {
            self.rows.push(row);
        }
    }

    fn render(&self) {
        self.rows.set_vec(self.rows_for(self.provider()));
        self.update_status();
    }

    fn update_status(&self) {
        let Some(cw) = self.window.borrow().as_ref().map(|cw| cw.clone_strong()) else { return };
        let st = cw.global::<ChatState>();
        st.set_tab(self.tab.get() as i32);
        let Some(provider) = self.provider() else {
            st.set_is_search(true);
            st.set_provider_name("Google".into());
            st.set_has_key(true); // search needs no key
            st.set_busy(false);
            st.set_waiting(false);
            st.set_notice(SharedString::new());
            return;
        };
        let tab = provider.index();
        let busy = self.pending.borrow().as_ref().is_some_and(|p| p.provider == provider);
        let reply_started =
            self.conversations.borrow()[tab].last().is_some_and(|t| t.role == Role::Assistant && !t.text.is_empty());
        st.set_is_search(false);
        st.set_provider_name(provider.name().into());
        st.set_vendor(provider.vendor().into());
        st.set_model(self.models.borrow()[tab].as_str().into());
        st.set_has_key(self.has_key.get()[tab]);
        st.set_busy(busy);
        st.set_waiting(busy && !reply_started);
        st.set_notice(self.notices.borrow()[tab].as_str().into());
    }
}

/// Settings → AI page index (kept clear of the other pages' numbers).
pub const SETTINGS_PAGE: i32 = 20;

// ------------------------------------------------------------------ settings page helpers

/// One row per provider for Settings → Search & AI, in the panel's tab order.
pub fn provider_rows(models: &[String; 3]) -> ModelRc<AiProviderRow> {
    let rows: Vec<AiProviderRow> = TABS
        .iter()
        .filter_map(|t| if let Tab::Ai(p) = t { Some(*p) } else { None })
        .map(|p| AiProviderRow {
            index: p.index() as i32,
            name: p.name().into(),
            vendor: p.vendor().into(),
            has_key: credentials::exists(&p.credential()),
            model: models[p.index()].as_str().into(),
            default_model: p.default_model().into(),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// Validates and stores a key. Returns a message for the user on failure.
pub fn save_key(index: usize, key: &str) -> Result<(), String> {
    let provider = Provider::from_index(index);
    // People often paste with a stray space or "Bearer " prefix.
    let key = key.trim();
    let key = key.strip_prefix("Bearer ").unwrap_or(key).trim();
    if !ai::valid_key(key) {
        return Err(format!("That doesn't look like a {} API key.", provider.vendor()));
    }
    if !credentials::write(&provider.credential(), key) {
        return Err("Windows Credential Manager couldn't save the key.".into());
    }
    storage::log(format!("saved {} API key", provider.name()));
    Ok(())
}

pub fn remove_key(index: usize) {
    let provider = Provider::from_index(index);
    credentials::delete(&provider.credential());
    storage::log(format!("removed {} API key", provider.name()));
}

pub fn open_keys_page(index: usize) {
    crate::platform::shell_open(Provider::from_index(index).keys_url());
}

/// The model to save for what the user typed: the provider's default when the field is empty, or an
/// error message when the name can't be a model ID.
pub fn clean_model(index: usize, model: &str) -> Result<String, String> {
    let model = model.trim();
    if model.is_empty() {
        Ok(Provider::from_index(index).default_model().to_string())
    } else if ai::valid_model(model) {
        Ok(model.to_string())
    } else {
        Err("Model names use only letters, numbers, dots, dashes and underscores. Not saved.".into())
    }
}

// ------------------------------------------------------------------ helpers

fn theme_window(cw: &ChatWindow, palette: &Palette) {
    crate::theme::apply(&cw.global::<Theme>(), palette);
}

fn position(cw: &ChatWindow, anchor: &Anchor) {
    let primary = monitors::all().into_iter().find(|m| m.primary);
    let scale = primary.as_ref().map(|m| m.scale).unwrap_or(1.0);
    let (w, h, gap) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32, (10.0 * scale) as i32);
    let (x, y) = match (&anchor.hud, &primary) {
        // Drops down under the right half of the Top HUD, near its controls.
        (Some(hud), _) if anchor.top_layout => (hud.right - w - gap, hud.bottom + gap),
        // Slides out beside the Sidebar, level with its controls at the bottom.
        (Some(hud), _) => (hud.right + gap, (hud.bottom - h - gap).max(hud.top + gap)),
        (None, Some(m)) => {
            let wa = m.work_area;
            (wa.left + (wa.right - wa.left - w) / 2, wa.top + (wa.bottom - wa.top - h) / 2)
        }
        (None, None) => return,
    };
    cw.window().set_position(PhysicalPosition::new(x, y));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_put_search_first_then_chatgpt_claude_gemini() {
        assert_eq!(tab_at(0), Tab::Search);
        assert_eq!(tab_of(Provider::ChatGpt), 1);
        assert_eq!(tab_of(Provider::Claude), 2);
        assert_eq!(tab_of(Provider::Gemini), 3);
        assert_eq!(tab_at(99), Tab::Ai(Provider::Gemini));
    }

    #[test]
    fn search_url_encodes_the_query() {
        assert_eq!(search_url("rust & slint?"), "https://www.google.com/search?q=rust%20%26%20slint%3F");
        // Nothing in the query can change the host or add parameters.
        assert_eq!(search_url("x&q=evil#frag"), "https://www.google.com/search?q=x%26q%3Devil%23frag");
    }
}
