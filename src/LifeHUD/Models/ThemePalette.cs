using System;
using System.Collections.Generic;
using System.Linq;

namespace LifeHUD.Models;

/// <summary>
/// A complete visual theme described by a handful of colours (ARGB hex). Every brush in the app is derived
/// from these, so themes stay internally consistent and adding one is a single entry below.
/// </summary>
/// <param name="Tint">Colour laid over the acrylic blur (its alpha sets how much of the desktop shows through).</param>
/// <param name="Ambient">Soft diagonal light gradient over the tint; gives each theme its mood. Empty = none.</param>
/// <param name="Card">Widget card fill.</param>
/// <param name="CardBorder">Hairline around cards.</param>
/// <param name="Text">Primary text; secondary/tertiary text and control fills are derived from it.</param>
/// <param name="Accent">Checkmarks, progress, highlights. Null = use the Windows accent colour.</param>
/// <param name="OnAccent">Foreground drawn on top of the accent (the task checkmark).</param>
/// <param name="AccentPlayButton">Use the accent for the big play button instead of the text colour.</param>
public sealed record ThemePalette(
    string Id,
    string Name,
    string Tagline,
    string Group,
    bool IsDark,
    uint Tint,
    uint[] Ambient,
    uint Card,
    uint CardBorder,
    uint Text,
    uint? Accent,
    uint OnAccent,
    bool AccentPlayButton = false);

public static class ThemePalettes
{
    public const string Classic = "classic";
    public const string SignatureGroup = "Signature";
    public const string MoodGroup = "Moods";

    /// <summary>Classic follows the Light/Dark/System mode and the Windows accent colour.</summary>
    public static readonly ThemePalette ClassicLight = new(Classic, "Classic", "Follows Windows", SignatureGroup, false,
        0xA8F4F4F7, Array.Empty<uint>(), 0x9EFFFFFF, 0x10000000, 0xE6000000, null, 0xFFFFFFFF);

    public static readonly ThemePalette ClassicDark = new(Classic, "Classic", "Follows Windows", SignatureGroup, true,
        0xB41A1A1E, Array.Empty<uint>(), 0x12FFFFFF, 0x14FFFFFF, 0xFFFFFFFF, null, 0xFF000000);

    public static readonly IReadOnlyList<ThemePalette> All = new[]
    {
        ClassicDark,

        // ---------------- Signature: quiet, premium, materials-inspired ----------------
        new ThemePalette("graphite", "Graphite", "Brushed aluminium, after dark", SignatureGroup, true,
            Tint: 0xC41C1C1F, Ambient: new[] { 0x1CFFFFFFu, 0x00FFFFFFu },
            Card: 0x14FFFFFF, CardBorder: 0x1CFFFFFF, Text: 0xFFF5F5F7, Accent: 0xFFD6D6DB, OnAccent: 0xFF1C1C1F),

        new ThemePalette("midnight", "Midnight", "Deep navy, soft moonlight", SignatureGroup, true,
            Tint: 0xC80B1220, Ambient: new[] { 0x332F4A80u, 0x000B1220u },
            Card: 0x14A8C0FF, CardBorder: 0x1FA8C0FF, Text: 0xFFEEF2FF, Accent: 0xFF8FB3FF, OnAccent: 0xFF0B1220),

        new ThemePalette("starlight", "Starlight", "Warm ivory and champagne gold", SignatureGroup, false,
            Tint: 0xC2F3E9D8, Ambient: new[] { 0x44FFF4DCu, 0x28E2C99Au },
            Card: 0xA8FFFDF8, CardBorder: 0x16583F1A, Text: 0xF02B2118, Accent: 0xFFAD8648, OnAccent: 0xFFFFFFFF),

        new ThemePalette("titanium", "Titanium", "Natural titanium, matte finish", SignatureGroup, false,
            Tint: 0xC8D3D1CC, Ambient: new[] { 0x38FFFFFFu, 0x20928A7Eu },
            Card: 0x80F7F6F3, CardBorder: 0x14000000, Text: 0xEB1F1E1C, Accent: 0xFF857A6C, OnAccent: 0xFFFFFFFF),

        new ThemePalette("rose", "Rosé", "Blush pink and rose gold", SignatureGroup, false,
            Tint: 0xC2F5E0DA, Ambient: new[] { 0x44FFD6CAu, 0x24E9AE9Eu },
            Card: 0xA8FFF8F6, CardBorder: 0x14602A1E, Text: 0xEB3A2622, Accent: 0xFFC27A68, OnAccent: 0xFFFFFFFF),

        // ---------------- Moods: expressive, atmospheric ----------------
        new ThemePalette("hearth", "Hearth", "Cozy. Firelight and warm wood", MoodGroup, true,
            Tint: 0xCC241A14, Ambient: new[] { 0x40FF9A4Du, 0x10FF7A3Du, 0x00241A14u },
            Card: 0x18FFE2C4, CardBorder: 0x22FFD8B0, Text: 0xFFFFF3E6, Accent: 0xFFFFB067, OnAccent: 0xFF2A1A0E),

        new ThemePalette("atelier", "Atelier", "Creative. Paper, paint and colour", MoodGroup, false,
            Tint: 0xB4FBF7F0, Ambient: new[] { 0x40FF6B6Bu, 0x2848C9B0u, 0x38FFD166u },
            Card: 0xB8FFFFFF, CardBorder: 0x14000000, Text: 0xEB1E1B2E, Accent: 0xFFFF5A5F, OnAccent: 0xFFFFFFFF,
            AccentPlayButton: true),

        new ThemePalette("metropolis", "Metropolis", "City. Slate, glass and streetlight", MoodGroup, true,
            Tint: 0xC8161A21, Ambient: new[] { 0x30F5A623u, 0x00161A21u, 0x2433A1FDu },
            Card: 0x12FFFFFF, CardBorder: 0x1AFFFFFF, Text: 0xFFE8ECF2, Accent: 0xFFF5A623, OnAccent: 0xFF161A21),

        new ThemePalette("evergreen", "Evergreen", "Nature. Forest floor and moss", MoodGroup, true,
            Tint: 0xC80F1F18, Ambient: new[] { 0x3346A36Bu, 0x08A3C46Bu, 0x000F1F18u },
            Card: 0x16C8F5D8, CardBorder: 0x20C8F5D8, Text: 0xFFEAF5EE, Accent: 0xFF7BD389, OnAccent: 0xFF0B1F12),

        new ThemePalette("neon", "Neon", "Cyberpunk. Neon haze and glow", MoodGroup, true,
            Tint: 0xD00A0612, Ambient: new[] { 0x48FF2BD6u, 0x000A0612u, 0x4000F0FFu },
            Card: 0x14FFFFFF, CardBorder: 0x48FF2BD6, Text: 0xFFF4F0FF, Accent: 0xFF00F0FF, OnAccent: 0xFF0A0612,
            AccentPlayButton: true),

        new ThemePalette("nebula", "Nebula", "Space. Violet dust and distant stars", MoodGroup, true,
            Tint: 0xCC0B0A1F, Ambient: new[] { 0x446A3DE8u, 0x24E8508Au, 0x000B0A1Fu },
            Card: 0x14C9B8FF, CardBorder: 0x26C9B8FF, Text: 0xFFEEEAFF, Accent: 0xFFB79CFF, OnAccent: 0xFF140B2E,
            AccentPlayButton: true),
    };

    public static ThemePalette? Find(string? id) =>
        All.FirstOrDefault(p => string.Equals(p.Id, id, StringComparison.OrdinalIgnoreCase));
}
