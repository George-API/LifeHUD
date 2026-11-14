using System;
using System.IO;
using System.Windows.Threading;
using LifeHUD.Models;

namespace LifeHUD.Services;

public sealed class SettingsService
{
    private readonly string _path;
    private readonly DispatcherTimer _saveTimer;

    public SettingsService(string path)
    {
        _path = path;
        IsFirstRun = !File.Exists(path);
        Current = JsonStore.Load(path, () => new AppSettings());
        Current.Normalize();
        // Debounced so slider drags don't hammer the disk.
        _saveTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(400) };
        _saveTimer.Tick += (_, _) => Flush();
    }

    public AppSettings Current { get; }
    public bool IsFirstRun { get; }

    /// <summary>Raised with a <see cref="SettingKeys"/> value after a change is applied.</summary>
    public event EventHandler<string>? Changed;

    public void Update(Action<AppSettings> mutate, string key)
    {
        mutate(Current);
        Current.Normalize();
        _saveTimer.Stop();
        _saveTimer.Start();
        Changed?.Invoke(this, key);
    }

    public void Flush()
    {
        _saveTimer.Stop();
        try { JsonStore.Save(_path, Current); }
        catch (Exception ex) { Log.Write("Saving settings failed", ex); }
    }
}
