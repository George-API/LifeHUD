using System;
using System.Windows;
using System.Windows.Media;

namespace LifeHUD.Controls;

/// <summary>
/// A day's low→high temperature span drawn against the whole forecast's range, coloured by
/// temperature, with an optional "now" marker.
/// </summary>
public sealed class RangeBar : FrameworkElement
{
    private static FrameworkPropertyMetadata Meta(object def) => new(def, FrameworkPropertyMetadataOptions.AffectsRender);

    public static readonly DependencyProperty RangeMinProperty = DependencyProperty.Register(nameof(RangeMin), typeof(double), typeof(RangeBar), Meta(0.0));
    public static readonly DependencyProperty RangeMaxProperty = DependencyProperty.Register(nameof(RangeMax), typeof(double), typeof(RangeBar), Meta(1.0));
    public static readonly DependencyProperty LowProperty = DependencyProperty.Register(nameof(Low), typeof(double), typeof(RangeBar), Meta(0.0));
    public static readonly DependencyProperty HighProperty = DependencyProperty.Register(nameof(High), typeof(double), typeof(RangeBar), Meta(1.0));
    public static readonly DependencyProperty MarkerProperty = DependencyProperty.Register(nameof(Marker), typeof(double), typeof(RangeBar), Meta(double.NaN));
    public static readonly DependencyProperty CelsiusProperty = DependencyProperty.Register(nameof(Celsius), typeof(bool), typeof(RangeBar), Meta(true));
    public static readonly DependencyProperty TrackBrushProperty = DependencyProperty.Register(nameof(TrackBrush), typeof(Brush), typeof(RangeBar), Meta(Brushes.Gray));

    public double RangeMin { get => (double)GetValue(RangeMinProperty); set => SetValue(RangeMinProperty, value); }
    public double RangeMax { get => (double)GetValue(RangeMaxProperty); set => SetValue(RangeMaxProperty, value); }
    public double Low { get => (double)GetValue(LowProperty); set => SetValue(LowProperty, value); }
    public double High { get => (double)GetValue(HighProperty); set => SetValue(HighProperty, value); }
    public double Marker { get => (double)GetValue(MarkerProperty); set => SetValue(MarkerProperty, value); }
    public bool Celsius { get => (bool)GetValue(CelsiusProperty); set => SetValue(CelsiusProperty, value); }
    public Brush TrackBrush { get => (Brush)GetValue(TrackBrushProperty); set => SetValue(TrackBrushProperty, value); }

    private static readonly (double t, Color c)[] Scale =
    {
        (-10, Color.FromRgb(0x5E, 0x7B, 0xFF)),
        (0, Color.FromRgb(0x5A, 0xC8, 0xFA)),
        (10, Color.FromRgb(0x5F, 0xD6, 0x9A)),
        (18, Color.FromRgb(0xFF, 0xD6, 0x0A)),
        (25, Color.FromRgb(0xFF, 0x9F, 0x0A)),
        (33, Color.FromRgb(0xFF, 0x45, 0x3A)),
    };

    protected override void OnRender(DrawingContext dc)
    {
        double w = ActualWidth, h = Math.Min(ActualHeight, 4);
        if (w <= 0 || h <= 0) return;
        double y = (ActualHeight - h) / 2, radius = h / 2;
        dc.DrawRoundedRectangle(TrackBrush, null, new Rect(0, y, w, h), radius, radius);

        double span = Math.Max(0.1, RangeMax - RangeMin);
        double X(double v) => Math.Clamp((v - RangeMin) / span, 0, 1) * w;
        double x1 = X(Low), x2 = X(High);
        if (x2 - x1 < h) { double mid = (x1 + x2) / 2; x1 = Math.Max(0, mid - h / 2); x2 = Math.Min(w, x1 + h); }

        var brush = new LinearGradientBrush { MappingMode = BrushMappingMode.Absolute, StartPoint = new Point(0, 0), EndPoint = new Point(w, 0) };
        for (int i = 0; i <= 5; i++)
        {
            double temp = RangeMin + span * i / 5;
            brush.GradientStops.Add(new GradientStop(ColorAt(Celsius ? temp : (temp - 32) * 5 / 9), i / 5.0));
        }
        brush.Freeze();
        dc.DrawRoundedRectangle(brush, null, new Rect(x1, y, x2 - x1, h), radius, radius);

        if (!double.IsNaN(Marker))
        {
            double mx = Math.Clamp(X(Marker), x1 + 2, x2 - 2);
            dc.DrawEllipse(Brushes.White, new Pen(TrackBrush, 1.5), new Point(mx, y + h / 2), 3.2, 3.2);
        }
    }

    private static Color ColorAt(double c)
    {
        if (c <= Scale[0].t) return Scale[0].c;
        for (int i = 1; i < Scale.Length; i++)
        {
            if (c <= Scale[i].t)
            {
                var (t0, c0) = Scale[i - 1];
                var (t1, c1) = Scale[i];
                double f = (c - t0) / (t1 - t0);
                return Color.FromRgb((byte)(c0.R + (c1.R - c0.R) * f), (byte)(c0.G + (c1.G - c0.G) * f), (byte)(c0.B + (c1.B - c0.B) * f));
            }
        }
        return Scale[^1].c;
    }
}
