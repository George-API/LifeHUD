using System;
using System.Runtime.InteropServices;
using System.Windows.Interop;
using System.Windows.Threading;
using LifeHUD.Models;
using static LifeHUD.Interop.NativeMethods;

namespace LifeHUD.Interop;

/// <summary>
/// Registers a window as a shell AppBar so Windows shrinks the work area around it.
/// Maximized windows then fill the remaining desktop instead of sliding underneath the HUD.
/// All coordinates are physical pixels.
/// </summary>
internal sealed class AppBar : IDisposable
{
    public readonly record struct DockRequest(HudEdge Edge, IntPtr Monitor, RECT MonitorBounds, RECT WorkArea, int Thickness);

    private readonly Func<DockRequest> _request;
    private readonly uint _callbackMessage = RegisterWindowMessage("LifeHUD.AppBarNotify");
    private readonly uint _taskbarCreatedMessage = RegisterWindowMessage("TaskbarCreated");
    private IntPtr _hwnd;
    private HwndSource? _source;
    private bool _positioning;
    private RECT? _bounds;

    public AppBar(Func<DockRequest> request) => _request = request;

    public bool IsRegistered { get; private set; }
    public IntPtr Monitor { get; private set; }
    public IntPtr Handle => _hwnd;

    /// <summary>Raised with true when a fullscreen app opens on the shell, false when it closes.</summary>
    public event EventHandler<bool>? FullscreenChanged;

    public void Attach(IntPtr hwnd)
    {
        _hwnd = hwnd;
        _source = HwndSource.FromHwnd(hwnd);
        _source?.AddHook(WndProc);
    }

    public void Register()
    {
        if (IsRegistered || _hwnd == IntPtr.Zero) return;
        var abd = NewData();
        abd.uCallbackMessage = _callbackMessage;
        IsRegistered = SHAppBarMessage(ABM_NEW, ref abd) != UIntPtr.Zero;
        if (!IsRegistered) Services.Log.Write("ABM_NEW failed");
    }

    public void Unregister()
    {
        if (!IsRegistered) return;
        var abd = NewData();
        SHAppBarMessage(ABM_REMOVE, ref abd);
        IsRegistered = false;
        _bounds = null;
    }

    /// <summary>Asks the shell for space on the requested edge, claims it, and moves the window into it.</summary>
    public void Reposition()
    {
        if (!IsRegistered) return;

        var req = _request();
        Monitor = req.Monitor;

        var abd = NewData();
        if (req.Edge == HudEdge.Top)
        {
            abd.uEdge = ABE_TOP;
            abd.rc = new RECT
            {
                Left = req.WorkArea.Left,
                Right = req.WorkArea.Right,
                Top = req.MonitorBounds.Top,
                Bottom = req.MonitorBounds.Top + req.Thickness,
            };
        }
        else
        {
            abd.uEdge = ABE_LEFT;
            abd.rc = new RECT
            {
                Left = req.MonitorBounds.Left,
                Right = req.MonitorBounds.Left + req.Thickness,
                Top = req.WorkArea.Top,
                Bottom = req.WorkArea.Bottom,
            };
        }

        // The shell may nudge us (e.g. a taskbar or another appbar already on that edge).
        SHAppBarMessage(ABM_QUERYPOS, ref abd);
        if (req.Edge == HudEdge.Top) abd.rc.Bottom = abd.rc.Top + req.Thickness;
        else abd.rc.Right = abd.rc.Left + req.Thickness;
        SHAppBarMessage(ABM_SETPOS, ref abd);

        _bounds = abd.rc;
        _positioning = true;
        try
        {
            SetWindowPos(_hwnd, IntPtr.Zero, abd.rc.Left, abd.rc.Top, abd.rc.Width, abd.rc.Height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
        finally
        {
            _positioning = false;
        }
    }

    private IntPtr WndProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == (int)_callbackMessage)
        {
            switch (wParam.ToInt32())
            {
                case ABN_POSCHANGED:
                    // Taskbar or another appbar moved; reclaim our slot.
                    _source?.Dispatcher.BeginInvoke(DispatcherPriority.Background, new Action(Reposition));
                    break;
                case ABN_FULLSCREENAPP:
                    FullscreenChanged?.Invoke(this, lParam != IntPtr.Zero);
                    break;
            }
            handled = true;
            return IntPtr.Zero;
        }

        if (msg == (int)_taskbarCreatedMessage)
        {
            // Explorer restarted: all appbar registrations were lost.
            if (IsRegistered)
            {
                IsRegistered = false;
                _source?.Dispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() => { Register(); Reposition(); }));
            }
            return IntPtr.Zero;
        }

        switch (msg)
        {
            case WM_ACTIVATE when IsRegistered:
            {
                var abd = NewData();
                SHAppBarMessage(ABM_ACTIVATE, ref abd);
                break;
            }
            case WM_WINDOWPOSCHANGED when IsRegistered:
            {
                var abd = NewData();
                SHAppBarMessage(ABM_WINDOWPOSCHANGED, ref abd);
                break;
            }
            case WM_WINDOWPOSCHANGING when IsRegistered && !_positioning && _bounds is RECT b:
            {
                // Pin the window to its reserved slot (blocks Win+Arrow, DPI auto-resize, etc.).
                var wp = Marshal.PtrToStructure<WINDOWPOS>(lParam);
                const uint moveOrSize = SWP_NOMOVE | SWP_NOSIZE;
                if ((wp.flags & moveOrSize) != moveOrSize)
                {
                    wp.x = b.Left; wp.y = b.Top; wp.cx = b.Width; wp.cy = b.Height;
                    wp.flags &= ~moveOrSize;
                    Marshal.StructureToPtr(wp, lParam, false);
                }
                break;
            }
        }
        return IntPtr.Zero;
    }

    private APPBARDATA NewData() => new()
    {
        cbSize = (uint)Marshal.SizeOf<APPBARDATA>(),
        hWnd = _hwnd,
    };

    /// <summary>Last-chance removal from a crash handler (any thread) so the reserved space is released.</summary>
    public static void EmergencyRemove(IntPtr hwnd)
    {
        if (hwnd == IntPtr.Zero) return;
        try
        {
            var abd = new APPBARDATA { cbSize = (uint)Marshal.SizeOf<APPBARDATA>(), hWnd = hwnd };
            SHAppBarMessage(ABM_REMOVE, ref abd);
        }
        catch { /* best effort */ }
    }

    public void Dispose()
    {
        Unregister();
        _source?.RemoveHook(WndProc);
        _source = null;
    }
}
