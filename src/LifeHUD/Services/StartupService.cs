using System;
using Microsoft.Win32;

namespace LifeHUD.Services;

/// <summary>Per-user "start with Windows" via the HKCU Run key (no admin rights needed).</summary>
public static class StartupService
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "LifeHUD";

    public static bool IsEnabled()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(RunKey);
            return key?.GetValue(ValueName) is string s && s.Length > 0;
        }
        catch { return false; }
    }

    public static void Apply(bool enable)
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(RunKey, writable: true) ?? Registry.CurrentUser.CreateSubKey(RunKey);
            if (enable && Environment.ProcessPath is string exe)
                key.SetValue(ValueName, $"\"{exe}\" --startup");
            else
                key.DeleteValue(ValueName, throwOnMissingValue: false);
        }
        catch (Exception ex)
        {
            Log.Write("Updating startup registration failed", ex);
        }
    }
}
