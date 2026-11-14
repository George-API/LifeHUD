using System.Windows;
using System.Windows.Controls;

namespace LifeHUD.Controls;

/// <summary>
/// Base class for every HUD widget. Each widget ships two purpose-built presentations and switches
/// between them on <see cref="IsVertical"/>; <see cref="IsCompact"/> lets it shed detail when space is tight.
/// </summary>
public class WidgetView : UserControl
{
    public static readonly DependencyProperty IsVerticalProperty =
        DependencyProperty.Register(nameof(IsVertical), typeof(bool), typeof(WidgetView), new PropertyMetadata(false));

    public static readonly DependencyProperty IsCompactProperty =
        DependencyProperty.Register(nameof(IsCompact), typeof(bool), typeof(WidgetView), new PropertyMetadata(false));

    public bool IsVertical
    {
        get => (bool)GetValue(IsVerticalProperty);
        set => SetValue(IsVerticalProperty, value);
    }

    public bool IsCompact
    {
        get => (bool)GetValue(IsCompactProperty);
        set => SetValue(IsCompactProperty, value);
    }
}
