using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using static LifeHUD.Interop.NativeMethods;

namespace LifeHUD.Interop;

internal sealed record MonitorInfo(IntPtr Handle, string DeviceName, RECT Bounds, RECT WorkArea, bool IsPrimary, uint Dpi)
{
    public double Scale => Dpi / 96.0;
}

internal static class MonitorService
{
    public static IReadOnlyList<MonitorInfo> GetAll()
    {
        var result = new List<MonitorInfo>();
        MonitorEnumProc callback = (IntPtr hMonitor, IntPtr _, ref RECT _, IntPtr _) =>
        {
            var info = Query(hMonitor);
            if (info != null) result.Add(info);
            return true;
        };
        EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, callback, IntPtr.Zero);
        GC.KeepAlive(callback);
        // Stable, human-friendly order: primary first, then left-to-right.
        return result.OrderByDescending(m => m.IsPrimary).ThenBy(m => m.Bounds.Left).ThenBy(m => m.Bounds.Top).ToList();
    }

    public static MonitorInfo Find(string? deviceName)
    {
        var all = GetAll();
        return all.FirstOrDefault(m => deviceName != null && string.Equals(m.DeviceName, deviceName, StringComparison.OrdinalIgnoreCase))
               ?? all.FirstOrDefault(m => m.IsPrimary)
               ?? all.First();
    }

    public static MonitorInfo? Query(IntPtr hMonitor)
    {
        var mi = new MONITORINFOEX { cbSize = Marshal.SizeOf<MONITORINFOEX>(), szDevice = string.Empty };
        if (!GetMonitorInfo(hMonitor, ref mi)) return null;
        uint dpi = 96;
        try
        {
            if (GetDpiForMonitor(hMonitor, 0, out uint dx, out _) == 0 && dx > 0) dpi = dx;
        }
        catch { /* pre-8.1 */ }
        return new MonitorInfo(hMonitor, mi.szDevice, mi.rcMonitor, mi.rcWork, (mi.dwFlags & MONITORINFOF_PRIMARY) != 0, dpi);
    }

    /// <summary>True if the foreground window is covering an entire monitor other than <paramref name="ours"/>.</summary>
    public static bool IsFullscreenElsewhere(IntPtr ours)
    {
        IntPtr fg = GetForegroundWindow();
        if (fg == IntPtr.Zero || fg == GetShellWindow()) return false;
        IntPtr mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        if (mon == ours) return false;
        var info = Query(mon);
        if (info == null || !GetWindowRect(fg, out var r)) return false;
        return r.Left <= info.Bounds.Left && r.Top <= info.Bounds.Top && r.Right >= info.Bounds.Right && r.Bottom >= info.Bounds.Bottom;
    }
}
