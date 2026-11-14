using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using LifeHUD.Controls;

namespace LifeHUD.Widgets;

public partial class ClockWidget : WidgetView
{
    public ClockWidget() => InitializeComponent();
}

public partial class WeatherWidget : WidgetView
{
    public WeatherWidget() => InitializeComponent();
}

public partial class MediaWidget : WidgetView
{
    public MediaWidget() => InitializeComponent();
}

public partial class NotesWidget : WidgetView
{
    public NotesWidget() => InitializeComponent();
}

public partial class HudControlsWidget : WidgetView
{
    public HudControlsWidget() => InitializeComponent();
}

public partial class TasksWidget : WidgetView
{
    public TasksWidget() => InitializeComponent();

    /// <summary>Escape clears the quick-entry box and hands focus back.</summary>
    private void OnAddBoxKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Escape || sender is not TextBox box) return;
        box.Clear();
        Keyboard.ClearFocus();
        e.Handled = true;
    }
}
