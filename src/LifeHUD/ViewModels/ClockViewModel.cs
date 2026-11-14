using System;
using System.Globalization;
using System.Windows.Threading;
using LifeHUD.Models;
using LifeHUD.Services;

namespace LifeHUD.ViewModels;

public sealed class ClockViewModel : ObservableObject
{
    private readonly SettingsService _settings;
    private readonly DispatcherTimer _timer;
    private DateTime _lastDay;
    private string _time = "", _period = "", _dateLong = "", _greeting = "", _dateShort = "";

    public ClockViewModel(SettingsService settings)
    {
        _settings = settings;
        _lastDay = DateTime.Today;
        _timer = new DispatcherTimer(DispatcherPriority.Background);
        _timer.Tick += (_, _) => Tick();
        _settings.Changed += (_, key) => { if (key == SettingKeys.Clock) Update(DateTime.Now); };
        Update(DateTime.Now);
        ScheduleNext();
        _timer.Start();
    }

    /// <summary>Raised once when the calendar day rolls over.</summary>
    public event EventHandler? DayChanged;

    public string Time { get => _time; private set => Set(ref _time, value); }
    public string Period { get => _period; private set => Set(ref _period, value); }
    public string DateLong { get => _dateLong; private set => Set(ref _dateLong, value); }
    public string DateShort { get => _dateShort; private set => Set(ref _dateShort, value); }
    public string Greeting { get => _greeting; private set => Set(ref _greeting, value); }

    private void Tick()
    {
        var now = DateTime.Now;
        Update(now);
        if (now.Date != _lastDay)
        {
            _lastDay = now.Date;
            DayChanged?.Invoke(this, EventArgs.Empty);
        }
        ScheduleNext();
    }

    /// <summary>Wake just after the next second boundary so the display never lags.</summary>
    private void ScheduleNext()
    {
        int ms = 1000 - DateTime.Now.Millisecond + 15;
        _timer.Interval = TimeSpan.FromMilliseconds(ms);
    }

    private void Update(DateTime now)
    {
        var s = _settings.Current;
        var culture = CultureInfo.CurrentCulture;
        string fmt = s.Use24HourClock ? "HH:mm" : "h:mm";
        if (s.ShowSeconds) fmt += ":ss";
        Time = now.ToString(fmt, culture);

        if (s.Use24HourClock) Period = "";
        else
        {
            string p = now.Hour < 12 ? culture.DateTimeFormat.AMDesignator : culture.DateTimeFormat.PMDesignator;
            Period = string.IsNullOrEmpty(p) ? (now.Hour < 12 ? "AM" : "PM") : p;
        }

        string monthDay = now.ToString(culture.DateTimeFormat.MonthDayPattern, culture);
        DateLong = $"{now.ToString("dddd", culture)}, {monthDay}";
        DateShort = $"{now.ToString("ddd", culture)} {monthDay}";

        Greeting = now.Hour switch
        {
            >= 5 and < 12 => "Good morning",
            >= 12 and < 17 => "Good afternoon",
            >= 17 and < 22 => "Good evening",
            _ => "Good night",
        };
    }
}
