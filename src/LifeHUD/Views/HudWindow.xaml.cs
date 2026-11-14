using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Threading;
using LifeHUD.Interop;
using LifeHUD.Models;
using LifeHUD.Services;
using LifeHUD.ViewModels;
using LifeHUD.Widgets;
using Microsoft.Win32;

namespace LifeHUD.Views;

public partial class HudWindow : Window
{
    private const double Gap = 8;
    private const double MinTopDip = 80, MaxTopDip = 220;
    private const double MinSideDip = 248, MaxSideDip = 560;

    private readonly SettingsService _settings;
    private readonly ThemeService _theme;
    private readonly IReadOnlyList<WidgetDescriptor> _widgets;
    private readonly AppBar _appBar;
    private readonly DispatcherTimer _displayDebounce;
    private IntPtr _hwnd;
    private bool _acrylic;

    internal HudWindow(HudViewModel vm, SettingsService settings, ThemeService theme)
    {
        InitializeComponent();
        _settings = settings;
        _theme = theme;
        DataContext = vm;
        _widgets = WidgetRegistry.Create(vm);

        _appBar = new AppBar(GetDockRequest);
        _appBar.FullscreenChanged += OnFullscreenChanged;

        _displayDebounce = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(500) };
        _displayDebounce.Tick += (_, _) => { _displayDebounce.Stop(); if (IsVisible) ResetAppBar(); };

        SourceInitialized += OnSourceInitialized;
        SizeChanged += (_, _) => UpdateDensity();
        _settings.Changed += OnSettingsChanged;
        _theme.ThemeChanged += (_, _) => ApplyWindowTheme();
        SystemEvents.DisplaySettingsChanged += OnDisplaySettingsChanged;
        DpiChanged += (_, _) => Dispatcher.BeginInvoke(DispatcherPriority.Background, new Action(_appBar.Reposition));

        ApplyLayout();
    }

    public IntPtr Handle => _hwnd;

    public void ShowHud()
    {
        new WindowInteropHelper(this).EnsureHandle();
        _appBar.Register();
        _appBar.Reposition();
        Show();
        _appBar.Reposition();
    }

    public void HideHud()
    {
        _appBar.Unregister();
        Hide();
    }

    private void OnSourceInitialized(object? sender, EventArgs e)
    {
        _hwnd = new WindowInteropHelper(this).Handle;
        if (HwndSource.FromHwnd(_hwnd) is HwndSource src && src.CompositionTarget is not null)
            src.CompositionTarget.BackgroundColor = Colors.Transparent;

        WindowEffects.HideFromAltTab(_hwnd);
        WindowEffects.RemoveCaptionButtons(_hwnd);
        WindowEffects.MakeFlush(_hwnd);
        WindowEffects.ExtendFrame(_hwnd);
        _appBar.Attach(_hwnd);
        ApplyWindowTheme();
    }

    private void ApplyWindowTheme()
    {
        if (_hwnd == IntPtr.Zero) return;
        WindowEffects.SetDarkMode(_hwnd, _theme.IsDark);
        _acrylic = WindowEffects.TryEnableAcrylic(_hwnd, _theme.HudAcrylicTint) || WindowEffects.TrySystemAcrylic(_hwnd);
        Log.Write($"HUD backdrop: {(_acrylic ? "acrylic" : "solid fallback")}, theme {(_theme.IsDark ? "dark" : "light")}");
        _theme.SetHudTranslucent(_acrylic); // re-applies brushes only if this changed
    }

    // ------------------------------------------------------------------ layout

    private void ApplyLayout()
    {
        bool vertical = _settings.Current.Layout == HudEdge.Left;

        foreach (var w in _widgets)
        {
            if (w.View.Parent is Panel p) p.Children.Remove(w.View);
            w.View.IsVertical = vertical;
            w.View.Margin = new Thickness(0);
        }
        Host.Children.Clear();
        Host.RowDefinitions.Clear();
        Host.ColumnDefinitions.Clear();

        if (!vertical)
        {
            // [ leading widgets ][ clock ][ trailing widgets ] with equal-width halves, so the clock
            // sits at the true centre of the screen.
            EdgeLine.BorderThickness = new Thickness(0, 0, 0, 1);
            Host.Margin = new Thickness(Gap);
            Host.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            Host.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            Host.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

            var leading = BuildStrip(HorizontalSlot.Leading);
            var trailing = BuildStrip(HorizontalSlot.Trailing);
            Grid.SetColumn(leading, 0);
            Grid.SetColumn(trailing, 2);
            Host.Children.Add(leading);
            Host.Children.Add(trailing);

            foreach (var w in _widgets.Where(w => w.Slot == HorizontalSlot.Center))
            {
                Grid.SetColumn(w.View, 1);
                Grid.SetRow(w.View, 0);
                Host.Children.Add(w.View);
            }
        }
        else
        {
            EdgeLine.BorderThickness = new Thickness(0, 0, 1, 0);
            Host.Margin = new Thickness(Gap, Gap + 4, Gap, Gap);
            int row = 0;
            var ordered = _widgets.OrderBy(w => w.VerticalOrder).ToList();
            foreach (var w in ordered)
            {
                Host.RowDefinitions.Add(new RowDefinition { Height = w.VerticalSize });
                Grid.SetRow(w.View, row);
                Grid.SetColumn(w.View, 0);
                w.View.Margin = new Thickness(0, 0, 0, row == ordered.Count - 1 ? 0 : Gap);
                Host.Children.Add(w.View);
                row++;
            }
        }
        UpdateDensity();
    }

    private Grid BuildStrip(HorizontalSlot slot)
    {
        var strip = new Grid();
        int col = 0;
        foreach (var w in _widgets.Where(w => w.Slot == slot).OrderBy(w => w.HorizontalOrder))
        {
            strip.ColumnDefinitions.Add(new ColumnDefinition { Width = w.HorizontalSize });
            Grid.SetColumn(w.View, col);
            Grid.SetRow(w.View, 0);
            w.View.Margin = new Thickness(col == 0 ? 0 : Gap, 0, 0, 0);
            strip.Children.Add(w.View);
            col++;
        }
        return strip;
    }

    /// <summary>Widgets shed secondary detail (mini forecast, hourly strip) when space is tight.</summary>
    private void UpdateDensity()
    {
        bool vertical = _settings.Current.Layout == HudEdge.Left;
        bool compact = vertical ? ActualHeight > 0 && ActualHeight < 900 : ActualWidth > 0 && ActualWidth < 1560;
        foreach (var w in _widgets) w.View.IsCompact = compact;
    }

    // ------------------------------------------------------------------ appbar

    private AppBar.DockRequest GetDockRequest()
    {
        var s = _settings.Current;
        var mon = MonitorService.Find(s.MonitorDevice);
        int thickness;
        if (s.Layout == HudEdge.Top)
        {
            double px = mon.Bounds.Height * s.TopSizePercent / 100.0;
            thickness = (int)Math.Round(Math.Clamp(px, MinTopDip * mon.Scale, Math.Max(MinTopDip, MaxTopDip) * mon.Scale));
        }
        else
        {
            double px = mon.Bounds.Width * s.SideSizePercent / 100.0;
            thickness = (int)Math.Round(Math.Clamp(px, MinSideDip * mon.Scale, MaxSideDip * mon.Scale));
        }
        return new AppBar.DockRequest(s.Layout, mon.Handle, mon.Bounds, mon.WorkArea, thickness);
    }

    /// <summary>
    /// Changing edge: release the old reservation, let the shell recompute the work area, then claim the new one.
    /// </summary>
    private void ResetAppBar()
    {
        _appBar.Unregister();
        Dispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() =>
        {
            if (!IsVisible) return;
            _appBar.Register();
            _appBar.Reposition();
            // A second pass picks up any work-area change the shell applied asynchronously.
            var settle = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(400) };
            settle.Tick += (_, _) => { settle.Stop(); _appBar.Reposition(); };
            settle.Start();
        }));
    }

    private void OnSettingsChanged(object? sender, string key)
    {
        switch (key)
        {
            case SettingKeys.Layout:
                ApplyLayout();
                if (IsVisible) ResetAppBar();
                break;
            case SettingKeys.Monitor:
                if (IsVisible) ResetAppBar();
                break;
            case SettingKeys.Size:
                if (IsVisible) _appBar.Reposition();
                break;
        }
    }

    private void OnDisplaySettingsChanged(object? sender, EventArgs e) =>
        Dispatcher.BeginInvoke(new Action(() => { _displayDebounce.Stop(); _displayDebounce.Start(); }));

    private void OnFullscreenChanged(object? sender, bool fullscreen)
    {
        if (fullscreen)
        {
            // Games / videos / presentations get the whole screen; ignore fullscreen apps on other monitors.
            if (MonitorService.IsFullscreenElsewhere(_appBar.Monitor)) return;
            Topmost = false;
            NativeMethods.SetWindowPos(_hwnd, NativeMethods.HWND_BOTTOM, 0, 0, 0, 0,
                NativeMethods.SWP_NOMOVE | NativeMethods.SWP_NOSIZE | NativeMethods.SWP_NOACTIVATE);
        }
        else
        {
            Topmost = true;
        }
    }

    protected override void OnClosed(EventArgs e)
    {
        SystemEvents.DisplaySettingsChanged -= OnDisplaySettingsChanged;
        _appBar.Dispose();
        base.OnClosed(e);
    }
}
