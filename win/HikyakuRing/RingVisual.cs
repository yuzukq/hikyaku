// リングとノブの表示。角度は 0° = 上・時計回り（hikyaku から届く角度と同じ）。Mac 版: RingOverlayView.swift

using System;
using System.Globalization;
using System.Windows;
using System.Windows.Media;
using Hikyaku.RingCore;

namespace Hikyaku.Ring;

public sealed class RingVisual : FrameworkElement
{
    private static readonly FontFamily IconFont = new("Segoe Fluent Icons, Segoe MDL2 Assets");
    private static readonly FontFamily TextFont = new("Yu Gothic UI, Segoe UI");
    private static readonly Brush Panel = Frozen(new SolidColorBrush(Color.FromArgb(0xE8, 0x1E, 0x1E, 0x22)));
    private static readonly Brush Accent = Frozen(new SolidColorBrush(Color.FromRgb(0x2F, 0x7C, 0xF6)));
    private static readonly Brush Primary = Brushes.White;
    private static readonly Brush Secondary = Frozen(new SolidColorBrush(Color.FromArgb(0xB0, 0xFF, 0xFF, 0xFF)));
    private static readonly Brush Tertiary = Frozen(new SolidColorBrush(Color.FromArgb(0x60, 0xFF, 0xFF, 0xFF)));

    /// <summary>リングの外周の半径（パッドの縁 = 距離 1000 に当たる）</summary>
    private const double Outer = 180;

    private readonly RingController _controller;

    public RingVisual(RingController controller)
    {
        _controller = controller;
        controller.Changed += InvalidateVisual;
    }

    private static Brush Frozen(Brush b)
    {
        b.Freeze();
        return b;
    }

    private static Brush White(double opacity) => Frozen(new SolidColorBrush(Color.FromArgb((byte)(opacity * 255), 0xFF, 0xFF, 0xFF)));

    protected override void OnRender(DrawingContext dc)
    {
        var c = new Point(OverlayWindow.Size / 2, OverlayWindow.Size / 2);
        dc.DrawEllipse(Panel, new Pen(White(0.18), 1), c, Outer, Outer);
        var mode = _controller.Mode;
        if (mode.Kind == RingModeKind.Knob && mode.Item is { } item)
        {
            DrawKnob(dc, c, _controller.Config.Items[item]);
        }
        else
        {
            DrawSelection(dc, c, mode.Kind == RingModeKind.Selecting ? mode.Item : null);
        }
        DrawFinger(dc, c);
    }

    private double CancelZone => Outer * _controller.Config.Tuning.CancelRadius / 1000;

    private void DrawSelection(DrawingContext dc, Point c, int? hover)
    {
        var f = _controller.Finger;
        var inCancel = f is { Touching: true } s && s.Radius < _controller.Config.Tuning.CancelRadius;
        dc.DrawEllipse(White(inCancel ? 0.22 : 0.06), null, c, CancelZone, CancelZone);
        DrawText(dc, "", IconFont, 16, inCancel ? Primary : Tertiary, c);

        for (var i = 0; i < RingConfig.SlotCount; i++)
        {
            var it = _controller.Config.Items[i];
            var highlighted = hover == i;
            var p = Offset(c, i * 45, Outer * 0.68);
            dc.PushOpacity(it.Action.Kind == ActionKind.None ? 0.35 : 1);
            var r = highlighted ? 24 : 21;
            dc.DrawEllipse(highlighted ? Accent : White(0.08), null, p, r, r);
            DrawText(dc, it.Glyph, IconFont, highlighted ? 22 : 20, Primary, p);
            DrawText(dc, it.Name, TextFont, 12, highlighted ? Primary : Secondary, new Point(p.X, p.Y + r + 10), 96);
            dc.Pop();
        }
    }

    private void DrawKnob(DrawingContext dc, Point c, RingItem item)
    {
        var tuning = _controller.Config.Tuning;
        var radius = Outer * 0.85;
        dc.DrawEllipse(null, new Pen(White(0.1), 6), c, radius, radius);
        // 今の値の円弧。1 段階に満たない回転の分も足して、指に合わせて動かす
        var readout = _controller.KnobReadout;
        if (readout?.Fraction(_controller.KnobRemainder / tuning.KnobStepDegrees) is { } fraction && fraction > 0)
        {
            DrawArc(dc, c, radius, 0, fraction * 360, new Pen(Accent, 6) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round });
        }
        // 1 段階に満たない回転を、指の位置から伸びる短い円弧で見せる
        var rem = _controller.KnobRemainder;
        if (_controller.Finger is { Touching: true } f && f.Radius >= tuning.CancelRadius && rem != 0)
        {
            var start = f.Angle - rem;
            DrawArc(dc, c, Outer * 0.96, Math.Min(start, f.Angle), Math.Abs(rem), new Pen(White(0.5), 3) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round });
        }
        DrawText(dc, item.Glyph, IconFont, 30, Primary, new Point(c.X, c.Y - 34));
        DrawText(dc, item.Name, TextFont, 16, Primary, new Point(c.X, c.Y + 4), 220, FontWeights.SemiBold);
        if (readout is not null) DrawText(dc, readout.Text, TextFont, 18, Secondary, new Point(c.X, c.Y + 32), 260);
        DrawText(dc, "回して調整・中心をタップで終了", TextFont, 11, Tertiary, new Point(c.X, c.Y + Outer * 0.62), 260);
    }

    private void DrawFinger(DrawingContext dc, Point c)
    {
        if (_controller.Finger is not { Touching: true } f) return;
        var p = Offset(c, f.Angle, Outer * Math.Min(f.Radius / 1000.0, 1.02));
        dc.DrawEllipse(Brushes.White, new Pen(White(0.3), 1), p, 5, 5);
    }

    /// <summary>startDeg から時計回りに sweepDeg の円弧（0° = 上）。</summary>
    private static void DrawArc(DrawingContext dc, Point c, double radius, double startDeg, double sweepDeg, Pen pen)
    {
        if (sweepDeg >= 359.9)
        {
            dc.DrawEllipse(null, pen, c, radius, radius);
            return;
        }
        var geometry = new StreamGeometry();
        using (var g = geometry.Open())
        {
            g.BeginFigure(Offset(c, startDeg, radius), false, false);
            g.ArcTo(Offset(c, startDeg + sweepDeg, radius), new Size(radius, radius), 0, sweepDeg > 180, SweepDirection.Clockwise, true, false);
        }
        geometry.Freeze();
        dc.DrawGeometry(null, pen, geometry);
    }

    private void DrawText(DrawingContext dc, string text, FontFamily font, double size, Brush brush, Point center,
        double maxWidth = 0, FontWeight? weight = null)
    {
        if (string.IsNullOrEmpty(text)) return;
        var ft = new FormattedText(text, CultureInfo.CurrentUICulture, FlowDirection.LeftToRight,
            new Typeface(font, FontStyles.Normal, weight ?? FontWeights.Normal, FontStretches.Normal), size, brush,
            VisualTreeHelper.GetDpi(this).PixelsPerDip)
        {
            TextAlignment = TextAlignment.Center,
            MaxLineCount = 1,
            Trimming = TextTrimming.CharacterEllipsis,
        };
        if (maxWidth > 0) ft.MaxTextWidth = maxWidth;
        var w = maxWidth > 0 ? maxWidth : ft.Width;
        // 中央揃えのとき、描く位置は幅の左端
        dc.DrawText(ft, new Point(center.X - (maxWidth > 0 ? w / 2 : 0), center.Y - ft.Height / 2));
    }

    /// <summary>角度（0° = 上・時計回り）と半径 → 中心からの位置（WPF は下が +）。</summary>
    private static Point Offset(Point c, double angle, double radius)
    {
        var t = angle * Math.PI / 180;
        return new Point(c.X + radius * Math.Sin(t), c.Y - radius * Math.Cos(t));
    }
}
