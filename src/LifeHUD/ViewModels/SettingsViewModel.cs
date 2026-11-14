using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Reflection;
using System.Windows.Input;
using LifeHUD.Interop;
using LifeHUD.Models;
using LifeHUD.Services;

namespace LifeHUD.ViewModels;

public sealed record Option<T>(T Value, string Label);

public sealed class SettingsViewModel : ObservableObject
{
    private readonly SettingsService _settings;
    private readonly WeatherViewModel _weather;
    private readonly Action<bool> _setHudVisible;
    private bool _switchingLayout;

    public SettingsViewModel(SettingsService settings, WeatherViewModel weather, Action<bool> setHudVisible)
    {
        _settings = settings;
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
