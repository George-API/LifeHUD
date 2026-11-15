//! Robustness tests for the feed parsers: thousands of truncated, mutated and hostile inputs, which
//! must never panic (release builds abort on panic, which would take the whole HUD down).

use super::{netflix, news, reddit, sports, stocks, youtube};

/// Tiny deterministic PRNG (xorshift64*), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Characters that tend to break parsers: delimiters, quotes, escapes, multi-byte UTF-8.
const NASTY: &[&str] = &[
    "<", ">", "\"", "'", "&", "&amp;", "&#", "&#x", ";", "\t", "\n", "\r\n", "\\", "/", "{", "}", "[", "]", ":", ",",
    "é", "日本", "🏒", "\u{feff}", "\0", "-", "+", ".", "e9", "NaN", "1e308", "-0", " ", "ago", "v=", "r/", "@", "%",
];

/// A valid-UTF-8 mutation of `s`: cuts, duplicates, deletions and nasty insertions, at char
/// boundaries.
fn mutate(rng: &mut Rng, s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    for _ in 0..1 + rng.below(6) {
        let at = rng.below(chars.len() + 1);
        match rng.below(5) {
            0 => chars.truncate(at),
            1 if at < chars.len() => {
                let end = (at + rng.below(40)).min(chars.len());
                chars.drain(at..end);
            }
            2 => {
                let ins: Vec<char> = NASTY[rng.below(NASTY.len())].chars().collect();
                chars.splice(at..at, ins);
            }
            3 if !chars.is_empty() => {
                let from = rng.below(chars.len());
                let end = (from + rng.below(60)).min(chars.len());
                let piece: Vec<char> = chars[from..end].to_vec();
                chars.splice(at.min(chars.len())..at.min(chars.len()), piece);
            }
            _ => {}
        }
    }
    chars.into_iter().collect()
}

/// Every prefix (at char boundaries, stepped), then many random mutations.
fn inputs(seed: u64, fixture: &str, mutations: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bounds: Vec<usize> = fixture.char_indices().map(|(i, _)| i).chain([fixture.len()]).collect();
    let step = (bounds.len() / 400).max(1);
    for b in bounds.iter().step_by(step) {
        out.push(fixture[..*b].to_string());
    }
    let mut rng = Rng(seed);
    for _ in 0..mutations {
        out.push(mutate(&mut rng, fixture));
    }
    out.extend(NASTY.iter().map(|s| s.repeat(3)));
    out.push(String::new());
    out
}

const ESPN_BOARD: &str = r#"{"events":[{"id":"1","date":"2026-11-15T00:00Z","links":[{"href":"https://www.espn.com/nba/game/_/gameId/1"}],
 "competitions":[{"status":{"type":{"state":"in","shortDetail":"7:24 - 3rd"}},"competitors":[
 {"homeAway":"home","score":"88","winner":false,"team":{"id":"3","abbreviation":"NO","shortDisplayName":"Pelicans","logo":"https://a.espncdn.com/i/teamlogos/nba/500/scoreboard/no.png"},"records":[{"summary":"4-9"}]},
 {"homeAway":"away","score":"91","team":{"id":"7","abbreviation":"DEN","displayName":"Denver Nuggets"}}]}]}]}"#;
const ESPN_NEWS: &str = r#"{"articles":[{"headline":"Big win","published":"2026-11-14T17:09:28Z","links":{"web":{"href":"https://www.espn.com/x"}},"images":[{"url":"https://a.espncdn.com/p.jpg"}]}]}"#;
const ESPN_TEAMS: &str = r#"{"sports":[{"leagues":[{"teams":[{"team":{"id":"21","abbreviation":"TOR","displayName":"Toronto Maple Leafs"}}]}]}]}"#;
const YT_FEED: &str = r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/" xmlns="http://www.w3.org/2005/Atom"><title>Ch</title>
<entry><yt:videoId>SOwUWPtXQ9I</yt:videoId><title>Chips &amp; Dips</title><link rel="alternate" href="https://www.youtube.com/watch?v=SOwUWPtXQ9I"/>
<author><name>Fireship</name></author><published>2026-11-12T21:17:10+00:00</published><media:group><media:title>Chips</media:title><media:community><media:statistics views="1234"/></media:community></media:group></entry></feed>"#;
const YT_PAGE: &str = r#"<script>var ytInitialData = {"contents":[{"lockupViewModel":{"contentId":"hdMszZ2ah5Q","contentType":"LOCKUP_CONTENT_TYPE_VIDEO",
"contentImage":{"thumbnailViewModel":{"overlays":[{"thumbnailBottomOverlayViewModel":{"badges":[{"thumbnailBadgeViewModel":{"text":"15:14"}}]}}]}},
"metadata":{"lockupMetadataViewModel":{"title":{"content":"Review"},"metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[{"text":{"content":"5.5M"}},{"text":{"content":"3d ago"}}]}]}}}}}},
{"videoRenderer":{"videoId":"abcdefghijk","title":{"runs":[{"text":"Old"}]},"publishedTimeText":{"simpleText":"2 weeks ago"},"viewCountText":{"simpleText":"12,345 views"},"lengthText":{"simpleText":"1:02:03"}}}]};</script>
<link rel="canonical" href="https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ"><meta property="og:title" content="MKBHD &amp; co">"canonicalBaseUrl":"/@mkbhd""#;
const REDDIT: &str = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/"><entry><author><name>/u/a</name></author><category term="x" label="r/x"/>
<content type="html">&lt;a href=&quot;https://github.com/x&quot;&gt;[link]&lt;/a&gt;</content><media:thumbnail url="https://b.thumbs.redditmedia.com/a.jpg" />
<link href="https://www.reddit.com/r/x/comments/1/y/" /><published>2026-11-14T02:38:21+00:00</published><title>Hi &amp; bye</title></entry></feed>"#;
const NETFLIX: &str = "week\tcategory\tweekly_rank\tshow_title\tseason_title\tweekly_hours_viewed\truntime\tweekly_views\tcumulative_weeks_in_top_10
2026-11-08\tTV (English)\t1\tTaken\tTaken: Season 1\t25400000\t1.9667\t12900000\t1
2026-11-08\tFilms (Non-English)\t1\tCollision\tN/A\t30300000\t1.7\t17200000\t2
2026-11-01\tTV (English)\t1\tTaken\tTaken: Season 1\t1\t1\t9000000\t1
";
const SPARK: &str = r#"{"spark":{"result":[{"symbol":"AAPL","response":[{"meta":{"symbol":"AAPL","regularMarketPrice":356.27,"chartPreviousClose":354.88,"priceHint":2,
"currentTradingPeriod":{"regular":{"start":1,"end":2}}},"indicators":{"quote":[{"close":[355.0,null,356.27]}]}}]}]}}"#;
const SEARCH: &str =
    r#"{"quotes":[{"symbol":"VOD.L","exchDisp":"London","shortname":"VODAFONE","quoteType":"EQUITY"}]}"#;

#[test]
fn espn_parsers_never_panic() {
    let lg = sports::league("nba").unwrap();
    let fav = [sports::Favorite { league: "nba".into(), id: "7".into(), abbr: "DEN".into(), name: "Denver".into() }];
    for (seed, fixture) in [(1, ESPN_BOARD), (2, ESPN_NEWS), (3, ESPN_TEAMS)] {
        for input in inputs(seed, fixture, 3000) {
            let games = sports::parse_scoreboard(&input, lg, &fav);
            let _ = sports::select(games, 1_794_600_000);
            let _ = sports::parse_news(&input, lg);
            let _ = sports::parse_teams(&input, lg);
        }
    }
}

#[test]
fn youtube_parsers_never_panic() {
    for (seed, fixture) in [(4, YT_FEED), (5, YT_PAGE)] {
        for input in inputs(seed, fixture, 3000) {
            let _ = youtube::parse_feed(&input);
            let _ = youtube::parse_videos_page(&input, "x", 1_794_600_000);
            let _ = youtube::parse_channel_page(&input);
        }
    }
    let mut rng = Rng(6);
    for seed in [
        "3d ago",
        "Streamed 2 weeks ago",
        "5.5M views",
        "12,345",
        "https://youtu.be/dQw4w9WgXcQ?t=1",
        "@mkbhd",
        "youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ",
    ] {
        for _ in 0..2000 {
            let s = mutate(&mut rng, seed);
            let _ = youtube::relative_age(&s);
            let _ = youtube::parse_views(&s);
            let _ = youtube::video_id_from(&s);
            let _ = youtube::channel_page(&s);
        }
    }
    for n in [0, 1, 999, 1000, 999_499, 999_999, 1_000_000, 9_999_999, u64::MAX] {
        let _ = youtube::views_label(n);
    }
}

#[test]
fn reddit_parser_never_panics() {
    for input in inputs(7, REDDIT, 4000) {
        let _ = reddit::parse(&input);
    }
    let mut rng = Rng(8);
    for seed in ["r/rust", "https://www.reddit.com/r/rust/comments/x", "/r/AskHistorians/"] {
        for _ in 0..2000 {
            let _ = reddit::name_from(&mutate(&mut rng, seed));
        }
    }
}

#[test]
fn netflix_parser_never_panics() {
    for input in inputs(9, NETFLIX, 4000) {
        let chart = netflix::parse(&input);
        for kind in [netflix::Kind::Shows, netflix::Kind::Films] {
            let _ = netflix::top(&chart, kind, true);
            let _ = netflix::top(&chart, kind, false);
        }
        let _ = netflix::week_label(&chart.week);
    }
    for n in [0, 1, 999, 999_499, 999_500, 999_999, 1_000_000, u64::MAX] {
        let _ = netflix::views_label(n);
    }
}

#[test]
fn stocks_parsers_never_panic() {
    for (seed, fixture) in [(10, SPARK), (11, SEARCH)] {
        for input in inputs(seed, fixture, 3000) {
            for q in stocks::parse_spark(&input) {
                let _ = stocks::sparkline(&q.points, q.previous_close, 30.0);
                let _ = stocks::format_price(q.price, q.decimals);
                let _ = q.percent();
            }
            let _ = stocks::parse_search(&input);
        }
    }
    for v in [0.0, -0.0, 1e300, -1e300, f64::MAX, f64::MIN_POSITIVE, 0.005, -0.005, 999.999] {
        for d in 0..=6 {
            let _ = stocks::format_price(v, d);
        }
    }
    for pts in [vec![1e300, -1e300], vec![0.0, 0.0], vec![f64::MAX, f64::MAX], vec![1.0; 500]] {
        if let Some((line, area, base)) = stocks::sparkline(&pts, 0.0, 46.0) {
            assert!(
                !line.contains("NaN") && !line.contains("inf") && !area.contains("NaN") && base.is_finite(),
                "{line}"
            );
        }
    }
}

#[test]
fn date_parser_never_panics() {
    let mut rng = Rng(12);
    for seed in
        ["2026-11-14T13:05:00Z", "Sat, 14 Nov 2026 13:05:00 GMT", "2026-11-15T00:00Z", "2026-11-14T10:00:00-05:00"]
    {
        for _ in 0..3000 {
            let _ = news::parse_date(&mutate(&mut rng, seed));
        }
    }
}

/// Edge cases found in review; each failed before its fix.
#[test]
fn review_edge_cases() {
    // A "v=" inside another parameter isn't the video id.
    assert_eq!(
        youtube::video_id_from("https://www.youtube.com/watch?ev=1&v=dQw4w9WgXcQ").as_deref(),
        Some("dQw4w9WgXcQ")
    );
    assert_eq!(
        youtube::video_id_from("https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ").as_deref(),
        Some("dQw4w9WgXcQ")
    );
    // Legacy channel URLs still resolve.
    assert_eq!(
        youtube::channel_page("https://www.youtube.com/c/Veritasium").unwrap(),
        "https://www.youtube.com/c/Veritasium"
    );
    assert_eq!(
        youtube::channel_page("youtube.com/user/marquesbrownlee/videos").unwrap(),
        "https://www.youtube.com/user/marquesbrownlee"
    );
    // No "1000K".
    assert_eq!(youtube::views_label(999_999), "999K views");
    assert_eq!(youtube::views_label(1_950_000), "1.9M views");
    assert_eq!(youtube::views_label(2_000_000), "2M views");
    assert_eq!(netflix::views_label(999_999), "1M views");
    // Games without a usable date aren't shown as 1970.
    let board = ESPN_BOARD.replace("2026-11-15T00:00Z", "TBD");
    let games = sports::parse_scoreboard(&board, sports::league("nba").unwrap(), &[]);
    assert!(games.is_empty(), "undated games are skipped");
    // Unfollowing a league keeps its favourites for when it's followed again.
    let fav = sports::Favorite { league: "nhl".into(), id: "21".into(), abbr: "TOR".into(), name: "Toronto".into() };
    let mut p = sports::Prefs { leagues: vec!["nba".into()], favorites: vec![fav.clone()] };
    p.normalize();
    assert_eq!(p.favorites, [fav]);
}

/// Absurd numbers from a hostile page or feed: no overflow (a panic in debug, garbage in release).
#[test]
fn huge_numbers_dont_overflow() {
    assert_eq!(youtube::relative_age("9223372036854775 years ago"), None);
    assert_eq!(youtube::relative_age("99999999999999 weeks ago"), None);
    let page = r#"var ytInitialData = {"x":{"videoRenderer":{"videoId":"abcdefghijk","title":{"runs":[{"text":"t"}]},"publishedTimeText":{"simpleText":"9223372036854 seconds ago"}}}};</script>"#;
    let v = youtube::parse_videos_page(page, "c", i64::MIN + 5);
    assert_eq!(v.len(), 1);
    let board = ESPN_BOARD.replace("2026-11-15T00:00Z", "99999-12-31T23:59:59Z");
    let _ = sports::select(sports::parse_scoreboard(&board, sports::league("nba").unwrap(), &[]), i64::MAX);
    let spark = SPARK.replace("\"start\":1,\"end\":2", "\"start\":-9223372036854775808,\"end\":9223372036854775807");
    for q in stocks::parse_spark(&spark) {
        let _ = q.is_open(0);
    }
}
