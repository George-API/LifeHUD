using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Net.NetworkInformation;
using System.Threading.Tasks;
using System.Windows.Input;
using System.Windows.Threading;
using LifeHUD.Models;
using LifeHUD.Services;
using Microsoft.Win32;

namespace LifeHUD.ViewModels;

public sealed record HourForecast(string Label, string TemperatureText, WeatherCondition Condition, bool IsDay);

public sealed record DayForecast(
    string Label, WeatherCondition Condition, string HighText, string LowText,
    double High, double Low, double RangeMin, double RangeMax, double Marker, bool Celsius);

public sealed class WeatherViewModel : ObservableObject
{
    private static readonly TimeSpan RefreshInterval = TimeSpan.FromMinutes(15);

    private readonly WeatherService _service;
    private readonly SettingsService _settings;
    private readonly DispatcherTimer _timer;
    private readonly Dispatcher _ui;
    private WeatherSnapshot? _snapshot;
    private GeoLocation? _deviceLocation;
    private DateTime _deviceLocationAt;
    private bool _busy, _stale;

    private bool _hasData;
    private string _location = "", _temperatureText = "", _description = "", _highLowText = "", _detailsText = "";
    private string _statusText = "", _placeholderText = "Loading weather…", _locationStatus = "";
    private WeatherCondition _condition = WeatherCondition.PartlyCloudy;
    private bool _isDay = true;
    private IReadOnlyList<HourForecast> _hours = Array.Empty<HourForecast>();
    private IReadOnlyList<DayForecast> _days = Array.Empty<DayForecast>();
    private IReadOnlyList<DayForecast> _upcomingDays = Array.Empty<DayForecast>();

    public WeatherViewModel(WeatherService service, SettingsService settings, Action openSettings, Dispatcher ui)
    {
        _service = service;
        _settings = settings;
        _ui = ui;
        OpenSettingsCommand = new RelayCommand(openSettings);
        RefreshCommand = new RelayCommand(() => _ = RefreshAsync());

        _timer = new DispatcherTimer(DispatcherPriority.Background) { Interval = RefreshInterval };
        _timer.Tick += (_, _) => _ = RefreshAsync();

        _settings.Changed += (_, key) =>
        {
            if (key == SettingKeys.Weather) _ = RefreshAsync();
            else if (key == SettingKeys.Clock && _snapshot != null) Apply(_snapshot);
        };

        NetworkChange.NetworkAvailabilityChanged += (_, e) =>
        {
            if (e.IsAvailable) _ui.BeginInvoke(new Action(() => RefreshSoon(TimeSpan.FromSeconds(3))));
        };
        SystemEvents.PowerModeChanged += (_, e) =>
        {
            if (e.Mode == PowerModes.Resume) _ui.BeginInvoke(new Action(() => RefreshSoon(TimeSpan.FromSeconds(8))));
        };
    }

    public ICommand OpenSettingsCommand { get; }
    public ICommand RefreshCommand { get; }

    public bool HasData { get => _hasData; private set => Set(ref _hasData, value); }
    public string Location { get => _location; private set => Set(ref _location, value); }
    public string TemperatureText { get => _temperatureText; private set => Set(ref _temperatureText, value); }
    public string Description { get => _description; private set => Set(ref _description, value); }
    public string HighLowText { get => _highLowText; private set => Set(ref _highLowText, value); }
    public string DetailsText { get => _detailsText; private set => Set(ref _detailsText, value); }
    public string StatusText { get => _statusText; private set => Set(ref _statusText, value); }
    public string PlaceholderText { get => _placeholderText; private set => Set(ref _placeholderText, value); }
    /// <summary>Human-readable explanation of where weather is coming from (shown in Settings).</summary>
    public string LocationStatus { get => _locationStatus; private set => Set(ref _locationStatus, value); }
    public WeatherCondition Condition { get => _condition; private set => Set(ref _condition, value); }
    public bool IsDay { get => _isDay; private set => Set(ref _isDay, value); }
    public IReadOnlyList<HourForecast> Hours { get => _hours; private set => Set(ref _hours, value); }
    public IReadOnlyList<DayForecast> Days { get => _days; private set => Set(ref _days, value); }
    public IReadOnlyList<DayForecast> UpcomingDays { get => _upcomingDays; private set => Set(ref _upcomingDays, value); }

    public async Task InitializeAsync()
    {
        var cached = JsonStore.Load<WeatherSnapshot?>(AppPaths.WeatherCache, () => null);
        if (cached != null && cached.Celsius == _settings.Current.UseCelsius && cached.LocationKey == CurrentLocationKey())
        {
            _snapshot = cached;
            _stale = DateTimeOffset.Now - cached.FetchedAt > RefreshInterval;
            Apply(cached);
        }
        _timer.Start();
        await RefreshAsync();
    }

    private void RefreshSoon(TimeSpan delay)
    {
        var t = new DispatcherTimer { Interval = delay };
        t.Tick += (_, _) => { t.Stop(); _ = RefreshAsync(); };
        t.Start();
    }

    private string CurrentLocationKey()
    {
        var city = _settings.Current.WeatherCity.Trim();
        return city.Length == 0 ? "device" : "city:" + city.ToLowerInvariant();
    }

    public async Task RefreshAsync()
    {
        if (_busy) return;
        _busy = true;
        try
        {
            var key = CurrentLocationKey();
            var loc = await ResolveLocationAsync();
            if (loc is null)
            {
                if (_snapshot is null || _snapshot.LocationKey != key) { HasData = false; _snapshot = null; }
                return;
            }

            var snap = await _service.FetchAsync(loc, _settings.Current.UseCelsius, key);
            _snapshot = snap;
            _stale = false;
            Apply(snap);
            LocationStatus = $"Showing weather for {snap.LocationName}.";
            try { JsonStore.Save(AppPaths.WeatherCache, snap); } catch { }
        }
        catch (Exception ex)
        {
            Log.Write("Weather refresh failed", ex);
            _stale = true;
            if (_snapshot != null && _snapshot.LocationKey == CurrentLocationKey()) Apply(_snapshot);
            else
            {
                HasData = false;
                PlaceholderText = "Offline — weather will update when you're back online.";
            }
        }
        finally
        {
            _busy = false;
        }
    }

    private async Task<GeoLocation?> ResolveLocationAsync()
    {
        var s = _settings.Current;
        var city = s.WeatherCity.Trim();

        if (city.Length > 0)
        {
            if (string.Equals(s.ResolvedCityQuery, city, StringComparison.OrdinalIgnoreCase)
                && s.ResolvedLatitude is double lat && s.ResolvedLongitude is double lon)
                return new GeoLocation(lat, lon, s.ResolvedCityName ?? city);

            var found = await _service.GeocodeAsync(city);
            if (found is null)
            {
                PlaceholderText = $"Couldn't find “{city}”. Check the city in Settings.";
                LocationStatus = $"No match for “{city}”. Try adding the country, e.g. “Paris, France”.";
                return null;
            }
            // Persist silently (not via Update, which would trigger another refresh).
            s.ResolvedCityQuery = city;
            s.ResolvedCityName = found.Name;
            s.ResolvedLatitude = found.Latitude;
            s.ResolvedLongitude = found.Longitude;
            _settings.Flush();
            return found;
        }

        if (_deviceLocation != null && DateTime.Now - _deviceLocationAt < TimeSpan.FromMinutes(60))
            return _deviceLocation;

        var device = await _service.GetDeviceLocationAsync();
        if (device is null)
        {
            PlaceholderText = "Set your city in Settings, or turn on Windows location.";
            LocationStatus = "Windows location is off or denied. Enter a city to get weather.";
            return _deviceLocation; // last known, if any
        }
        _deviceLocation = device;
        _deviceLocationAt = DateTime.Now;
        return device;
    }

    private void Apply(WeatherSnapshot s)
    {
        bool h24 = _settings.Current.Use24HourClock;
        var culture = CultureInfo.CurrentCulture;
        string T(double v) => $"{Math.Round(v):0}°";

        Location = s.LocationName;
        TemperatureText = T(s.Temperature);
        Condition = WeatherCodes.ToCondition(s.Code);
        IsDay = s.IsDay;
        Description = WeatherCodes.Describe(s.Code, s.IsDay);

        var today = s.Days.FirstOrDefault();
        HighLowText = today is null ? "" : $"H:{T(today.High)}  L:{T(today.Low)}";
        DetailsText = $"Feels like {T(s.FeelsLike)}  ·  Humidity {s.Humidity:0}%  ·  Wind {s.WindSpeed:0} {(s.Celsius ? "km/h" : "mph")}";

        // Skip hours that have passed since the snapshot was taken (matters when showing cached data).
        int skip = Math.Max(0, (int)Math.Floor((DateTimeOffset.Now - s.FetchedAt).TotalHours));
        Hours = s.Hours.Skip(skip).Take(6).Select((h, i) => new HourForecast(
            i == 0 ? "Now" : h.Time.ToString(h24 ? "HH" : "h tt", culture).ToLower(culture).Replace(" ", ""),
            T(i == 0 ? s.Temperature : h.Temperature),
            WeatherCodes.ToCondition(h.Code),
            h.IsDay)).ToList();

        if (s.Days.Count > 0)
        {
            double min = s.Days.Min(d => d.Low), max = s.Days.Max(d => d.High);
            Days = s.Days.Select((d, i) => new DayForecast(
                i == 0 ? "Today" : d.Date.ToString("ddd", culture),
                WeatherCodes.ToCondition(d.Code), T(d.High), T(d.Low), d.High, d.Low, min, max,
                i == 0 ? s.Temperature : double.NaN, s.Celsius)).ToList();
            UpcomingDays = Days.Skip(1).Take(3).ToList();
        }

        var age = DateTimeOffset.Now - s.FetchedAt;
        StatusText = _stale
            ? $"Offline · {s.FetchedAt.LocalDateTime.ToString("t", culture)}"
            : age > TimeSpan.FromMinutes(45) ? $"Updated {s.FetchedAt.LocalDateTime.ToString("t", culture)}" : "";

        HasData = true;
    }
}
