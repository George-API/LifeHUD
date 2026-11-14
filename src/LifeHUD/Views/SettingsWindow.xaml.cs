using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using LifeHUD.Services;
using LifeHUD.ViewModels;

namespace LifeHUD.Views;

public partial class SettingsWindow : Window
{
    private readonly ThemeService _theme;

    internal SettingsWindow(SettingsViewModel vm, ThemeService theme)
    {
        _theme = theme;
        ApplyTheme();
        InitializeComponent();
        DataContext = vm;
        _theme.ThemeChanged += OnThemeChanged;
    }

    private void OnThemeChanged(object? sender, EventArgs e) => ApplyTheme();

    /// <summary>WPF's built-in Fluent theme gives native Windows 11 controls and a Mica backdrop.</summary>
    private void ApplyTheme() => ThemeMode = _theme.IsDark ? ThemeMode.Dark : ThemeMode.Light;

    private void OnCityKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter) return;
        CityBox.GetBindingExpression(TextBox.TextProperty)?.UpdateSource();
        e.Handled = true;
    }

    protected override void OnClosed(EventArgs e)
    {
        _theme.ThemeChanged -= OnThemeChanged;
        base.OnClosed(e);
    }
}
