// hikyaku 本体の簡単な絵（基準の向き）: 青いトッププレート、中央に黒い円のパッド、左下にリセットボタン、右下にオレンジのキースイッチ。
// 寸法は見た目の目安（縦 100 に対する比率）。Mac 版の HikyakuDrawing と同じ。

using System;
using System.Windows;
using System.Windows.Media;

namespace Hikyaku.Ring;

public sealed class HikyakuDrawing : FrameworkElement
{
    private static readonly Brush Plate = Make(0x29, 0x66, 0xDB);
    private static readonly Brush Pad = Make(0x14, 0x14, 0x14);
    private static readonly Brush Reset = Make(0xD9, 0xD9, 0xD9);
    private static readonly Brush Switch = Make(0xFA, 0x8C, 0x1F);
    private static readonly Brush Stem = Make(0xCC, 0x61, 0x0D);

    private static Brush Make(byte r, byte g, byte b)
    {
        var brush = new SolidColorBrush(Color.FromRgb(r, g, b));
        brush.Freeze();
        return brush;
    }

    protected override void OnRender(DrawingContext dc)
    {
        var s = Math.Min(ActualWidth, ActualHeight) / 100;
        var ox = (ActualWidth - 80 * s) / 2;
        var oy = (ActualHeight - 100 * s) / 2;
        Rect R(double x, double y, double w, double h) => new(ox + x * s, oy + y * s, w * s, h * s);
        Point P(double x, double y) => new(ox + x * s, oy + y * s);

        dc.DrawRoundedRectangle(Plate, new Pen(new SolidColorBrush(Color.FromArgb(0x40, 0, 0, 0)), 1), R(0, 0, 80, 100), 9 * s, 9 * s);
        dc.DrawEllipse(Pad, new Pen(new SolidColorBrush(Color.FromArgb(0x26, 0xFF, 0xFF, 0xFF)), 1), P(40, 40), 28 * s, 28 * s);
        dc.DrawEllipse(Reset, new Pen(new SolidColorBrush(Color.FromArgb(0x66, 0, 0, 0)), 0.8), P(12.5, 87.5), 3.5 * s, 3.5 * s);
        dc.DrawRoundedRectangle(Switch, null, R(55, 76, 17, 17), 2.5 * s, 2.5 * s);
        dc.DrawRectangle(Stem, null, R(62.5, 79.5, 2, 10));
        dc.DrawRectangle(Stem, null, R(58.5, 83.5, 10, 2));
    }
}
