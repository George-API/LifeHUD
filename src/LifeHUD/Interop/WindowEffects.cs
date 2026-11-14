using System;
using System.Runtime.InteropServices;
using System.Windows.Media;
using static LifeHUD.Interop.NativeMethods;

namespace LifeHUD.Interop;

internal static class WindowEffects
{
    private const int DWMWA_USE_IMMERSIVE_DARK_MODE = 20;
    private const int DWMWA_WINDOW_CORNER_PREFERENCE = 33;
    private const int DWMWA_BORDER_COLOR = 34;
    private const int DWMWA_SYSTEMBACKDROP_TYPE = 38;
    private const int DWMWCP_DONOTROUND = 1;
    private const int DWMSBT_TRANSIENTWINDOW = 3; // acrylic
    private const int WCA_ACCENT_POLICY = 19;
    private const int ACCENT_ENABLE_ACRYLICBLURBEHIND = 4;

    /// <summary>Keeps the HUD out of Alt+Tab and Task View.</summary>
    public static void HideFromAltTab(IntPtr hwnd)
    {
        long ex = GetWindowLongPtr(hwnd, GWL_EXSTYLE).ToInt64();
        ex |= WS_EX_TOOLWINDOW;
        ex &= ~WS_EX_APPWINDOW;
        SetWindowLongPtr(hwnd, GWL_EXSTYLE, new IntPtr(ex));
    }

    /// <summary>
    /// The HUD keeps a captioned frame for DWM composition; without the system menu DWM draws no caption
    /// buttons into the extended glass.
    /// </summary>
    public static void RemoveCaptionButtons(IntPtr hwnd)
    {
        long style = GetWindowLongPtr(hwnd, GWL_STYLE).ToInt64();
        style &= ~(WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
        SetWindowLongPtr(hwnd, GWL_STYLE, new IntPtr(style));
    }

    public static void SetDarkMode(IntPtr hwnd, bool dark)
    {
        int value = dark ? 1 : 0;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, ref value, sizeof(int));
    }

    /// <summary>Docked surfaces sit flush against the screen edge: square corners, no DWM outline.</summary>
    public static void MakeFlush(IntPtr hwnd)
    {
        int corner = DWMWCP_DONOTROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, ref corner, sizeof(int));
        int none = unchecked((int)0xFFFFFFFE); // DWMWA_COLOR_NONE
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, ref none, sizeof(int));
    }

    public static void ExtendFrame(IntPtr hwnd)
    {
        var margins = new MARGINS(-1);
        DwmExtendFrameIntoClientArea(hwnd, ref margins);
    }

    /// <summary>
    /// Acrylic blur that stays live while the window is inactive (an always-on HUD is almost never the
    /// active window, and DWM system backdrops fall back to a flat colour when inactive).
    /// </summary>
    public static bool TryEnableAcrylic(IntPtr hwnd, Color tint)
    {
        var accent = new AccentPolicy
        {
            AccentState = ACCENT_ENABLE_ACRYLICBLURBEHIND,
            AccentFlags = 2,
            GradientColor = ((uint)tint.A << 24) | ((uint)tint.B << 16) | ((uint)tint.G << 8) | tint.R,
        };
        int size = Marshal.SizeOf<AccentPolicy>();
        IntPtr ptr = Marshal.AllocHGlobal(size);
        try
        {
            Marshal.StructureToPtr(accent, ptr, false);
            var data = new WindowCompositionAttributeData { Attribute = WCA_ACCENT_POLICY, Data = ptr, SizeOfData = size };
            return SetWindowCompositionAttribute(hwnd, ref data) != 0;
        }
        catch
        {
            return false;
        }
        finally
        {
            Marshal.FreeHGlobal(ptr);
        }
    }

    public static bool TrySystemAcrylic(IntPtr hwnd)
    {
        if (Environment.OSVersion.Version.Build < 22621) return false;
        int type = DWMSBT_TRANSIENTWINDOW;
        return DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, ref type, sizeof(int)) == 0;
    }
}
