//! Optional AI chat with Claude, ChatGPT or Gemini, using the user's own API key.
//!
//! Each request runs on its own short-lived thread and streams the reply back over Server-Sent
//! Events. Requests go straight from this PC to the chosen provider; nothing is proxied, logged or
//! saved. Log lines name the provider and HTTP status only, never keys or message text.

use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::services::http;
use crate::storage;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    ChatGpt,
    Gemini,
}

impl Provider {
    /// Tab order in the chat panel and Settings.
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::ChatGpt, Provider::Gemini];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Provider {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn id(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::ChatGpt => "chatgpt",
            Provider::Gemini => "gemini",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::ChatGpt => "ChatGPT",
            Provider::Gemini => "Gemini",
        }
    }

    pub fn vendor(self) -> &'static str {
        match self {
            Provider::Claude => "Anthropic",
            Provider::ChatGpt => "OpenAI",
            Provider::Gemini => "Google",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Claude => "claude-opus-5-5",
            Provider::ChatGpt => "gpt-6.1-sol",
            // Google's alias for its newest Flash model, so the default doesn't go stale.
            Provider::Gemini => "gemini-flash-latest",
        }
    }

    /// Where the user creates an API key.
    pub fn keys_url(self) -> &'static str {
        match self {
            Provider::Claude => "https://platform.claude.com/settings/keys",
            Provider::ChatGpt => "https://platform.openai.com/api-keys",
            Provider::Gemini => "https://aistudio.google.com/apikey",
        }
    }

    /// The Windows Credential Manager entry holding this provider's key.
    pub fn credential(self) -> String {
        format!("LifeHUD/ai/{}", self.id())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub role: Role,
    pub text: String,
}

/// How a reply ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Complete,
    /// Hit the output limit.
    Truncated,
    /// The provider's safety systems declined to answer.
    Declined,
}

/// What the worker reports back, in order: any number of `Text` chunks, then one `Finished` or
/// `Failed`. Nothing is sent after a request is cancelled.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Text(String),
    Finished(Finish),
    Failed(String),
}

pub struct Request {
    pub provider: Provider,
    pub model: String,
    pub key: String,
    pub history: Vec<Turn>,
}

/// Cancels the request when asked; the worker stops at the next chunk and sends nothing more.
pub struct Handle(Arc<AtomicBool>);

impl Handle {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Longest message the user can send, in characters.
pub const MAX_INPUT_CHARS: usize = 16_000;
/// Only the most recent turns are sent, which bounds cost and request size in long chats.
const MAX_TURNS: usize = 24;
const MAX_OUTPUT_TOKENS: u32 = 32_000;
/// Hard cap on a streamed reply, so a misbehaving server can't exhaust memory.
const MAX_STREAM_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ERROR_BYTES: u64 = 64 * 1024;
/// UI updates are batched to roughly 25 per second, however fast tokens arrive.
const FLUSH_EVERY: Duration = Duration::from_millis(40);

const SYSTEM_PROMPT: &str = "You are a helpful assistant inside Life HUD, a small panel on the user's Windows desktop. \
Keep answers concise and conversational. Replies are shown as plain text, so write short paragraphs and \
simple lists, and avoid Markdown headings, tables and bold text. Use a code block only when you show code.";

/// Starts a request on its own thread. `sink` is called from that thread.
pub fn start(req: Request, sink: impl Fn(Update) + Send + 'static) -> Handle {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let spawned = std::thread::Builder::new().name("ai-chat".into()).spawn(move || {
        let deliver = |u: Update| {
            if !flag.load(Ordering::Relaxed) {
                sink(u);
            }
        };
        run(&req, &flag, &deliver);
    });
    if let Err(e) = spawned {
        storage::log(format!("could not start chat request: {e}"));
    }
    Handle(cancel)
}

// ------------------------------------------------------------------ validation

/// Model IDs end up in URLs and JSON, so only plain identifier characters are allowed.
pub fn valid_model(model: &str) -> bool {
    (1..=100).contains(&model.len())
        && model.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Keys go into HTTP headers, so anything outside printable ASCII (including spaces) is rejected.
pub fn valid_key(key: &str) -> bool {
    (8..=512).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic())
}

/// The turns to send: the most recent ones, starting with a user turn, skipping empty replies.
fn recent(history: &[Turn]) -> Vec<&Turn> {
    let turns: Vec<&Turn> = history.iter().filter(|t| !t.text.trim().is_empty()).collect();
    let mut start = turns.len().saturating_sub(MAX_TURNS);
    while start < turns.len() && turns[start].role != Role::User {
        start += 1;
    }
    turns[start..].to_vec()
}

// ------------------------------------------------------------------ requests

struct HttpRequest {
    url: String,
    headers: Vec<(&'static str, String)>,
    body: Value,
}

/// Claude models that take an explicit effort level and server-side refusal fallbacks.
fn claude_extras(model: &str) -> (bool, bool) {
    const EFFORT: [&str; 12] = [
        "claude-fable-5-2",
        "claude-fable-5-1",
        "claude-fable-5",
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-sonnet-5-5",
        "claude-sonnet-5",
        "claude-sonnet-4-6",
        "claude-haiku-5-5",
    ];
    const FALLBACKS: [&str; 5] =
        ["claude-fable-5-2", "claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];
    (EFFORT.contains(&model), FALLBACKS.contains(&model))
}

fn build(req: &Request) -> HttpRequest {
    let turns = recent(&req.history);
    match req.provider {
        Provider::Claude => {
            let messages: Vec<Value> = turns
                .iter()
                .map(|t| json!({ "role": if t.role == Role::User { "user" } else { "assistant" }, "content": t.text }))
                .collect();
            let mut body = json!({
                "model": req.model,
                "max_tokens": MAX_OUTPUT_TOKENS,
                "system": SYSTEM_PROMPT,
                "messages": messages,
                "stream": true,
            });
            let mut headers = vec![("x-api-key", req.key.clone()), ("anthropic-version", "2026-10-21".to_string())];
            let (effort, fallbacks) = claude_extras(&req.model);
            if effort {
                body["output_config"] = json!({ "effort": "medium" });
            }
            if fallbacks {
                // If a safety classifier declines, Anthropic retries on a suitable model in the same stream.
                body["fallbacks"] = json!("default");
                headers.push(("anthropic-beta", "server-side-fallback-2026-07-01".to_string()));
            }
            HttpRequest { url: "https://api.anthropic.com/v1/messages".into(), headers, body }
        }
        Provider::ChatGpt => {
            let input: Vec<Value> = turns
                .iter()
                .map(|t| json!({ "role": if t.role == Role::User { "user" } else { "assistant" }, "content": t.text }))
                .collect();
            HttpRequest {
                url: "https://api.openai.com/v1/responses".into(),
                headers: vec![("authorization", format!("Bearer {}", req.key))],
                // `store: false`: don't keep the conversation on OpenAI's side for later retrieval.
                body: json!({
                    "model": req.model,
                    "instructions": SYSTEM_PROMPT,
                    "input": input,
                    "max_output_tokens": MAX_OUTPUT_TOKENS,
                    "stream": true,
                    "store": false,
                }),
            }
        }
        Provider::Gemini => {
            let contents: Vec<Value> = turns
                .iter()
                .map(|t| json!({ "role": if t.role == Role::User { "user" } else { "model" }, "parts": [{ "text": t.text }] }))
                .collect();
            HttpRequest {
                url: format!(
                    "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
                    req.model
                ),
                headers: vec![("x-goog-api-key", req.key.clone())],
                body: json!({
                    "systemInstruction": { "parts": [{ "text": SYSTEM_PROMPT }] },
                    "contents": contents,
                    "generationConfig": { "maxOutputTokens": MAX_OUTPUT_TOKENS },
                }),
            }
        }
    }
}

// ------------------------------------------------------------------ streaming

/// Incremental Server-Sent Events parser: feed it lines, get back (event, data) when one completes.
#[derive(Default)]
struct Sse {
    event: String,
    data: String,
}

impl Sse {
    fn line(&mut self, line: &str) -> Option<(String, String)> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return self.finish();
        }
        if line.starts_with(':') {
            return None; // comment / keep-alive
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = value.to_string(),
            "data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            _ => {}
        }
        None
    }

    /// Dispatches whatever is buffered (a blank line, or the end of the stream).
    fn finish(&mut self) -> Option<(String, String)> {
        if self.data.is_empty() {
            self.event.clear();
            return None;
        }
        Some((std::mem::take(&mut self.event), std::mem::take(&mut self.data)))
    }
}

/// What one SSE event contributes: text to append, and possibly how the reply ended.
#[derive(Debug, Default, PartialEq)]
struct Chunk {
    text: String,
    end: Option<Update>,
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(*k)).and_then(Value::as_str)
}

fn parse_event(provider: Provider, data: &str) -> Chunk {
    let mut chunk = Chunk::default();
    let Ok(v) = serde_json::from_str::<Value>(data) else { return chunk };
    match provider {
        Provider::Claude => match str_at(&v, &["type"]) {
            // Only visible text; thinking deltas and fallback markers aren't shown.
            Some("content_block_delta") if str_at(&v, &["delta", "type"]) == Some("text_delta") => {
                chunk.text = str_at(&v, &["delta", "text"]).unwrap_or_default().to_string();
            }
            Some("message_delta") => {
                chunk.end = str_at(&v, &["delta", "stop_reason"]).map(|r| {
                    Update::Finished(match r {
                        "max_tokens" => Finish::Truncated,
                        "refusal" => Finish::Declined,
                        _ => Finish::Complete,
                    })
                });
            }
            Some("message_stop") => chunk.end = Some(Update::Finished(Finish::Complete)),
            Some("error") => chunk.end = Some(Update::Failed(provider_message(&v).unwrap_or_default())),
            _ => {}
        },
        Provider::ChatGpt => match str_at(&v, &["type"]) {
            Some("response.output_text.delta") | Some("response.refusal.delta") => {
                chunk.text = str_at(&v, &["delta"]).unwrap_or_default().to_string();
            }
            Some("response.completed") => chunk.end = Some(Update::Finished(Finish::Complete)),
            Some("response.incomplete") => {
                let reason = str_at(&v, &["response", "incomplete_details", "reason"]);
                let finish = if reason == Some("content_filter") { Finish::Declined } else { Finish::Truncated };
                chunk.end = Some(Update::Finished(finish));
            }
            Some("response.failed") => {
                let msg = str_at(&v, &["response", "error", "message"]).unwrap_or_default();
                chunk.end = Some(Update::Failed(msg.to_string()));
            }
            Some("error") => {
                let msg = str_at(&v, &["message"]).or_else(|| str_at(&v, &["error", "message"])).unwrap_or_default();
                chunk.end = Some(Update::Failed(msg.to_string()));
            }
            _ => {}
        },
        Provider::Gemini => {
            if v.get("error").is_some() {
                chunk.end = Some(Update::Failed(provider_message(&v).unwrap_or_default()));
                return chunk;
            }
            if str_at(&v, &["promptFeedback", "blockReason"]).is_some() {
                chunk.end = Some(Update::Finished(Finish::Declined));
                return chunk;
            }
            let candidate = &v["candidates"][0];
            if let Some(parts) = candidate["content"]["parts"].as_array() {
                for part in parts.iter().filter(|p| p["thought"].as_bool() != Some(true)) {
                    chunk.text.push_str(part["text"].as_str().unwrap_or_default());
                }
            }
            chunk.end = match candidate["finishReason"].as_str() {
                None | Some("FINISH_REASON_UNSPECIFIED") => None,
                Some("STOP") => Some(Update::Finished(Finish::Complete)),
                Some("MAX_TOKENS") => Some(Update::Finished(Finish::Truncated)),
                Some(_) => Some(Update::Finished(Finish::Declined)),
            };
        }
    }
    chunk
}

fn run(req: &Request, cancel: &AtomicBool, sink: &dyn Fn(Update)) {
    let http = build(req);
    let mut call = http::streaming_agent().post(&http.url).header("content-type", "application/json");
    for (name, value) in &http.headers {
        call = call.header(*name, value.as_str());
    }
    let mut response = match call.send(http.body.to_string()) {
        Ok(r) => r,
        Err(e) => {
            storage::log(format!("chat request to {} failed: {e}", req.provider.name()));
            let msg = if matches!(e, ureq::Error::Timeout(_)) {
                format!("{} didn't answer in time. Try again.", req.provider.vendor())
            } else {
                format!("Couldn't reach {}. Check your internet connection.", req.provider.vendor())
            };
            return sink(Update::Failed(msg));
        }
    };

    let status = response.status().as_u16();
    if status != 200 {
        let body = response.body_mut().with_config().limit(MAX_ERROR_BYTES).read_to_string().unwrap_or_default();
        storage::log(format!("chat request to {} returned HTTP {status}", req.provider.name()));
        return sink(Update::Failed(http_error(req, status, &body)));
    }

    let reader = BufReader::new(response.into_body().into_with_config().limit(MAX_STREAM_BYTES).reader());
    let mut sse = Sse::default();
    let mut pending = String::new();
    let mut last_flush = Instant::now();
    let mut received = false;
    let flush = |pending: &mut String| {
        if !pending.is_empty() {
            sink(Update::Text(std::mem::take(pending)));
        }
    };

    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                storage::log(format!("chat stream from {} broke: {e}", req.provider.name()));
                flush(&mut pending);
                return sink(Update::Failed("The connection dropped before the reply finished.".into()));
            }
        };
        let Some((_, data)) = sse.line(&line) else { continue };
        let chunk = parse_event(req.provider, &data);
        if !chunk.text.is_empty() {
            received = true;
            pending.push_str(&chunk.text);
        }
        if let Some(end) = chunk.end {
            flush(&mut pending);
            return sink(match end {
                Update::Failed(m) => Update::Failed(clean_message(req, &m)),
                other => other,
            });
        }
        if last_flush.elapsed() >= FLUSH_EVERY {
            flush(&mut pending);
            last_flush = Instant::now();
        }
    }

    // The stream ended without an explicit finish (Gemini can end on a chunk without one).
    if let Some((_, data)) = sse.finish() {
        let chunk = parse_event(req.provider, &data);
        received |= !chunk.text.is_empty();
        pending.push_str(&chunk.text);
    }
    flush(&mut pending);
    sink(if received {
        Update::Finished(Finish::Complete)
    } else {
        Update::Failed("The reply ended before any text arrived. Try again.".into())
    });
}

// ------------------------------------------------------------------ errors

/// `error.message` (all three providers) or a top-level `message`.
fn provider_message(v: &Value) -> Option<String> {
    str_at(v, &["error", "message"]).or_else(|| str_at(v, &["message"])).map(str::to_string)
}

/// One line, bounded, with the key scrubbed in case a provider echoes any of it back.
fn clean_message(req: &Request, msg: &str) -> String {
    let mut msg = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    if !req.key.is_empty() {
        msg = msg.replace(&req.key, "•••");
    }
    if msg.chars().count() > 200 {
        msg = msg.chars().take(199).collect::<String>() + "…";
    }
    if msg.is_empty() { format!("{} returned an error.", req.provider.vendor()) } else { msg }
}

fn http_error(req: &Request, status: u16, body: &str) -> String {
    let vendor = req.provider.vendor();
    let detail = serde_json::from_str::<Value>(body).ok().and_then(|v| provider_message(&v));
    let detail = detail.map(|d| clean_message(req, &d));
    // Gemini reports a bad key as 400 with reason API_KEY_INVALID rather than 401.
    let bad_key = matches!(status, 401 | 403) || (status == 400 && body.contains("API_KEY_INVALID"));
    match status {
        _ if bad_key => format!("{vendor} didn't accept the API key. Check it in Settings → AI."),
        404 => format!("{vendor} doesn't recognise the model “{}”. Check it in Settings → AI.", req.model),
        408 | 504 => format!("{vendor} didn't answer in time. Try again."),
        413 => "This conversation is too long. Start a new chat.".into(),
        429 => format!("{vendor} says you've hit a rate or spending limit. Wait a moment, or check your plan."),
        500..=599 => format!("{vendor} is having trouble right now. Try again shortly."),
        _ => detail.unwrap_or_else(|| format!("{vendor} returned an error (HTTP {status}).")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: Role, text: &str) -> Turn {
        Turn { role, text: text.into() }
    }

    fn request(provider: Provider, model: &str) -> Request {
        Request {
            provider,
            model: model.into(),
            key: "sk-test-123456".into(),
            history: vec![turn(Role::User, "Hi"), turn(Role::Assistant, "Hello!"), turn(Role::User, "Weather?")],
        }
    }

    #[test]
    fn validates_models_and_keys() {
        assert!(valid_model("claude-opus-5-5"));
        assert!(valid_model("gpt-6.1-sol"));
        assert!(!valid_model("gemini/../../x"));
        assert!(!valid_model("model:streamGenerateContent"));
        assert!(!valid_model(""));
        assert!(valid_key("sk-ant-api03-abcdef"));
        assert!(!valid_key("sk-ant\r\nX-Evil: 1"));
        assert!(!valid_key("has space"));
        assert!(!valid_key("short"));
    }

    #[test]
    fn sends_recent_turns_starting_with_the_user() {
        let mut history = vec![turn(Role::Assistant, "orphan")];
        for i in 0..30 {
            history.push(turn(Role::User, &format!("q{i}")));
            history.push(turn(Role::Assistant, if i == 29 { "" } else { "a" }));
        }
        let sent = recent(&history);
        assert!(sent.len() <= MAX_TURNS);
        assert_eq!(sent[0].role, Role::User);
        assert_eq!(sent.last().unwrap().text, "q29"); // empty trailing reply skipped
    }

    #[test]
    fn builds_claude_request() {
        let r = build(&request(Provider::Claude, "claude-opus-5-5"));
        assert_eq!(r.url, "https://api.anthropic.com/v1/messages");
        assert!(r.headers.contains(&("anthropic-version", "2026-10-21".into())));
        assert!(r.headers.contains(&("anthropic-beta", "server-side-fallback-2026-07-01".into())));
        assert_eq!(r.body["messages"][1]["role"], "assistant");
        assert_eq!(r.body["fallbacks"], "default");
        assert_eq!(r.body["output_config"]["effort"], "medium");
        assert!(r.body.get("temperature").is_none());
        // Older or unknown models get the plain request.
        let r = build(&request(Provider::Claude, "claude-haiku-4-5"));
        assert!(r.body.get("fallbacks").is_none() && r.body.get("output_config").is_none());
        assert!(r.headers.iter().all(|(h, _)| *h != "anthropic-beta"));
    }

    #[test]
    fn builds_openai_and_gemini_requests() {
        let r = build(&request(Provider::ChatGpt, "gpt-6.1-sol"));
        assert_eq!(r.body["store"], false);
        assert_eq!(r.body["input"][2]["content"], "Weather?");
        assert_eq!(r.headers[0], ("authorization", "Bearer sk-test-123456".into()));
        let r = build(&request(Provider::Gemini, "gemini-flash-latest"));
        assert!(r.url.ends_with("/models/gemini-flash-latest:streamGenerateContent?alt=sse"));
        assert_eq!(r.body["contents"][1]["role"], "model");
        assert_eq!(r.body["contents"][2]["parts"][0]["text"], "Weather?");
        assert!(!r.url.contains("key="), "the key goes in a header, never the URL");
    }

    #[test]
    fn parses_sse_lines() {
        let mut sse = Sse::default();
        assert_eq!(sse.line(": keep-alive"), None);
        assert_eq!(sse.line("event: message_start"), None);
        assert_eq!(sse.line("data: {\"a\":1}\r"), None);
        assert_eq!(sse.line(""), Some(("message_start".into(), "{\"a\":1}".into())));
        assert_eq!(sse.line(""), None);
        sse.line("data: one");
        sse.line("data: two");
        assert_eq!(sse.finish(), Some((String::new(), "one\ntwo".into())));
    }

    #[test]
    fn parses_claude_events() {
        let text = parse_event(
            Provider::Claude,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
        );
        assert_eq!(text.text, "Hi");
        let thinking = parse_event(
            Provider::Claude,
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"…"}}"#,
        );
        assert_eq!(thinking, Chunk::default());
        let refusal = parse_event(Provider::Claude, r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#);
        assert_eq!(refusal.end, Some(Update::Finished(Finish::Declined)));
        let err = parse_event(
            Provider::Claude,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        );
        assert_eq!(err.end, Some(Update::Failed("Overloaded".into())));
    }

    #[test]
    fn parses_openai_events() {
        assert_eq!(
            parse_event(Provider::ChatGpt, r#"{"type":"response.output_text.delta","delta":"Hey"}"#).text,
            "Hey"
        );
        let done = parse_event(Provider::ChatGpt, r#"{"type":"response.completed","response":{}}"#);
        assert_eq!(done.end, Some(Update::Finished(Finish::Complete)));
        let cut = parse_event(
            Provider::ChatGpt,
            r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#,
        );
        assert_eq!(cut.end, Some(Update::Finished(Finish::Truncated)));
        let failed =
            parse_event(Provider::ChatGpt, r#"{"type":"response.failed","response":{"error":{"message":"boom"}}}"#);
        assert_eq!(failed.end, Some(Update::Failed("boom".into())));
    }

    #[test]
    fn parses_gemini_chunks() {
        let c = parse_event(
            Provider::Gemini,
            r#"{"candidates":[{"content":{"parts":[{"text":"plan","thought":true},{"text":"Hello "},{"text":"there"}],"role":"model"}}]}"#,
        );
        assert_eq!(c, Chunk { text: "Hello there".into(), end: None });
        let stop = parse_event(
            Provider::Gemini,
            r#"{"candidates":[{"content":{"parts":[{"text":"."}]},"finishReason":"STOP"}]}"#,
        );
        assert_eq!(stop.end, Some(Update::Finished(Finish::Complete)));
        let blocked = parse_event(Provider::Gemini, r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#);
        assert_eq!(blocked.end, Some(Update::Finished(Finish::Declined)));
        let safety = parse_event(Provider::Gemini, r#"{"candidates":[{"finishReason":"SAFETY"}]}"#);
        assert_eq!(safety.end, Some(Update::Finished(Finish::Declined)));
    }

    #[test]
    fn friendly_errors_never_echo_the_key() {
        let req = request(Provider::ChatGpt, "gpt-6.1-sol");
        assert!(http_error(&req, 401, "").contains("API key"));
        assert!(http_error(&req, 404, "").contains("gpt-6.1-sol"));
        let echoed = r#"{"error":{"message":"Bad key sk-test-123456 provided"}}"#;
        let msg = http_error(&req, 400, echoed);
        assert!(!msg.contains("sk-test-123456"), "{msg}");
        assert!(http_error(&req, 418, "not json").contains("HTTP 418"));
        let gemini = request(Provider::Gemini, "gemini-flash-latest");
        let invalid =
            r#"{"error":{"code":400,"message":"API key not valid.","details":[{"reason":"API_KEY_INVALID"}]}}"#;
        assert!(http_error(&gemini, 400, invalid).starts_with("Google didn't accept the API key"));
    }
}
