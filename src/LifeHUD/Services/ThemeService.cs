using System;
using System.Windows;
using System.Windows.Media;
using LifeHUD.Models;
using Microsoft.Win32;

namespace LifeHUD.Services;

/// <summary>
/// Owns every colour in the app. Brushes are written into Application resources and consumed via
/// DynamicResource, so a theme or accent change restyles everything live.
/// </summary>
public sealed class ThemeService
{
    private ThemePreference _preference;

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

    /// <summary>True when the HUD has a live blur behind it (so surfaces can be translucent).</summary>
    public bool HudTranslucent { get; private set; } = true;

    public event EventHandler? ThemeChanged;

    /// <summary>Acrylic tint the HUD window should use for the current theme.</summary>
    public Color HudAcrylicTint => IsDark ? Color.FromArgb(0xB4, 0x1A, 0x1A, 0x1E) : Color.FromArgb(0xA8, 0xF4, 0xF4, 0xF7);

    public void SetPreference(ThemePreference preference)
    {
        _preference = preference;
        Apply();
    }

    public void SetHudTranslucent(bool translucent)
    {
        if (HudTranslucent == translucent) return;
        HudTranslucent = translucent;
        Apply();
    }

    public void Apply()
    {
        IsDark = _preference switch
        {
            ThemePreference.Dark => true,
            ThemePreference.Light => false,
            _ => SystemPrefersDark(),
        };
        Accent = ReadAccent(IsDark);

        var r = Application.Current.Resources;
        void Set(string key, uint argb) => r[key] = Freeze(new SolidColorBrush(FromArgb(argb)));

        if (IsDark)
        {
            Set("HudTintBrush", HudTranslucent ? 0x14000000 : 0xFA1C1C20);
            Set("HudEdgeBrush", 0x1AFFFFFF);
            Set("CardBrush", 0x12FFFFFF);
            Set("CardBorderBrush", 0x14FFFFFF);
            Set("TextPrimaryBrush", 0xFFFFFFFF);
            Set("TextSecondaryBrush", 0xB8FFFFFF);
            Set("TextTertiaryBrush", 0x78FFFFFF);
            Set("ControlFillBrush", 0x14FFFFFF);
            Set("ControlFillHoverBrush", 0x22FFFFFF);
            Set("InputBrush", 0x10FFFFFF);
            Set("DividerBrush", 0x18FFFFFF);
            Set("TrackBrush", 0x2EFFFFFF);
            Set("ScrollThumbBrush", 0x50FFFFFF);
            Set("TooltipBrush", 0xFF2C2C30);
            Set("TooltipBorderBrush", 0x33FFFFFF);
            Set("PlayButtonBrush", 0xF2FFFFFF);
            Set("PlayButtonForegroundBrush", 0xFF111114);
            r["WeatherCloudBrush"] = Gradient(0xFFFFFFFF, 0xFFC3CCDA);
        }
        else
        {
            Set("HudTintBrush", HudTranslucent ? 0x26FFFFFF : 0xFAF3F3F6);
            Set("HudEdgeBrush", 0x14000000);
            Set("CardBrush", 0x9EFFFFFF);
            Set("CardBorderBrush", 0x10000000);
            Set("TextPrimaryBrush", 0xE6000000);
            Set("TextSecondaryBrush", 0xA0000000);
            Set("TextTertiaryBrush", 0x6E000000);
            Set("ControlFillBrush", 0x0C000000);
            Set("ControlFillHoverBrush", 0x16000000);
            Set("InputBrush", 0x0A000000);
            Set("DividerBrush", 0x12000000);
            Set("TrackBrush", 0x1C000000);
            Set("ScrollThumbBrush", 0x40000000);
            Set("TooltipBrush", 0xFFFAFAFA);
            Set("TooltipBorderBrush", 0x1A000000);
            Set("PlayButtonBrush", 0xEB111114);
            Set("PlayButtonForegroundBrush", 0xFFFFFFFF);
            r["WeatherCloudBrush"] = Gradient(0xFFC4CEDC, 0xFF7E8DA5);
        }

        r["AccentBrush"] = Freeze(new SolidColorBrush(Accent));
        r["AccentForegroundBrush"] = Freeze(new SolidColorBrush(IsDark ? Colors.Black : Colors.White));
        r["SpotifyBrush"] = Freeze(new SolidColorBrush(Color.FromRgb(0x1E, 0xD7, 0x60)));

        ThemeChanged?.Invoke(this, EventArgs.Empty);
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

    private static Color ReadAccent(bool dark)
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

    private static Color FromArgb(uint argb) =>
        Color.FromArgb((byte)(argb >> 24), (byte)(argb >> 16), (byte)(argb >> 8), (byte)argb);

    private static Brush Gradient(uint top, uint bottom) =>
        Freeze(new LinearGradientBrush(FromArgb(top), FromArgb(bottom), 90));

    private static T Freeze<T>(T f) where T : Freezable { f.Freeze(); return f; }
}
