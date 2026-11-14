using System;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Threading;
using LifeHUD.Interop;
using LifeHUD.Models;
using LifeHUD.Services;
using LifeHUD.ViewModels;
using LifeHUD.Views;

namespace LifeHUD;

public partial class App : Application
{
    private const string InstanceMutexName = @"Local\LifeHUD.Instance";
    private const string ActivateEventName = @"Local\LifeHUD.Activate";
    private const string ExitEventName = @"Local\LifeHUD.Exit";

    private Mutex? _instanceMutex;
    private EventWaitHandle? _activateEvent, _exitEvent;
    private SettingsService? _settings;
    private ThemeService? _theme;
    private MediaService? _media;
    private WeatherViewModel? _weather;
    private NotesViewModel? _notes;
    private HudWindow? _hud;
    private SettingsWindow? _settingsWindow;
    private TrayIcon? _tray;
    private IntPtr _hudHandle;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        // Single instance: a second launch just opens Settings in the running one.
        _instanceMutex = new Mutex(true, InstanceMutexName, out bool isFirst);
        bool exitRequested = Array.Exists(e.Args, a => a.Equals("--exit", StringComparison.OrdinalIgnoreCase));
        if (!isFirst || exitRequested)
        {
            // "LifeHUD --exit" asks a running instance to quit cleanly (releasing its reserved screen space).
            try { EventWaitHandle.OpenExisting(exitRequested ? ExitEventName : ActivateEventName).Set(); } catch { }
            Shutdown();
            return;
        }
        _activateEvent = new EventWaitHandle(false, EventResetMode.AutoReset, ActivateEventName);
        ThreadPool.RegisterWaitForSingleObject(_activateEvent,
            (_, _) => Dispatcher.BeginInvoke(new Action(ShowSettings)), null, Timeout.Infinite, executeOnlyOnce: false);
        _exitEvent = new EventWaitHandle(false, EventResetMode.AutoReset, ExitEventName);
        ThreadPool.RegisterWaitForSingleObject(_exitEvent,
            (_, _) => Dispatcher.BeginInvoke(new Action(Shutdown)), null, Timeout.Infinite, executeOnlyOnce: true);

        AppPaths.Ensure();
        HookCrashHandlers();

        _settings = new SettingsService(AppPaths.Settings);
        _settings.Current.StartWithWindows = StartupService.IsEnabled();

        _theme = new ThemeService();
        _theme.Configure(_settings.Current.Theme, _settings.Current.Palette);
        _settings.Changed += (_, key) =>
        {
            if (key == SettingKeys.Theme) _theme.Configure(_settings.Current.Theme, _settings.Current.Palette);
        };

        var clock = new ClockViewModel(_settings);
        _weather = new WeatherViewModel(new WeatherService(), _settings, ShowSettings, Dispatcher);
        var tasks = new TasksViewModel(AppPaths.Tasks, clock);
        _notes = new NotesViewModel(AppPaths.Notes);
        _media = new MediaService(Dispatcher);
        var media = new MediaViewModel(_media, _settings);
        var hudVm = new HudViewModel(_settings, clock, _weather, tasks, _notes, media, ShowSettings, () => SetHudVisible(false));

        _hud = new HudWindow(hudVm, _settings, _theme);
        if (_settings.Current.HudVisible)
        {
            _hud.ShowHud();
            _hudHandle = _hud.Handle;
        }

        _tray = new TrayIcon(_settings, SetHudVisible, ShowSettings, Shutdown);

        _ = _media.StartAsync();
        _ = _weather.InitializeAsync();

        if (_settings.IsFirstRun) Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(ShowSettings));
    }

    public void SetHudVisible(bool visible)
    {
        if (_settings is null || _hud is null) return;
        if (visible) { _hud.ShowHud(); _hudHandle = _hud.Handle; }
        else _hud.HideHud();
        _settings.Update(s => s.HudVisible = visible, SettingKeys.Visibility);
    }

    public void ShowSettings()
    {
        if (_settings is null || _theme is null || _weather is null) return;
        if (_settingsWindow is { IsLoaded: true })
        {
            if (_settingsWindow.WindowState == WindowState.Minimized) _settingsWindow.WindowState = WindowState.Normal;
            _settingsWindow.Activate();
            return;
        }
        _settingsWindow = new SettingsWindow(new SettingsViewModel(_settings, _weather, _theme, SetHudVisible), _theme);
        _settingsWindow.Closed += (_, _) => _settingsWindow = null;
        _settingsWindow.Show();
        _settingsWindow.Activate();
    }

    private void FlushAll()
    {
        try { _notes?.Flush(); } catch { }
        try { _settings?.Flush(); } catch { }
    }

    private void HookCrashHandlers()
    {
        DispatcherUnhandledException += (_, e) =>
        {
            // A HUD should degrade, not disappear: log, keep running.
            Log.Write("Unhandled UI exception", e.Exception);
            e.Handled = true;
        };
        TaskScheduler.UnobservedTaskException += (_, e) =>
        {
            Log.Write("Unobserved task exception", e.Exception);
            e.SetObserved();
        };
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
        {
            Log.Write("Fatal exception", e.ExceptionObject as Exception);
            // Never leave a reserved strip of dead screen behind.
            AppBar.EmergencyRemove(_hudHandle);
            FlushAll();
        };
    }

    protected override void OnSessionEnding(SessionEndingCancelEventArgs e)
    {
        FlushAll();
        base.OnSessionEnding(e);
    }

    protected override void OnExit(ExitEventArgs e)
    {
        FlushAll();
        try { _hud?.Close(); } catch { }
        AppBar.EmergencyRemove(_hudHandle);
        _tray?.Dispose();
        _media?.Dispose();
        _activateEvent?.Dispose();
        _exitEvent?.Dispose();
        try { _instanceMutex?.ReleaseMutex(); } catch { }
        _instanceMutex?.Dispose();
        base.OnExit(e);
    }
}
