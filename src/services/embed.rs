//! YouTube's embedded player, served from a page on this PC.
//!
//! YouTube refuses to play embeds and its old pop-out page without a referring web page ("Error 153:
//! video player configuration error"), and an app window opened straight on a YouTube URL has none.
//! So Life HUD serves one tiny page that is nothing but YouTube's official player filling the
//! window. The docked and pop-out players open that page.
//!
//! Any web page the user visits can also send requests to 127.0.0.1, so the server is strict:
//! - it binds to loopback only, on a random port;
//! - every URL carries a secret token made fresh each launch, so other pages can't guess one;
//! - the `Host` header must be exactly `127.0.0.1:<port>`, which defeats DNS rebinding;
//! - it answers exactly one shape of request, `GET /<token>/youtube/<video id>`, with a fixed page.
//!   The id must match YouTube's 11-character alphabet, so nothing a request says is echoed back
//!   except a validated id. Everything else gets a 404.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;
use std::time::Duration;

use crate::services::youtube::is_video_id;

/// The page's window title once its player has started (the docked player watches for it).
pub const PLAYING_TITLE: &str = "Life HUD player \u{25B6}";

/// The page for video `id`, starting the server on first use. `None` if it couldn't start.
pub fn youtube_url(id: &str) -> Option<String> {
    if !is_video_id(id) {
        return None;
    }
    let (port, token) = server()?;
    Some(format!("http://127.0.0.1:{port}/{token}/youtube/{id}"))
}

/// The server's port and this launch's token, starting it on first use.
fn server() -> Option<(u16, &'static str)> {
    static SERVER: OnceLock<Option<(u16, String)>> = OnceLock::new();
    SERVER
        .get_or_init(|| {
            let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
            let port = listener.local_addr().ok()?.port();
            let token = token();
            let t = token.clone();
            std::thread::Builder::new()
                .name("embed".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let _ = serve(stream, port, &t);
                    }
                })
                .ok()?;
            Some((port, token))
        })
        .as_ref()
        .map(|(port, token)| (*port, token.as_str()))
}

/// 128 random bits as hex, from the OS-seeded keys behind std's `RandomState`.
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let half = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(u64::from(std::process::id()));
        h.finish()
    };
    format!("{:016x}{:016x}", half(), half())
}

fn serve(mut stream: TcpStream, port: u16, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let body = {
        // Capped, so a client can't feed endless headers.
        let mut reader = BufReader::new(&stream).take(8192);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let mut host_ok = false;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.trim().eq_ignore_ascii_case("host")
            {
                host_ok = value.trim() == format!("127.0.0.1:{port}");
            }
        }
        let path = line.strip_prefix("GET ").and_then(|rest| rest.split(' ').next()).unwrap_or("");
        let page = if host_ok { path.strip_prefix('/').and_then(|p| p.strip_prefix(token)) } else { None };
        // "/youtube/<id>", or "/youtube/<id>/muted" (tests).
        page.and_then(|p| p.strip_prefix("/youtube/")).and_then(|rest| {
            let (id, muted) = rest.strip_suffix("/muted").map_or((rest, false), |id| (id, true));
            is_video_id(id).then(|| page_html(id, port, muted))
        })
    };
    let response = match body {
        Some(body) => format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
             Cache-Control: no-store\r\nReferrer-Policy: strict-origin-when-cross-origin\r\n\
             Content-Security-Policy: default-src 'none'; frame-src https://www.youtube.com; \
             style-src 'unsafe-inline'; script-src 'unsafe-inline'\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        ),
        None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
    };
    stream.write_all(response.as_bytes())
}

/// The whole page: YouTube's player, edge to edge on black. Browsers only allow sound after a click
/// in the page, so the player loads on the first click (a play symbol invites it in the pop-out; the
/// docked player clicks for the user) and the title then says so.
fn page_html(id: &str, port: u16, muted: bool) -> String {
    let mute = if muted { "&mute=1" } else { "" };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>Life HUD player</title>
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}iframe{{display:block;border:0;width:100%;height:100%}}
#go{{position:fixed;inset:0;cursor:pointer;display:flex;align-items:center;justify-content:center}}
#go::after{{content:"";border-style:solid;border-width:22px 0 22px 38px;border-color:transparent transparent transparent #fff;opacity:.85}}</style>
</head><body><div id="go"></div><script>
document.getElementById("go").addEventListener("click",function(){{
var f=document.createElement("iframe");
f.src="https://www.youtube.com/embed/{id}?autoplay=1&playsinline=1&rel=0{mute}&origin=http://127.0.0.1:{port}";
f.allow="autoplay; encrypted-media; picture-in-picture; fullscreen";f.allowFullscreen=true;
document.body.replaceChildren(f);document.title="{playing}";}},{{once:true}});
</script></body></html>"#,
        playing = PLAYING_TITLE
    )
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    /// (port, token)
    fn server() -> (u16, &'static str) {
        super::youtube_url("dQw4w9WgXcQ").unwrap();
        super::server().unwrap()
    }

    fn get_with_host(path: &str, host: &str) -> String {
        let (port, _) = server();
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    fn get(path: &str) -> String {
        let (port, _) = server();
        get_with_host(path, &format!("127.0.0.1:{port}"))
    }

    #[test]
    fn serves_only_the_player_page() {
        let (port, token) = server();
        assert_eq!(token.len(), 32);
        assert_eq!(
            super::youtube_url("dQw4w9WgXcQ").unwrap(),
            format!("http://127.0.0.1:{port}/{token}/youtube/dQw4w9WgXcQ")
        );
        assert!(super::youtube_url("not a video").is_none());
        let ok = get(&format!("/{token}/youtube/dQw4w9WgXcQ"));
        assert!(ok.starts_with("HTTP/1.1 200"));
        assert!(ok.contains("https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1"));
        assert!(!ok.contains("mute=1"));
        assert!(get(&format!("/{token}/youtube/dQw4w9WgXcQ/muted")).contains("mute=1"));
        for bad in
            ["/", "/youtube/", "/youtube/../../etc", "/youtube/dQw4w9WgXcQ<script>", "/other", "/youtube/x/muted"]
        {
            assert!(get(&format!("/{token}{bad}")).starts_with("HTTP/1.1 404"), "{bad}");
        }
    }

    #[test]
    fn other_pages_and_rebinding_get_nothing() {
        let (port, token) = server();
        let page = format!("/{token}/youtube/dQw4w9WgXcQ");
        assert!(get("/youtube/dQw4w9WgXcQ").starts_with("HTTP/1.1 404"), "no token");
        assert!(
            get("/00000000000000000000000000000000/youtube/dQw4w9WgXcQ").starts_with("HTTP/1.1 404"),
            "wrong token"
        );
        assert!(get_with_host(&page, "evil.example").starts_with("HTTP/1.1 404"), "DNS rebinding");
        assert!(get_with_host(&page, &format!("localhost:{port}")).starts_with("HTTP/1.1 404"));
        assert!(get(&page).starts_with("HTTP/1.1 200"));
    }
}
