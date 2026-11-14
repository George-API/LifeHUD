using System;
using LifeHUD.Models;
using Forms = System.Windows.Forms;

namespace LifeHUD.Services;

/// <summary>Notification-area icon: the always-available way back to a hidden HUD.</summary>
internal sealed class TrayIcon : IDisposable
{
    private readonly Forms.NotifyIcon _icon;
    private readonly SettingsService _settings;
    private readonly Forms.ToolStripMenuItem _toggle, _top, _side;

    public TrayIcon(SettingsService settings, Action<bool> setHudVisible, Action showSettings, Action exit)
    {
        _settings = settings;
        var menu = new Forms.ContextMenuStrip { ShowImageMargin = false };
        _toggle = new Forms.ToolStripMenuItem("Hide HUD", null, (_, _) => setHudVisible(!_settings.Current.HudVisible));
        _top = new Forms.ToolStripMenuItem("Top HUD", null, (_, _) => SetLayout(HudEdge.Top));
        _side = new Forms.ToolStripMenuItem("Life Sidebar", null, (_, _) => SetLayout(HudEdge.Left));
        menu.Items.Add(_toggle);
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add(_top);
        menu.Items.Add(_side);
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add(new Forms.ToolStripMenuItem("Settings…", null, (_, _) => showSettings()));
        menu.Items.Add(new Forms.ToolStripMenuItem("Quit Life HUD", null, (_, _) => exit()));
        menu.Opening += (_, _) => Sync();

        _icon = new Forms.NotifyIcon
        {
            Icon = LoadIcon(),
            Text = "Life HUD",
            ContextMenuStrip = menu,
            Visible = true,
        };
        _icon.MouseClick += (_, e) =>
        {
            if (e.Button == Forms.MouseButtons.Left)
            {
                if (_settings.Current.HudVisible) showSettings();
                else setHudVisible(true);
            }
        };
    }

    private void SetLayout(HudEdge edge)
    {
        if (_settings.Current.Layout != edge) _settings.Update(s => s.Layout = edge, SettingKeys.Layout);
    }

    private void Sync()
    {
        _toggle.Text = _settings.Current.HudVisible ? "Hide HUD" : "Show HUD";
        _top.Checked = _settings.Current.Layout == HudEdge.Top;
        _side.Checked = _settings.Current.Layout == HudEdge.Left;
    }

    private static System.Drawing.Icon LoadIcon()
    {
        try
        {
            var res = System.Windows.Application.GetResourceStream(new Uri("pack://application:,,,/Assets/LifeHUD.ico"));
            if (res != null)
            {
                using var stream = res.Stream;
                return new System.Drawing.Icon(stream, Forms.SystemInformation.SmallIconSize);
            }
        }
        catch (Exception ex)
        {
            Log.Write("Loading tray icon failed", ex);
        }
        return System.Drawing.SystemIcons.Application;
    }

    public void Dispose()
    {
        _icon.Visible = false;
        _icon.Dispose();
    }
}
