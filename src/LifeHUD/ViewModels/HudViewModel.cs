using System;
using System.Windows.Input;
using LifeHUD.Models;
using LifeHUD.Services;

namespace LifeHUD.ViewModels;

public sealed class HudViewModel : ObservableObject
{
    private readonly SettingsService _settings;

    public HudViewModel(SettingsService settings, ClockViewModel clock, WeatherViewModel weather,
        TasksViewModel tasks, NotesViewModel notes, MediaViewModel media,
        Action openSettings, Action hide)
    {
        _settings = settings;
        Clock = clock;
        Weather = weather;
        Tasks = tasks;
        Notes = notes;
        Media = media;

        OpenSettingsCommand = new RelayCommand(openSettings);
        HideCommand = new RelayCommand(hide);
        ToggleLayoutCommand = new RelayCommand(() => _settings.Update(
            s => s.Layout = s.Layout == HudEdge.Top ? HudEdge.Left : HudEdge.Top, SettingKeys.Layout));

        _settings.Changed += (_, key) => { if (key == SettingKeys.Layout) Raise(nameof(IsVertical), nameof(LayoutToggleTip)); };
    }

    public ClockViewModel Clock { get; }
    public WeatherViewModel Weather { get; }
    public TasksViewModel Tasks { get; }
    public NotesViewModel Notes { get; }
    public MediaViewModel Media { get; }

    public ICommand OpenSettingsCommand { get; }
    public ICommand ToggleLayoutCommand { get; }
    public ICommand HideCommand { get; }

    public bool IsVertical => _settings.Current.Layout == HudEdge.Left;
    public string LayoutToggleTip => IsVertical ? "Switch to Top HUD" : "Switch to Life Sidebar";
}
