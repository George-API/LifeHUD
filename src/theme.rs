//! Themes are a handful of colours; every other token is derived here so themes stay consistent.
//! Adding a theme is one entry in [`PALETTES`], or two (light and dark) for a theme that follows the
//! Light/Dark/Auto setting.

use slint::{Color, ModelRc, VecModel};

use crate::settings::Appearance;
use crate::{Theme, ThemeSwatch};

pub const CLASSIC: &str = "classic";

/// Colours are 0xAARRGGBB.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: bool,
    /// Laid over the acrylic blur; its alpha decides how much of the desktop shows through.
    pub tint: u32,
    /// Soft diagonal light that gives the theme its mood.
    pub ambient: &'static [u32],
    pub card: u32,
    pub card_border: u32,
    pub text: u32,
    /// `None` = the Windows accent colour.
    pub accent: Option<u32>,
    pub on_accent: u32,
    pub accent_play_button: bool,
    /// Decorative sticker art for mood themes (0 = none). See ui/stickers.slint.
    pub stickers: i32,
    /// Stronger secondary text and edges (Graphite, Slate).
    pub high_contrast: bool,
}

#[allow(clippy::too_many_arguments)] // a compact, table-like palette constructor
const fn p(
    id: &'static str,
    name: &'static str,
    dark: bool,
    tint: u32,
    ambient: &'static [u32],
    card: u32,
    card_border: u32,
    text: u32,
    accent: Option<u32>,
    on_accent: u32,
    accent_play_button: bool,
) -> Palette {
    Palette {
        id,
        name,
        dark,
        tint,
        ambient,
        card,
        card_border,
        text,
        accent,
        on_accent,
        accent_play_button,
        stickers: 0,
        high_contrast: false,
    }
}

impl Palette {
    const fn high_contrast(mut self) -> Self {
        self.high_contrast = true;
        self
    }

    const fn with_stickers(mut self, set: i32) -> Self {
        self.stickers = set;
        self
    }
}

/// Signature themes come as a pair, one light and one dark entry under the same id, and follow the
/// Light/Dark/Auto setting. Themes with a single entry (Slate and the moods) have one fixed look.
pub const PALETTES: &[Palette] = &[
    p(CLASSIC, "Classic", true, 0xB41A1A1E, &[], 0x12FFFFFF, 0x14FFFFFF, 0xFFFFFFFF, None, 0xFF000000, false),
    p(CLASSIC, "Classic", false, 0xA8F4F4F7, &[], 0x9EFFFFFF, 0x10000000, 0xE6000000, None, 0xFFFFFFFF, false),
    // True black for OLED screens, with stronger text and edges for readability.
    p(
        "graphite",
        "Graphite",
        true,
        0xF0000000,
        &[0x10FFFFFF, 0x00FFFFFF],
        0x14FFFFFF,
        0x33FFFFFF,
        0xFFFFFFFF,
        Some(0xFFE6E6EB),
        0xFF000000,
        false,
    )
    .high_contrast(),
    // Porcelain: bright white glass and near-black ink, the light counterpart to true black.
    p(
        "graphite",
        "Graphite",
        false,
        0xE8F4F4F6,
        &[0x30FFFFFF, 0x00FFFFFF],
        0xCCFFFFFF,
        0x24000000,
        0xFF111113,
        Some(0xFF1D1D1F),
        0xFFFFFFFF,
        false,
    )
    .high_contrast(),
    // Mid-tone tech grey, halfway between the dark and light sets: brushed-aluminium glass with an
    // ice-blue accent. One look in every mode, since it already sits in between.
    p(
        "slate",
        "Slate",
        true,
        0xD852565D,
        &[0x1CFFFFFF, 0x00FFFFFF, 0x1488B4E0],
        0x10FFFFFF,
        0x24FFFFFF,
        0xFFFFFFFF,
        Some(0xFFBEE6FF),
        0xFF1E2126,
        false,
    )
    .high_contrast(),
    // Gunmetal and steel blue.
    p(
        "titanium",
        "Titanium",
        true,
        0xCC23272D,
        &[0x24C8D6E8, 0x0023272D],
        0x14E6EEF8,
        0x1EDDE6F2,
        0xFFEEF2F6,
        Some(0xFFA9C2DD),
        0xFF16202C,
        false,
    ),
    // Cool silver.
    p(
        "titanium",
        "Titanium",
        false,
        0xC8D6DBE2,
        &[0x38FFFFFF, 0x2096A8C0],
        0x80F7F9FC,
        0x14000000,
        0xEB1A1F26,
        Some(0xFF4A6787),
        0xFFFFFFFF,
        false,
    ),
    // Deep navy with periwinkle.
    p(
        "midnight",
        "Midnight",
        true,
        0xC80B1220,
        &[0x332F4A80, 0x000B1220],
        0x14A8C0FF,
        0x1FA8C0FF,
        0xFFEEF2FF,
        Some(0xFF8FB3FF),
        0xFF0B1220,
        false,
    ),
    // Ice blue with sapphire.
    p(
        "midnight",
        "Midnight",
        false,
        0xC8E3EAF5,
        &[0x40FFFFFF, 0x248FB3FF],
        0xA8F8FAFF,
        0x161A2A50,
        0xEB0E1A33,
        Some(0xFF2F5BB7),
        0xFFFFFFFF,
        false,
    ),
    // Aubergine with lilac.
    p(
        "amethyst",
        "Amethyst",
        true,
        0xCC1C1726,
        &[0x38A07CE0, 0x001C1726, 0x18E0A8D8],
        0x16EDE0FF,
        0x22DCCBFF,
        0xFFF4F0FF,
        Some(0xFFC4ADFF),
        0xFF1C1230,
        false,
    ),
    // Lavender mist with violet.
    p(
        "amethyst",
        "Amethyst",
        false,
        0xC8E9E4F2,
        &[0x40FBF8FF, 0x24A890D8],
        0xB0FBF9FF,
        0x16301A50,
        0xEB211A33,
        Some(0xFF6A4FB0),
        0xFFFFFFFF,
        false,
    ),
    // Deep teal ink with aqua.
    p(
        "lagoon",
        "Lagoon",
        true,
        0xCC0E1F21,
        &[0x3440C8B8, 0x000E1F21, 0x1470A8E0],
        0x16C8F5F0,
        0x20B8EEE6,
        0xFFE8F7F5,
        Some(0xFF6FD8C8),
        0xFF06201C,
        false,
    ),
    // Pale sea glass with jade.
    p(
        "lagoon",
        "Lagoon",
        false,
        0xC8DCEEEC,
        &[0x40F2FFFC, 0x2470C8BE],
        0xB0F7FCFB,
        0x16004040,
        0xEB0C2A2A,
        Some(0xFF16736C),
        0xFFFFFFFF,
        false,
    ),
    // Smoked olive with a chartreuse-gold accent.
    p(
        "olive",
        "Olive",
        true,
        0xCC1E2117,
        &[0x34A8B26B, 0x001E2117, 0x14D9C27A],
        0x16E8EEC8,
        0x20E2E8C0,
        0xFFF2F4E6,
        Some(0xFFC5D17A),
        0xFF1C2010,
        false,
    ),
    // Stone and sage paper with olive.
    p(
        "olive",
        "Olive",
        false,
        0xC8E6E6D2,
        &[0x40F5F2DC, 0x24A3A86B],
        0xB0FBFAF2,
        0x16404520,
        0xEB252818,
        Some(0xFF5C6B2A),
        0xFFFFFFFF,
        false,
    ),
    // Champagne (id kept from Starlight): espresso with gold.
    p(
        "starlight",
        "Champagne",
        true,
        0xCC1F1A14,
        &[0x34E8C48A, 0x001F1A14, 0x14FFF0D0],
        0x16FFF1DA,
        0x22F5DDB4,
        0xFFFBF3E6,
        Some(0xFFE2C28A),
        0xFF231A0C,
        false,
    ),
    // Warm ivory with antique gold.
    p(
        "starlight",
        "Champagne",
        false,
        0xC2F3E9D8,
        &[0x44FFF4DC, 0x28E2C99A],
        0xA8FFFDF8,
        0x16583F1A,
        0xF02B2118,
        Some(0xFF846738),
        0xFFFFFFFF,
        false,
    ),
    // Cocoa with soft peach.
    p(
        "peach",
        "Peach",
        true,
        0xCC2A1D19,
        &[0x38FF9E7A, 0x002A1D19, 0x18FFC9A8],
        0x16FFE5D6,
        0x22FFD2BC,
        0xFFFFF1E8,
        Some(0xFFFFB494),
        0xFF2A1810,
        false,
    ),
    // Apricot cream with terracotta.
    p(
        "peach",
        "Peach",
        false,
        0xC8F8E3D3,
        &[0x48FFE8D6, 0x28F5A880],
        0xB0FFF8F2,
        0x16703A1A,
        0xEB3A2418,
        Some(0xFFB0502C),
        0xFFFFFFFF,
        false,
    ),
    // Wine with rose quartz.
    p(
        "rose",
        "Rosé",
        true,
        0xCC2A1A20,
        &[0x38E9A0AE, 0x002A1A20, 0x18F5C6B8],
        0x16FFE0E6,
        0x22FFD0DA,
        0xFFFFF0F2,
        Some(0xFFF2A9B6),
        0xFF2A1218,
        false,
    ),
    // Blush with clay rose.
    p(
        "rose",
        "Rosé",
        false,
        0xC2F5E0DA,
        &[0x44FFD6CA, 0x24E9AE9E],
        0xA8FFF8F6,
        0x14602A1E,
        0xEB3A2622,
        Some(0xFF925C4E),
        0xFFFFFFFF,
        false,
    ),
    // Polar night with northern-lights green and violet.
    p(
        "aurora",
        "Aurora",
        true,
        0xCC0B1A1F,
        &[0x3850E3A4, 0x000B1A1F, 0x2C9B7BFF],
        0x16D6FFF0,
        0x20C8F5E8,
        0xFFEAFBF6,
        Some(0xFF5EE0B0),
        0xFF06201A,
        false,
    ),
    // Frost with a cool jade.
    p(
        "aurora",
        "Aurora",
        false,
        0xC8E4EEF0,
        &[0x40F4FFFB, 0x2470D8B8, 0x1CA890F0],
        0xB0F8FCFC,
        0x16003A40,
        0xEB0C2630,
        Some(0xFF0F7A66),
        0xFFFFFFFF,
        false,
    ),
    // After sunset: coral fading through magenta into indigo.
    p(
        "dusk",
        "Dusk",
        true,
        0xCC1C1426,
        &[0x40FF8A5C, 0x24E0508C, 0x2C5A4FCF],
        0x16FFE4DC,
        0x22FFC8C0,
        0xFFFFF0EC,
        Some(0xFFFF9E7A),
        0xFF2A1218,
        false,
    ),
    // Golden hour: peach to lilac with a raspberry accent.
    p(
        "dusk",
        "Dusk",
        false,
        0xC8F7E6E0,
        &[0x48FFD0B0, 0x30F0A0C0, 0x28A898E8],
        0xB0FFF9F7,
        0x16602040,
        0xEB2E1A2E,
        Some(0xFFC4456A),
        0xFFFFFFFF,
        false,
    ),
    // Oxblood with brass.
    p(
        "bordeaux",
        "Bordeaux",
        true,
        0xCC22100F,
        &[0x388C1C2C, 0x0022100F, 0x18D4A85A],
        0x16FFE0DC,
        0x22F0C8A8,
        0xFFFBEFEA,
        Some(0xFFD9B26A),
        0xFF2A1408,
        false,
    ),
    // Linen with deep claret.
    p(
        "bordeaux",
        "Bordeaux",
        false,
        0xC8EFE2DC,
        &[0x44FFF2EA, 0x248C2A3A],
        0xB0FFFAF7,
        0x16501018,
        0xEB2A1216,
        Some(0xFF7A1F2E),
        0xFFFFFFFF,
        false,
    ),
    // Warm e-ink: sepia charcoal with cream text and a vermilion seal.
    p(
        "paper",
        "Paper",
        true,
        0xE61C1A17,
        &[0x14FFF4E0, 0x001C1A17],
        0x14FFF6E8,
        0x22FFF0DC,
        0xFFF2ECE0,
        Some(0xFFE8826A),
        0xFF1C1A17,
        false,
    ),
    // Off-white paper and near-black ink.
    p(
        "paper",
        "Paper",
        false,
        0xE6F1EDE4,
        &[0x30FFFDF6, 0x00F1EDE4],
        0xC8FBF9F3,
        0x1C2A2418,
        0xF01F1B16,
        Some(0xFFB8432E),
        0xFFFFFFFF,
        false,
    ),
    // Deep spruce with fresh mint.
    p(
        "mint",
        "Mint",
        true,
        0xCC10201B,
        &[0x3486F0C4, 0x0010201B, 0x14C8FFE8],
        0x16DAFFEE,
        0x20C4F7E0,
        0xFFEBFBF4,
        Some(0xFF7EEDC0),
        0xFF06261A,
        false,
    ),
    // Crisp mint cream with spearmint green.
    p(
        "mint",
        "Mint",
        false,
        0xC8E6F6EF,
        &[0x48F2FFF9, 0x2C8EE8C4],
        0xB8FBFFFD,
        0x16004A30,
        0xEB0E2A20,
        Some(0xFF13946A),
        0xFFFFFFFF,
        false,
    ),
    // Moods: one look each, with sticker art.
    p(
        "hearth",
        "Hearth",
        true,
        0xCC241A14,
        &[0x40FF9A4D, 0x10FF7A3D, 0x00241A14],
        0x18FFE2C4,
        0x22FFD8B0,
        0xFFFFF3E6,
        Some(0xFFFFB067),
        0xFF2A1A0E,
        false,
    )
    .with_stickers(1),
    p(
        "atelier",
        "Atelier",
        false,
        0xB4FBF7F0,
        &[0x40FF6B6B, 0x2848C9B0, 0x38FFD166],
        0xB8FFFFFF,
        0x14000000,
        0xEB1E1B2E,
        Some(0xFFC04448),
        0xFFFFFFFF,
        true,
    )
    .with_stickers(2),
    p(
        "metropolis",
        "Metropolis",
        true,
        0xC8161A21,
        &[0x30F5A623, 0x00161A21, 0x2433A1FD],
        0x12FFFFFF,
        0x1AFFFFFF,
        0xFFE8ECF2,
        Some(0xFFF5A623),
        0xFF161A21,
        false,
    )
    .with_stickers(3),
    p(
        "evergreen",
        "Evergreen",
        true,
        0xC80F1F18,
        &[0x3346A36B, 0x08A3C46B, 0x000F1F18],
        0x16C8F5D8,
        0x20C8F5D8,
        0xFFEAF5EE,
        Some(0xFF7BD389),
        0xFF0B1F12,
        false,
    )
    .with_stickers(4),
    p(
        "neon",
        "Neon",
        true,
        0xD00A0612,
        &[0x48FF2BD6, 0x000A0612, 0x4000F0FF],
        0x14FFFFFF,
        0x48FF2BD6,
        0xFFF4F0FF,
        Some(0xFF00F0FF),
        0xFF0A0612,
        true,
    )
    .with_stickers(5),
    // Sea glass, sand and sky: the light mood.
    p(
        "coast",
        "Coast",
        false,
        0xBCEEF6F4,
        &[0x3C8FDCCB, 0x24FFE0B5, 0x2C7FC1EA],
        0xB0FFFFFF,
        0x14003840,
        0xEB0F2A2E,
        Some(0xFF1C7C74),
        0xFFFFFFFF,
        true,
    )
    .with_stickers(6),
];

pub fn find(id: &str) -> Option<&'static Palette> {
    PALETTES.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// One entry per theme, in gallery order (the first variant of each id).
pub fn themes() -> Vec<&'static Palette> {
    PALETTES.iter().filter(|p| find(p.id).is_some_and(|f| std::ptr::eq(f, *p))).collect()
}

/// Whether a theme has both a light and a dark look, and so follows the Light/Dark/Auto setting.
pub fn adaptive(id: &str) -> bool {
    PALETTES.iter().filter(|p| p.id.eq_ignore_ascii_case(id)).count() > 1
}

/// The concrete palette for a theme id, picking its light or dark look by the appearance setting.
/// Unknown ids fall back to Classic.
pub fn resolve(id: &str, appearance: Appearance) -> Palette {
    let id = if find(id).is_some() { id } else { CLASSIC };
    let first = *find(id).unwrap_or(&PALETTES[0]);
    if !adaptive(id) {
        return first;
    }
    let dark = match appearance {
        Appearance::Dark => true,
        Appearance::Light => false,
        Appearance::Auto => crate::platform::system_prefers_dark(),
    };
    PALETTES.iter().find(|p| p.id.eq_ignore_ascii_case(id) && p.dark == dark).copied().unwrap_or(first)
}

pub fn argb(v: u32) -> Color {
    Color::from_argb_u8((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn with_alpha(c: Color, a: u8) -> Color {
    Color::from_argb_u8(a, c.red(), c.green(), c.blue())
}

/// The palette's ambient light as three gradient stops (start, middle, end).
pub fn ambient_stops(p: &Palette) -> [Color; 3] {
    let clear = Color::from_argb_u8(0, 0, 0, 0);
    match p.ambient {
        [] => [clear; 3],
        [a] => [argb(*a); 3],
        [a, b] => [argb(*a), mix(argb(*a), argb(*b)), argb(*b)],
        [a, b, c, ..] => [argb(*a), argb(*b), argb(*c)],
    }
}

fn mix(a: Color, b: Color) -> Color {
    let m = |x: u8, y: u8| ((x as u16 + y as u16) / 2) as u8;
    Color::from_argb_u8(m(a.alpha(), b.alpha()), m(a.red(), b.red()), m(a.green(), b.green()), m(a.blue(), b.blue()))
}

pub fn accent(p: &Palette) -> Color {
    p.accent.map(argb).unwrap_or_else(|| crate::platform::system_accent(p.dark))
}

/// Writes every design token for `p` into a window's Theme global.
pub fn apply(theme: &Theme, p: &Palette) {
    theme.on_sticker_tile(crate::stickers::tile);
    let d = p.dark;
    let text = argb(p.text);
    let ink = |a: u8| with_alpha(text, a);
    let tint = argb(p.tint);
    let accent = accent(p);

    theme.set_dark(d);
    theme.set_sticker_set(p.stickers);
    let [a, b, c] = ambient_stops(p);
    theme.set_ambient_a(a);
    theme.set_ambient_b(b);
    theme.set_ambient_c(c);
    let hc = p.high_contrast;
    theme.set_edge(ink(if hc {
        0x33
    } else if d {
        0x1A
    } else {
        0x14
    }));
    theme.set_card(argb(p.card));
    theme.set_card_border(argb(p.card_border));
    theme.set_text(text);
    theme.set_text2(ink(if hc {
        0xE0
    } else if d {
        0xB8
    } else {
        0xB4
    }));
    theme.set_text3(ink(if hc {
        0xA8
    } else if d {
        0x78
    } else {
        0x86
    }));
    theme.set_fill(ink(if d { 0x14 } else { 0x0C }));
    theme.set_fill_hover(ink(if d { 0x22 } else { 0x16 }));
    theme.set_input(ink(if d { 0x10 } else { 0x0A }));
    theme.set_divider(ink(if hc {
        0x2E
    } else if d {
        0x18
    } else {
        0x12
    }));
    theme.set_track(ink(if d { 0x2E } else { 0x1C }));
    theme.set_accent(accent);
    theme.set_on_accent(if p.accent.is_some() {
        argb(p.on_accent)
    } else if d {
        Color::from_rgb_u8(0, 0, 0)
    } else {
        Color::from_rgb_u8(255, 255, 255)
    });
    if p.accent_play_button {
        theme.set_play_bg(accent);
        theme.set_play_fg(argb(p.on_accent | 0xFF00_0000));
    } else {
        theme.set_play_bg(ink(if d { 0xF2 } else { 0xEB }));
        theme.set_play_fg(Color::from_rgb_u8(tint.red(), tint.green(), tint.blue()));
    }
}

/// Swatches for the Appearance gallery: signature themes (light and dark) first, then the moods.
pub fn swatches(selected: &str, appearance: Appearance) -> (ModelRc<ThemeSwatch>, ModelRc<ThemeSwatch>) {
    let (signature, moods): (Vec<ThemeSwatch>, Vec<ThemeSwatch>) = themes()
        .into_iter()
        .map(|entry| {
            let p = resolve(entry.id, appearance);
            let t = argb(p.tint);
            let [ambient_a, ambient_b, ambient_c] = ambient_stops(&p);
            ThemeSwatch {
                id: p.id.into(),
                name: p.name.into(),
                surface: Color::from_rgb_u8(t.red(), t.green(), t.blue()),
                ambient_a,
                ambient_b,
                ambient_c,
                card: argb(p.card),
                text: argb(p.text),
                accent: accent(&p),
                selected: p.id.eq_ignore_ascii_case(selected),
                sticker: p.stickers,
            }
        })
        .partition(|s| s.sticker == 0);
    (ModelRc::new(VecModel::from(signature)), ModelRc::new(VecModel::from(moods)))
}

/// Loads both halves of the Appearance gallery into a Settings window.
pub fn fill_gallery(st: &crate::SettingsState, selected: &str, appearance: Appearance) {
    let (signature, moods) = swatches(selected, appearance);
    st.set_themes(signature);
    st.set_moods(moods);
}

/// iOS-style temperature colour scale, by °C.
pub fn temperature_color(celsius: f64) -> Color {
    const SCALE: [(f64, (u8, u8, u8)); 6] = [
        (-10.0, (0x5E, 0x7B, 0xFF)),
        (0.0, (0x5A, 0xC8, 0xFA)),
        (10.0, (0x5F, 0xD6, 0x9A)),
        (18.0, (0xFF, 0xD6, 0x0A)),
        (25.0, (0xFF, 0x9F, 0x0A)),
        (33.0, (0xFF, 0x45, 0x3A)),
    ];
    if celsius <= SCALE[0].0 {
        let (r, g, b) = SCALE[0].1;
        return Color::from_rgb_u8(r, g, b);
    }
    for w in SCALE.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if celsius <= t1 {
            let f = (celsius - t0) / (t1 - t0);
            let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f) as u8;
            return Color::from_rgb_u8(lerp(c0.0, c1.0), lerp(c0.1, c1.1), lerp(c0.2, c1.2));
        }
    }
    let (r, g, b) = SCALE[5].1;
    Color::from_rgb_u8(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_theme_has_at_most_one_light_and_one_dark_look() {
        for t in themes() {
            let variants: Vec<_> = PALETTES.iter().filter(|p| p.id == t.id).collect();
            assert!(variants.len() <= 2, "{} has {} entries", t.id, variants.len());
            if let [a, b] = variants[..] {
                assert_ne!(a.dark, b.dark, "{} needs one light and one dark entry", t.id);
                assert_eq!(a.name, b.name);
            }
        }
    }

    #[test]
    fn resolve_picks_the_light_or_dark_look() {
        assert!(!resolve("peach", Appearance::Light).dark);
        assert!(resolve("peach", Appearance::Dark).dark);
        assert!(!resolve(CLASSIC, Appearance::Light).dark);
        // Single-look themes ignore the setting.
        assert_eq!(resolve("neon", Appearance::Light).id, "neon");
        assert!(resolve("slate", Appearance::Light).dark);
        assert_eq!(resolve("unknown", Appearance::Dark).id, CLASSIC);
        assert!(adaptive("olive") && !adaptive("hearth"));
        assert!(
            adaptive("aurora") && adaptive("dusk") && adaptive("bordeaux") && adaptive("paper") && adaptive("mint")
        );
    }

    #[test]
    fn temperature_scale_is_monotonic_in_red_at_the_hot_end() {
        assert!(temperature_color(33.0).red() >= temperature_color(18.0).red());
    }
}
