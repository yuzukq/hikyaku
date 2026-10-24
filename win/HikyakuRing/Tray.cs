// タスクトレイのアイコンとメニュー。ダブルクリックで設定画面。

using System;
using System.Drawing;
using Forms = System.Windows.Forms;

namespace Hikyaku.Ring;

internal sealed class Tray : IDisposable
{
    private readonly Forms.NotifyIcon _icon;
    private readonly RingController _controller;
    private readonly Forms.ToolStripMenuItem _status = new() { Enabled = false };
    private readonly Forms.ToolStripMenuItem _toggle;
    private readonly Icon _ready = MakeIcon(true);
    private readonly Icon _idle = MakeIcon(false);

    public Tray(RingController controller, Action quit)
    {
        _controller = controller;
        _toggle = new Forms.ToolStripMenuItem("リングを開く / 閉じる（Ctrl+Alt+R）", null, (_, _) => controller.ToggleRing());
        var menu = new Forms.ContextMenuStrip();
        menu.Items.Add(_status);
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add(_toggle);
        menu.Items.Add(new Forms.ToolStripMenuItem("設定…", null, (_, _) => controller.ShowSettings()));
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add(new Forms.ToolStripMenuItem("終了", null, (_, _) => quit()));
        menu.Opening += (_, _) => Refresh();
        _icon = new Forms.NotifyIcon { ContextMenuStrip = menu, Visible = true };
        _icon.DoubleClick += (_, _) => controller.ShowSettings();
        controller.Changed += Refresh;
        Refresh();
    }

    private void Refresh()
    {
        var status = _controller.Status;
        var profile = _controller.Profile is { } p ? $"（送り先 {p + 1}）" : "";
        _status.Text = status.Label + profile;
        _toggle.Enabled = status.IsReady;
        _icon.Icon = status.IsReady ? _ready : _idle;
        var tip = "hikyaku リング: " + status.Label;
        _icon.Text = tip.Length > 63 ? tip[..63] : tip;
    }

    /// <summary>リングの形のアイコン（つながっていれば中を塗る）。</summary>
    private static Icon MakeIcon(bool filled)
    {
        using var bmp = new Bitmap(32, 32);
        using (var g = Graphics.FromImage(bmp))
        {
            g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            g.Clear(Color.Transparent);
            using var pen = new Pen(Color.White, 3);
            g.DrawEllipse(pen, 3, 3, 26, 26);
            if (filled)
            {
                using var brush = new SolidBrush(Color.White);
                g.FillEllipse(brush, 11, 11, 10, 10);
            }
            else
            {
                g.DrawEllipse(pen, 12, 12, 8, 8);
            }
        }
        return Icon.FromHandle(bmp.GetHicon());
    }

    public void Dispose()
    {
        _icon.Visible = false;
        _icon.Dispose();
    }
}
