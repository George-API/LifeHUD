using System;
using System.Globalization;

namespace LifeHUD.Models;

public enum HudEdge { Top, Left }

public enum ThemePreference { System, Light, Dark }

public static class SettingKeys
{
    public const string Layout = nameof(Layout);
    public const string Size = nameof(Size);
    public const string Monitor = nameof(Monitor);
    public const string Theme = nameof(Theme);
    public const string Clock = nameof(Clock);
    public const string Weather = nameof(Weather);
    public const string Media = nameof(Media);
    public const string Startup = nameof(Startup);
    public const string Visibility = nameof(Visibility);
}

public sealed class AppSettings
{
    public const double TopSizeMin = 7, TopSizeMax = 18;
    public const double SideSizeMin = 10, SideSizeMax = 25;

    public HudEdge Layout { get; set; } = HudEdge.Top;
    /// <summary>Percent of monitor height reserved by the Top HUD.</summary>
    public double TopSizePercent { get; set; } = 10;
    /// <summary>Percent of monitor width reserved by the Life Sidebar.</summary>
    public double SideSizePercent { get; set; } = 14;
    /// <summary>Win32 device name (\\.\DISPLAY1); null means the primary display.</summary>
    public string? MonitorDevice { get; set; }
    public bool HudVisible { get; set; } = true;

    public ThemePreference Theme { get; set; } = ThemePreference.System;
    /// <summary>Id of a <see cref="ThemePalette"/>; "classic" follows <see cref="Theme"/> and the Windows accent.</summary>
    public string Palette { get; set; } = ThemePalettes.Classic;

    public bool Use24HourClock { get; set; } = CultureInfo.CurrentCulture.DateTimeFormat.ShortTimePattern.Contains('H');
    public bool ShowSeconds { get; set; }

    /// <summary>City for weather. Empty means "use Windows location".</summary>
    public string WeatherCity { get; set; } = string.Empty;
    public bool UseCelsius { get; set; } = SafeIsMetric();
    // Cached geocoding result for WeatherCity, so we don't geocode on every refresh.
    public string? ResolvedCityQuery { get; set; }
    public string? ResolvedCityName { get; set; }
    public double? ResolvedLatitude { get; set; }
    public double? ResolvedLongitude { get; set; }

    public bool PreferSpotify { get; set; } = true;
    public bool StartWithWindows { get; set; }

    public void Normalize()
    {
        TopSizePercent = Math.Clamp(double.IsFinite(TopSizePercent) ? TopSizePercent : 10, TopSizeMin, TopSizeMax);
        SideSizePercent = Math.Clamp(double.IsFinite(SideSizePercent) ? SideSizePercent : 14, SideSizeMin, SideSizeMax);
        WeatherCity ??= string.Empty;
        if (ThemePalettes.Find(Palette) is null) Palette = ThemePalettes.Classic;
    }

    private static bool SafeIsMetric()
    {
        try { return RegionInfo.CurrentRegion.IsMetric; } catch { return true; }
    }
}
