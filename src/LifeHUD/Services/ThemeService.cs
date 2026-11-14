using System;
using System.Windows;
using System.Windows.Media;
using LifeHUD.Models;
using Microsoft.Win32;

namespace LifeHUD.Services;

/// <summary>
/// Owns every colour in the app. A <see cref="ThemePalette"/> is expanded into the full brush set, written
/// into Application resources and consumed via DynamicResource, so switching themes restyles everything live.
/// </summary>
public sealed class ThemeService
{
    private ThemePreference _mode;
    private string _paletteId = ThemePalettes.Classic;

    public ThemeService()
    {
        SystemEvents.UserPreferenceChanged += (_, e) =>
        {
            if (e.Category is UserPreferenceCategory.General or UserPreferenceCategory.Color or UserPreferenceCategory.VisualStyle)
                Application.Current?.Dispatcher.BeginInvoke(new Action(Apply));
        };
    }

    public bool IsDark { get; private set; }
    public Color Accent { get; private set; }
    public ThemePalette Palette { get; private set; } = ThemePalettes.ClassicDark;

    /// <summary>True when the HUD has a live blur behind it (so surfaces can be translucent).</summary>
    public bool HudTranslucent { get; private set; } = true;

    public event EventHandler? ThemeChanged;

    /// <summary>Acrylic tint the HUD window should use for the current theme.</summary>
    public Color HudAcrylicTint => C(Palette.Tint);

    public void Configure(ThemePreference mode, string? paletteId)
    {
        _mode = mode;
        _paletteId = string.IsNullOrWhiteSpace(paletteId) ? ThemePalettes.Classic : paletteId;
        Apply();
    }

    public void SetHudTranslucent(bool translucent)
    {
        if (HudTranslucent == translucent) return;
        HudTranslucent = translucent;
        Apply();
    }

    /// <summary>The concrete palette for an id, resolving Classic against the light/dark mode.</summary>
    public ThemePalette Resolve(string? id)
    {
        var p = ThemePalettes.Find(id);
        if (p is null || p.Id == ThemePalettes.Classic)
        {
            bool dark = _mode switch
            {
                ThemePreference.Dark => true,
                ThemePreference.Light => false,
                _ => SystemPrefersDark(),
            };
            return dark ? ThemePalettes.ClassicDark : ThemePalettes.ClassicLight;
        }
        return p;
    }

    public void Apply()
    {
        var p = Palette = Resolve(_paletteId);
        IsDark = p.IsDark;
        Accent = p.Accent is uint a ? C(a) : ReadAccent(IsDark);

        var r = Application.Current.Resources;
        void Set(string key, Color c) => r[key] = Freeze(new SolidColorBrush(c));

        Color text = C(p.Text);
        Color Ink(byte alpha) => Color.FromArgb(alpha, text.R, text.G, text.B);
        // Derived alphas are tuned separately for dark and light bases.
        bool d = IsDark;

        // Surface: translucent tint over acrylic, or an opaque version when blur isn't available.
        Color tint = C(p.Tint);
        r["HudTintBrush"] = HudTranslucent
            ? Ambient(p)
            : Ambient(p, Color.FromArgb(0xFA, tint.R, tint.G, tint.B));

        Set("HudEdgeBrush", Ink(d ? (byte)0x1A : (byte)0x14));
        Set("CardBrush", C(p.Card));
        Set("CardBorderBrush", C(p.CardBorder));
        Set("TextPrimaryBrush", text);
        Set("TextSecondaryBrush", Ink(d ? (byte)0xB8 : (byte)0xA0));
        Set("TextTertiaryBrush", Ink(d ? (byte)0x78 : (byte)0x6E));
        Set("ControlFillBrush", Ink(d ? (byte)0x14 : (byte)0x0C));
        Set("ControlFillHoverBrush", Ink(d ? (byte)0x22 : (byte)0x16));
        Set("InputBrush", Ink(d ? (byte)0x10 : (byte)0x0A));
        Set("DividerBrush", Ink(d ? (byte)0x18 : (byte)0x12));
        Set("TrackBrush", Ink(d ? (byte)0x2E : (byte)0x1C));
        Set("ScrollThumbBrush", Ink(d ? (byte)0x50 : (byte)0x40));

        // Tooltips float above everything as separate popups, so they need an opaque surface.
        Color tip = d ? Mix(Color.FromRgb(tint.R, tint.G, tint.B), Colors.White, 0.08) : Mix(Color.FromRgb(tint.R, tint.G, tint.B), Colors.White, 0.6);
        Set("TooltipBrush", tip);
        Set("TooltipBorderBrush", Ink(0x26));

        Set("AccentBrush", Accent);
        Set("AccentForegroundBrush", p.Accent is null ? (d ? Colors.Black : Colors.White) : C(p.OnAccent));
        Set("PlayButtonBrush", p.AccentPlayButton ? Accent : Ink(d ? (byte)0xF2 : (byte)0xEB));
        Set("PlayButtonForegroundBrush", p.AccentPlayButton ? C(p.OnAccent | 0xFF000000) : Color.FromRgb(tint.R, tint.G, tint.B));
        Set("SpotifyBrush", Color.FromRgb(0x1E, 0xD7, 0x60));
        r["WeatherCloudBrush"] = d ? Gradient(0xFFFFFFFF, 0xFFC3CCDA) : Gradient(0xFFC4CEDC, 0xFF7E8DA5);

        ThemeChanged?.Invoke(this, EventArgs.Empty);
    }

    /// <summary>The palette's ambient light as a brush, optionally laid over a solid base.</summary>
    public static Brush Ambient(ThemePalette p, Color? solidBase = null)
    {
        if (p.Ambient.Length == 0)
            return Freeze(new SolidColorBrush(solidBase ?? Colors.Transparent));

        var g = new LinearGradientBrush { StartPoint = new Point(0, 0), EndPoint = new Point(1, 1) };
        for (int i = 0; i < p.Ambient.Length; i++)
        {
            Color c = C(p.Ambient[i]);
            if (solidBase is Color b) c = Over(c, b); // flatten onto the opaque base
            g.GradientStops.Add(new GradientStop(c, p.Ambient.Length == 1 ? 0 : i / (double)(p.Ambient.Length - 1)));
        }
        return Freeze(g);
    }

    private static bool SystemPrefersDark()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            return key?.GetValue("AppsUseLightTheme") is int v && v == 0;
        }
        catch { return true; }
    }

    public static Color ReadAccent(bool dark)
    {
        try
        {
            var ui = new Windows.UI.ViewManagement.UISettings();
            var c = ui.GetColorValue(dark ? Windows.UI.ViewManagement.UIColorType.AccentLight2
                                          : Windows.UI.ViewManagement.UIColorType.AccentDark1);
            return Color.FromArgb(c.A, c.R, c.G, c.B);
        }
        catch
        {
            return Color.FromRgb(0x00, 0x78, 0xD4);
        }
    }

    public static Color C(uint argb) =>
        Color.FromArgb((byte)(argb >> 24), (byte)(argb >> 16), (byte)(argb >> 8), (byte)argb);

    /// <summary>Alpha-composite <paramref name="top"/> over an opaque <paramref name="bottom"/>.</summary>
    public static Color Over(Color top, Color bottom)
    {
        double a = top.A / 255.0;
        return Color.FromArgb(bottom.A,
            (byte)(top.R * a + bottom.R * (1 - a)),
            (byte)(top.G * a + bottom.G * (1 - a)),
            (byte)(top.B * a + bottom.B * (1 - a)));
    }

    private static Color Mix(Color a, Color b, double t) =>
        Color.FromRgb((byte)(a.R + (b.R - a.R) * t), (byte)(a.G + (b.G - a.G) * t), (byte)(a.B + (b.B - a.B) * t));

    private static Brush Gradient(uint top, uint bottom) =>
        Freeze(new LinearGradientBrush(C(top), C(bottom), 90));

    private static T Freeze<T>(T f) where T : Freezable { f.Freeze(); return f; }
}
