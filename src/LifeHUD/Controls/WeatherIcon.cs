using System;
using System.Windows;
using System.Windows.Media;
using LifeHUD.Models;

namespace LifeHUD.Controls;

/// <summary>
/// Vector weather glyphs drawn in a 100×100 design space. Crisp at any size and DPI, themable,
/// and colourful without relying on colour-emoji fonts (which WPF can't render).
/// </summary>
public sealed class WeatherIcon : FrameworkElement
{
    public static readonly DependencyProperty ConditionProperty = DependencyProperty.Register(
        nameof(Condition), typeof(WeatherCondition), typeof(WeatherIcon),
        new FrameworkPropertyMetadata(WeatherCondition.Clear, FrameworkPropertyMetadataOptions.AffectsRender));

    public static readonly DependencyProperty IsDayProperty = DependencyProperty.Register(
        nameof(IsDay), typeof(bool), typeof(WeatherIcon),
        new FrameworkPropertyMetadata(true, FrameworkPropertyMetadataOptions.AffectsRender));

    public static readonly DependencyProperty CloudBrushProperty = DependencyProperty.Register(
        nameof(CloudBrush), typeof(Brush), typeof(WeatherIcon),
        new FrameworkPropertyMetadata(Brushes.WhiteSmoke, FrameworkPropertyMetadataOptions.AffectsRender));

    public WeatherCondition Condition { get => (WeatherCondition)GetValue(ConditionProperty); set => SetValue(ConditionProperty, value); }
    public bool IsDay { get => (bool)GetValue(IsDayProperty); set => SetValue(IsDayProperty, value); }
    public Brush CloudBrush { get => (Brush)GetValue(CloudBrushProperty); set => SetValue(CloudBrushProperty, value); }

    private static readonly Brush SunBrush = Frozen(new RadialGradientBrush(Color.FromRgb(0xFF, 0xE0, 0x6B), Color.FromRgb(0xFF, 0xB2, 0x1F)));
    private static readonly Pen RayPen = FrozenPen(new SolidColorBrush(Color.FromRgb(0xFF, 0xC1, 0x2E)), 5.5);
    private static readonly Brush MoonBrush = Frozen(new LinearGradientBrush(Color.FromRgb(0xF4, 0xF1, 0xE4), Color.FromRgb(0xC9, 0xC4, 0xB0), 45));
    private static readonly Pen RainPen = FrozenPen(new SolidColorBrush(Color.FromRgb(0x4F, 0xB7, 0xFF)), 5.5);
    private static readonly Pen DrizzlePen = FrozenPen(new SolidColorBrush(Color.FromRgb(0x7C, 0xC8, 0xFF)), 4.5);
    private static readonly Brush SnowBrush = Frozen(new SolidColorBrush(Color.FromRgb(0xA8, 0xDD, 0xFF)));
    private static readonly Brush BoltBrush = Frozen(new LinearGradientBrush(Color.FromRgb(0xFF, 0xE0, 0x4D), Color.FromRgb(0xFF, 0xA7, 0x26), 90));
    private static readonly Pen FogPen = FrozenPen(new SolidColorBrush(Color.FromArgb(0xB0, 0x9E, 0xA9, 0xBA)), 5);

    private static readonly Geometry CloudShape = BuildCloud();

    protected override void OnRender(DrawingContext dc)
    {
        double size = Math.Min(ActualWidth, ActualHeight);
        if (size <= 0) return;
        dc.PushTransform(new TranslateTransform((ActualWidth - size) / 2, (ActualHeight - size) / 2));
        dc.PushTransform(new ScaleTransform(size / 100, size / 100));

        switch (Condition)
        {
            case WeatherCondition.Clear:
                if (IsDay) DrawSun(dc, 50, 50, 21); else DrawMoon(dc, 50, 50, 26);
                break;
            case WeatherCondition.PartlyCloudy:
                if (IsDay) DrawSun(dc, 38, 36, 15); else DrawMoon(dc, 38, 36, 18);
                DrawCloud(dc, 58, 62, 0.78);
                break;
            case WeatherCondition.Cloudy:
                dc.PushOpacity(0.55);
                DrawCloud(dc, 62, 40, 0.62);
                dc.Pop();
                DrawCloud(dc, 48, 58, 0.86);
                break;
            case WeatherCondition.Fog:
                DrawCloud(dc, 50, 42, 0.82);
                dc.DrawLine(FogPen, new Point(22, 74), new Point(70, 74));
                dc.DrawLine(FogPen, new Point(32, 86), new Point(80, 86));
                break;
            case WeatherCondition.Drizzle:
                DrawCloud(dc, 50, 40, 0.86);
                foreach (var x in new[] { 34.0, 52, 70 }) dc.DrawLine(DrizzlePen, new Point(x, 72), new Point(x - 2, 80));
                break;
            case WeatherCondition.Rain:
                DrawCloud(dc, 50, 40, 0.86);
                foreach (var x in new[] { 34.0, 52, 70 }) dc.DrawLine(RainPen, new Point(x, 70), new Point(x - 5, 88));
                break;
            case WeatherCondition.Snow:
                DrawCloud(dc, 50, 40, 0.86);
                foreach (var (x, y) in new[] { (32.0, 76.0), (50.0, 84.0), (68.0, 76.0), (41.0, 92.0), (59.0, 92.0) })
                    dc.DrawEllipse(SnowBrush, null, new Point(x, y), 4, 4);
                break;
            case WeatherCondition.Thunder:
                DrawCloud(dc, 50, 38, 0.86);
                var bolt = Geometry.Parse("M 54,62 L 40,82 L 50,82 L 45,98 L 64,74 L 53,74 L 60,62 Z");
                dc.DrawGeometry(BoltBrush, null, bolt);
                break;
        }
        dc.Pop();
        dc.Pop();
    }

    private static void DrawSun(DrawingContext dc, double cx, double cy, double r)
    {
        for (int i = 0; i < 8; i++)
        {
            double a = i * Math.PI / 4;
            var p1 = new Point(cx + Math.Cos(a) * (r + 7), cy + Math.Sin(a) * (r + 7));
            var p2 = new Point(cx + Math.Cos(a) * (r + 15), cy + Math.Sin(a) * (r + 15));
            dc.DrawLine(RayPen, p1, p2);
        }
        dc.DrawEllipse(SunBrush, null, new Point(cx, cy), r, r);
    }

    private static void DrawMoon(DrawingContext dc, double cx, double cy, double r)
    {
        var moon = new CombinedGeometry(GeometryCombineMode.Exclude,
            new EllipseGeometry(new Point(cx, cy), r, r),
            new EllipseGeometry(new Point(cx + r * 0.55, cy - r * 0.38), r * 0.82, r * 0.82));
        dc.DrawGeometry(MoonBrush, null, moon);
    }

    private void DrawCloud(DrawingContext dc, double cx, double cy, double scale)
    {
        // Base cloud is centred on (50,47).
        var t = new TransformGroup();
        t.Children.Add(new TranslateTransform(-50, -47));
        t.Children.Add(new ScaleTransform(scale, scale));
        t.Children.Add(new TranslateTransform(cx, cy));
        dc.PushTransform(t);
        dc.DrawGeometry(CloudBrush, null, CloudShape);
        dc.Pop();
    }

    private static Geometry BuildCloud()
    {
        // Overlapping circles + a rounded base, filled as a union (non-zero fill, no stroke).
        var g = new GeometryGroup { FillRule = FillRule.Nonzero };
        g.Children.Add(new EllipseGeometry(new Point(38, 52), 17, 17));
        g.Children.Add(new EllipseGeometry(new Point(57, 41), 22, 22));
        g.Children.Add(new EllipseGeometry(new Point(74, 55), 14, 14));
        g.Children.Add(new RectangleGeometry(new Rect(21, 50, 67, 19), 9.5, 9.5));
        g.Freeze();
        return g;
    }

    private static Brush Frozen(Brush b) { b.Freeze(); return b; }

    private static Pen FrozenPen(Brush b, double thickness)
    {
        var p = new Pen(b, thickness) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round };
        p.Freeze();
        return p;
    }
}
