using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Reflection;
using System.Windows;
using System.Windows.Media;
using System.Windows.Input;
using LifeHUD.Interop;
using LifeHUD.Models;
using LifeHUD.Services;

namespace LifeHUD.ViewModels;

public sealed record Option<T>(T Value, string Label);

/// <summary>One swatch in the theme gallery, with brushes for a miniature preview of the HUD.</summary>
public sealed class PaletteOption : ObservableObject
{
    private readonly Func<string, ThemePalette> _resolve;
    private readonly Action<string> _select;
    private bool _isSelected;

    public PaletteOption(ThemePalette palette, Func<string, ThemePalette> resolve, Action<string> select, bool selected)
    {
        Id = palette.Id;
        Name = palette.Name;
        Tagline = palette.Tagline;
        Group = palette.Group;
        _resolve = resolve;
        _select = select;
        _isSelected = selected;
    }

    public string Id { get; }
    public string Name { get; }
    public string Tagline { get; }
    public string Group { get; }

    private ThemePalette P => _resolve(Id);
    public Brush SurfaceBrush => Solid(P.Tint, opaque: true);
    public Brush AmbientBrush => ThemeService.Ambient(P);
    public Brush CardBrush => Solid(P.Card);
    public Brush CardBorderBrush => Solid(P.CardBorder);
    public Brush TextBrush => Solid(P.Text);
    public Brush AccentBrush
    {
        get
        {
            if (P.Accent is uint a) return Solid(a);
            var b = new SolidColorBrush(ThemeService.ReadAccent(P.IsDark)); // Classic uses the Windows accent
            b.Freeze();
            return b;
        }
    }

    public bool IsSelected
    {
        get => _isSelected;
        set
        {
            if (!Set(ref _isSelected, value) || !value) return;
            _select(Id);
        }
    }

    internal void SetSelected(bool value) { if (_isSelected != value) { _isSelected = value; Raise(nameof(IsSelected)); } }

    /// <summary>Classic changes with light/dark mode, so its preview must be rebuilt.</summary>
    internal void RefreshPreview() =>
        Raise(nameof(SurfaceBrush), nameof(AmbientBrush), nameof(CardBrush), nameof(CardBorderBrush), nameof(TextBrush), nameof(AccentBrush));

    private static Brush Solid(uint argb, bool opaque = false)
    {
        var c = ThemeService.C(opaque ? argb | 0xFF000000 : argb);
        var b = new SolidColorBrush(c);
        b.Freeze();
        return b;
    }
}

public sealed class SettingsViewModel : ObservableObject
{
    private readonly SettingsService _settings;
    private readonly WeatherViewModel _weather;
    private readonly Action<bool> _setHudVisible;
    private readonly ThemeService _themeService;
    private readonly IReadOnlyList<PaletteOption> _allPalettes;
    private bool _switchingLayout;

    public SettingsViewModel(SettingsService settings, WeatherViewModel weather, ThemeService theme, Action<bool> setHudVisible)
    {
        _settings = settings;
        _themeService = theme;
        var palettes = ThemePalettes.All.Select(p => new PaletteOption(p, theme.Resolve, SelectPalette,
            string.Equals(p.Id, settings.Current.Palette, StringComparison.OrdinalIgnoreCase))).ToList();
        SignaturePalettes = palettes.Where(p => p.Group == ThemePalettes.SignatureGroup).ToList();
        MoodPalettes = palettes.Where(p => p.Group == ThemePalettes.MoodGroup).ToList();
        _allPalettes = palettes;
        theme.ThemeChanged += (_, _) => { foreach (var p in _allPalettes.Where(p => p.Id == ThemePalettes.Classic)) p.RefreshPreview(); };
        _weather = weather;
        _setHudVisible = setHudVisible;

        var monitors = MonitorService.GetAll();
        Monitors = monitors.Select((m, i) => new Option<string>(m.DeviceName,
            $"Display {i + 1}{(m.IsPrimary ? " (main)" : "")} · {m.Bounds.Width} × {m.Bounds.Height}")).ToList();

        OpenDataFolderCommand = new RelayCommand(() =>
        {
            try { Process.Start(new ProcessStartInfo(AppPaths.Root) { UseShellExecute = true }); } catch { }
        });
        ToggleHudCommand = new RelayCommand(() =>
        {
            _setHudVisible(!_settings.Current.HudVisible);
            Raise(nameof(HudToggleText));
        });

        _weather.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName == nameof(WeatherViewModel.LocationStatus)) Raise(nameof(WeatherStatus));
        };
        _settings.Changed += (_, key) =>
        {
            if (key == SettingKeys.Layout && !_switchingLayout) RaiseLayout();
            if (key == SettingKeys.Visibility) Raise(nameof(HudToggleText));
        };
    }

    private AppSettings S => _settings.Current;

    // ---------- Layout ----------
    public bool IsTopLayout
    {
        get => S.Layout == HudEdge.Top;
        set { if (value) SetLayout(HudEdge.Top); }
    }

    public bool IsSideLayout
    {
        get => S.Layout == HudEdge.Left;
        set { if (value) SetLayout(HudEdge.Left); }
    }

    private void SetLayout(HudEdge edge)
    {
        if (S.Layout == edge) return;
        _settings.Update(s => s.Layout = edge, SettingKeys.Layout);
        RaiseLayout();
    }

    private void RaiseLayout()
    {
        // Slider range changes coerce its value; ignore those echoes so one layout's size never leaks into the other.
        _switchingLayout = true;
        try { Raise(nameof(IsTopLayout), nameof(IsSideLayout), nameof(SizeMin), nameof(SizeMax), nameof(SizePercent), nameof(SizeCaption)); }
        finally { _switchingLayout = false; }
    }

    public double SizeMin => IsTopLayout ? AppSettings.TopSizeMin : AppSettings.SideSizeMin;
    public double SizeMax => IsTopLayout ? AppSettings.TopSizeMax : AppSettings.SideSizeMax;

    public double SizePercent
    {
        get => IsTopLayout ? S.TopSizePercent : S.SideSizePercent;
        set
        {
            if (_switchingLayout) return;
            double v = Math.Round(value * 2) / 2;
            if (Math.Abs(v - SizePercent) < 0.01) return;
            _settings.Update(s => { if (s.Layout == HudEdge.Top) s.TopSizePercent = v; else s.SideSizePercent = v; }, SettingKeys.Size);
            Raise(nameof(SizePercent), nameof(SizeCaption));
        }
    }

    public string SizeCaption => $"{SizePercent:0.#}% of screen {(IsTopLayout ? "height" : "width")}";

    public IReadOnlyList<Option<string>> Monitors { get; }

    public string? SelectedMonitor
    {
        get => Monitors.FirstOrDefault(m => string.Equals(m.Value, S.MonitorDevice, StringComparison.OrdinalIgnoreCase))?.Value
               ?? Monitors.FirstOrDefault()?.Value;
        set
        {
            if (value is null || string.Equals(value, S.MonitorDevice, StringComparison.OrdinalIgnoreCase)) return;
            _settings.Update(s => s.MonitorDevice = value, SettingKeys.Monitor);
            Raise(nameof(SelectedMonitor));
        }
    }

    // ---------- Appearance ----------
    public IReadOnlyList<Option<ThemePreference>> ThemeOptions { get; } = new[]
    {
        new Option<ThemePreference>(ThemePreference.System, "Match Windows"),
        new Option<ThemePreference>(ThemePreference.Light, "Light"),
        new Option<ThemePreference>(ThemePreference.Dark, "Dark"),
    };

    public ThemePreference SelectedTheme
    {
        get => S.Theme;
        set { if (S.Theme != value) { _settings.Update(s => s.Theme = value, SettingKeys.Theme); Raise(nameof(SelectedTheme)); } }
    }

    public IReadOnlyList<PaletteOption> SignaturePalettes { get; }
    public IReadOnlyList<PaletteOption> MoodPalettes { get; }

    /// <summary>Light/Dark mode only applies to Classic; every other theme has its own fixed tone.</summary>
    public bool IsModeEditable => string.Equals(S.Palette, ThemePalettes.Classic, StringComparison.OrdinalIgnoreCase);

    public string CurrentPaletteText
    {
        get
        {
            var p = _themeService.Resolve(S.Palette);
            return $"{p.Name}. {p.Tagline}";
        }
    }

    private void SelectPalette(string id)
    {
        foreach (var p in _allPalettes) p.SetSelected(p.Id == id);
        if (string.Equals(S.Palette, id, StringComparison.OrdinalIgnoreCase)) return;
        _settings.Update(s => s.Palette = id, SettingKeys.Theme);
        Raise(nameof(IsModeEditable), nameof(CurrentPaletteText));
    }

    public bool Use24HourClock
    {
        get => S.Use24HourClock;
        set { if (S.Use24HourClock != value) { _settings.Update(s => s.Use24HourClock = value, SettingKeys.Clock); Raise(nameof(Use24HourClock)); } }
    }

    public bool ShowSeconds
    {
        get => S.ShowSeconds;
        set { if (S.ShowSeconds != value) { _settings.Update(s => s.ShowSeconds = value, SettingKeys.Clock); Raise(nameof(ShowSeconds)); } }
    }

    // ---------- Weather ----------
    public string City
    {
        get => S.WeatherCity;
        set
        {
            string v = (value ?? "").Trim();
            if (v == S.WeatherCity) return;
            _settings.Update(s => s.WeatherCity = v, SettingKeys.Weather);
            Raise(nameof(City));
        }
    }

    public string WeatherStatus => _weather.LocationStatus;

    public IReadOnlyList<Option<bool>> UnitOptions { get; } = new[]
    {
        new Option<bool>(true, "Celsius (°C)"),
        new Option<bool>(false, "Fahrenheit (°F)"),
    };

    public bool UseCelsius
    {
        get => S.UseCelsius;
        set { if (S.UseCelsius != value) { _settings.Update(s => s.UseCelsius = value, SettingKeys.Weather); Raise(nameof(UseCelsius)); } }
    }

    // ---------- Media ----------
    public bool PreferSpotify
    {
        get => S.PreferSpotify;
        set { if (S.PreferSpotify != value) { _settings.Update(s => s.PreferSpotify = value, SettingKeys.Media); Raise(nameof(PreferSpotify)); } }
    }

    // ---------- General ----------
    public bool StartWithWindows
    {
        get => S.StartWithWindows;
        set
        {
            if (S.StartWithWindows == value) return;
            StartupService.Apply(value);
            _settings.Update(s => s.StartWithWindows = value, SettingKeys.Startup);
            Raise(nameof(StartWithWindows));
        }
    }

    public string HudToggleText => S.HudVisible ? "Hide HUD" : "Show HUD";
    public ICommand ToggleHudCommand { get; }
    public ICommand OpenDataFolderCommand { get; }
    public string DataFolder => AppPaths.Root;

    public string VersionText =>
        $"Life HUD {Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "0.1.0"}";
}
