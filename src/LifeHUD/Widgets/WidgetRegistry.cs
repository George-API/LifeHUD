using System.Collections.Generic;
using System.Windows;
using LifeHUD.Controls;
using LifeHUD.ViewModels;

namespace LifeHUD.Widgets;

/// <summary>Where a widget lives in the Top HUD. The clock is the centrepiece; the two sides are mirrored halves.</summary>
public enum HorizontalSlot { Leading, Center, Trailing }

/// <summary>
/// Declares a widget and how each layout should place it. Adding a widget = one view + one line here.
/// </summary>
public sealed record WidgetDescriptor(
    string Id,
    WidgetView View,
    HorizontalSlot Slot,
    int HorizontalOrder,
    GridLength HorizontalSize,
    int VerticalOrder,
    GridLength VerticalSize);

public static class WidgetRegistry
{
    private static GridLength Star(double v) => new(v, GridUnitType.Star);

    public static IReadOnlyList<WidgetDescriptor> Create(HudViewModel vm) => new[]
    {
        //                     id         view                                        slot                     h#  h size          v#  v size
        new WidgetDescriptor("weather",  new WeatherWidget { DataContext = vm.Weather }, HorizontalSlot.Leading,  0, GridLength.Auto, 1, GridLength.Auto),
        new WidgetDescriptor("tasks",    new TasksWidget   { DataContext = vm.Tasks },   HorizontalSlot.Leading,  1, Star(1),         3, Star(3)),
        new WidgetDescriptor("clock",    new ClockWidget   { DataContext = vm.Clock },   HorizontalSlot.Center,   0, GridLength.Auto, 0, GridLength.Auto),
        new WidgetDescriptor("notes",    new NotesWidget   { DataContext = vm.Notes },   HorizontalSlot.Trailing, 0, Star(1),         4, Star(2)),
        new WidgetDescriptor("media",    new MediaWidget   { DataContext = vm.Media },   HorizontalSlot.Trailing, 1, GridLength.Auto, 2, GridLength.Auto),
        new WidgetDescriptor("controls", new HudControlsWidget { DataContext = vm },     HorizontalSlot.Trailing, 2, GridLength.Auto, 5, GridLength.Auto),
    };
}
