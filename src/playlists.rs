//! Saved Spotify playlists for the music widget's right-click menu. No account is involved: the user
//! pastes a link, Life HUD stores it locally, and choosing it opens it in the Spotify app.

use serde::{Deserialize, Serialize};

/// Most people keep a handful; the cap keeps the menu usable and the settings file small.
pub const MAX: usize = 40;
/// Kinds of Spotify link that make sense in a "browse" menu.
const KINDS: [&str; 4] = ["playlist", "album", "artist", "show"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub name: String,
    /// Canonical `spotify:<kind>:<id>`.
    pub uri: String,
    #[serde(default)]
    pub pinned: bool,
}

/// Turns a pasted Spotify link into a canonical `spotify:<kind>:<id>` URI.
///
/// Accepts share links (`https://open.spotify.com/playlist/<id>?si=…`, including `/intl-xx/` and
/// `/embed/` forms) and URIs (`spotify:playlist:<id>`, and the legacy `spotify:user:<u>:playlist:<id>`).
pub fn parse_link(input: &str) -> Option<String> {
    let input = input.trim();
    let (kind, id) = if let Some(rest) = input.strip_prefix("spotify:") {
        let parts: Vec<&str> = rest.split(':').collect();
        match parts.as_slice() {
            [.., kind, id] => (*kind, *id),
            _ => return None,
        }
    } else {
        let rest = input.strip_prefix("https://").or_else(|| input.strip_prefix("http://")).unwrap_or(input);
        let path = rest.strip_prefix("open.spotify.com/")?;
        let path = path.split(['?', '#']).next().unwrap_or("");
        let mut segments = path.split('/').filter(|s| !s.is_empty() && !s.starts_with("intl-") && *s != "embed");
        (segments.next()?, segments.next()?)
    };
    let valid_id = (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric());
    (KINDS.contains(&kind) && valid_id).then(|| format!("spotify:{kind}:{id}"))
}

/// The `https://open.spotify.com/...` page for a canonical URI.
pub fn web_url(uri: &str) -> String {
    let mut parts = uri.splitn(3, ':').skip(1);
    let (kind, id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    format!("https://open.spotify.com/{kind}/{id}")
}

/// A name to show until the real title is known ("Playlist", "Album", …).
pub fn placeholder_name(uri: &str) -> String {
    let kind = uri.split(':').nth(1).unwrap_or("playlist");
    let mut chars = kind.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Display order everywhere: pinned first, then the rest, each in the order they were added.
pub fn ordered(list: &[Playlist]) -> Vec<&Playlist> {
    list.iter().filter(|p| p.pinned).chain(list.iter().filter(|p| !p.pinned)).collect()
}

/// Drops invalid and duplicate entries and caps the list.
pub fn normalize(list: &mut Vec<Playlist>) {
    let mut seen: Vec<String> = Vec::new();
    list.retain_mut(|p| {
        let Some(uri) = parse_link(&p.uri) else { return false };
        if seen.contains(&uri) {
            return false;
        }
        seen.push(uri.clone());
        p.uri = uri;
        p.name = p.name.trim().to_string();
        if p.name.is_empty() {
            p.name = placeholder_name(&p.uri);
        }
        true
    });
    list.truncate(MAX);
}

/// Pulls the title out of Spotify's public oEmbed response (`{"title": "…", …}`).
pub fn title_from_oembed(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let title = v.get("title")?.as_str()?.trim();
    (!title.is_empty()).then(|| title.chars().take(80).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_share_links_and_uris() {
        let id = "37i9dQZF1DXcBWIGoYBM5M";
        let want = Some(format!("spotify:playlist:{id}"));
        assert_eq!(parse_link(&format!("https://open.spotify.com/playlist/{id}?si=abc123")), want);
        assert_eq!(parse_link(&format!("  open.spotify.com/intl-de/playlist/{id}  ")), want);
        assert_eq!(parse_link(&format!("https://open.spotify.com/embed/playlist/{id}")), want);
        assert_eq!(parse_link(&format!("spotify:playlist:{id}")), want);
        assert_eq!(parse_link(&format!("spotify:user:someone:playlist:{id}")), want);
        assert_eq!(
            parse_link("https://open.spotify.com/album/4aawyAB9vmqN3uQ7FjRGTy"),
            Some("spotify:album:4aawyAB9vmqN3uQ7FjRGTy".into())
        );
    }

    #[test]
    fn rejects_other_links() {
        assert_eq!(parse_link("https://open.spotify.com/track/abc"), None); // single tracks aren't browsable
        assert_eq!(parse_link("https://example.com/playlist/abc"), None);
        assert_eq!(parse_link("https://spotify.link/xyz"), None);
        assert_eq!(parse_link("spotify:playlist:bad id"), None);
        assert_eq!(parse_link("spotify:playlist:abc:extra&cmd"), None);
        assert_eq!(parse_link(""), None);
    }

    #[test]
    fn web_url_and_placeholder() {
        assert_eq!(web_url("spotify:album:abc"), "https://open.spotify.com/album/abc");
        assert_eq!(placeholder_name("spotify:album:abc"), "Album");
    }

    #[test]
    fn pinned_come_first_in_added_order() {
        let p = |name: &str, pinned| Playlist { name: name.into(), uri: format!("spotify:playlist:{name}"), pinned };
        let list = vec![p("a", false), p("b", true), p("c", false), p("d", true)];
        let names: Vec<&str> = ordered(&list).iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["b", "d", "a", "c"]);
    }

    #[test]
    fn normalize_repairs_and_dedupes() {
        let mut list = vec![
            Playlist { name: "  ".into(), uri: "https://open.spotify.com/playlist/abc?si=1".into(), pinned: true },
            Playlist { name: "Dup".into(), uri: "spotify:playlist:abc".into(), pinned: false },
            Playlist { name: "Bad".into(), uri: "nope".into(), pinned: false },
        ];
        normalize(&mut list);
        assert_eq!(list, [Playlist { name: "Playlist".into(), uri: "spotify:playlist:abc".into(), pinned: true }]);
    }

    #[test]
    fn reads_oembed_title() {
        assert_eq!(title_from_oembed(r#"{"title":" Deep Focus ","type":"rich"}"#), Some("Deep Focus".into()));
        assert_eq!(title_from_oembed(r#"{"type":"rich"}"#), None);
        assert_eq!(title_from_oembed("not json"), None);
    }
}
