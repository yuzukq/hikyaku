// リングを出す透明な窓。フォーカスを奪わず、マウスも受けない（操作はすべてパッドから）。
// 窓の位置は、カーソルのあるモニタの DPI に合わせて実際のピクセルで決める（モニタをまたいでも正しい位置に出す）。

using System;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;

namespace Hikyaku.Ring;

public sealed class OverlayWindow : Window
{
    /// <summary>窓の大きさ（DIP）。</summary>
    public const double Size = 400;

    public OverlayWindow(RingController controller)
    {
        WindowStyle = WindowStyle.None;
        AllowsTransparency = true;
        Background = Brushes.Transparent;
        Topmost = true;
        ShowInTaskbar = false;
        ShowActivated = false;
        ResizeMode = ResizeMode.NoResize;
        Width = Size;
        Height = Size;
        Content = new RingVisual(controller);
        SourceInitialized += (_, _) =>
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            var ex = NativeMethods.GetWindowLongPtr(hwnd, NativeMethods.GWL_EXSTYLE).ToInt64();
            ex |= NativeMethods.WS_EX_NOACTIVATE | NativeMethods.WS_EX_TRANSPARENT | NativeMethods.WS_EX_TOOLWINDOW;
            NativeMethods.SetWindowLongPtr(hwnd, NativeMethods.GWL_EXSTYLE, new IntPtr(ex));
        };
    }

    /// <summary>カーソルを中心に出す。画面の端では、はみ出さないようにずらす。</summary>
    public void ShowNearCursor()
    {
        var hwnd = new WindowInteropHelper(this).EnsureHandle();
        NativeMethods.GetCursorPos(out var cursor);
        var monitor = NativeMethods.MonitorFromPoint(cursor, NativeMethods.MONITOR_DEFAULTTONEAREST);
        var info = new NativeMethods.MONITORINFO { cbSize = System.Runtime.InteropServices.Marshal.SizeOf<NativeMethods.MONITORINFO>() };
        NativeMethods.GetMonitorInfo(monitor, ref info);
        var scale = NativeMethods.GetDpiForMonitor(monitor, 0, out var dpi, out _) == 0 ? dpi / 96.0 : 1.0;
        var size = (int)Math.Round(Size * scale);
        var work = info.rcWork;
        var x = Math.Clamp(cursor.X - size / 2, work.Left, Math.Max(work.Left, work.Right - size));
        var y = Math.Clamp(cursor.Y - size / 2, work.Top, Math.Max(work.Top, work.Bottom - size));
        NativeMethods.SetWindowPos(hwnd, NativeMethods.HWND_TOPMOST, x, y, size, size, NativeMethods.SWP_NOACTIVATE);
        Show();
        // 表示したあとで WPF が DPI に合わせて大きさを直すことがあるので、もう一度合わせる
        NativeMethods.SetWindowPos(hwnd, NativeMethods.HWND_TOPMOST, x, y, size, size, NativeMethods.SWP_NOACTIVATE);
    }
}
