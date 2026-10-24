// 全体のホットキー Ctrl+Alt+R（リングの開閉）。メッセージ専用の窓で WM_HOTKEY を受ける。

using System;
using System.Windows.Interop;

namespace Hikyaku.Ring;

internal sealed class HotKey : IDisposable
{
    private const int Id = 0x484B; // 'HK'
    private readonly HwndSource _source;
    private readonly Action _pressed;

    public HotKey(Action pressed)
    {
        _pressed = pressed;
        _source = new HwndSource(new HwndSourceParameters("HikyakuRingHotKey") { ParentWindow = new IntPtr(-3) }); // HWND_MESSAGE
        _source.AddHook(Hook);
        if (!NativeMethods.RegisterHotKey(_source.Handle, Id, NativeMethods.MOD_CONTROL | NativeMethods.MOD_ALT | NativeMethods.MOD_NOREPEAT, 'R'))
        {
            Log.Error("registering Ctrl+Alt+R failed (used by another app?)");
        }
    }

    private IntPtr Hook(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == NativeMethods.WM_HOTKEY && wParam.ToInt32() == Id)
        {
            _pressed();
            handled = true;
        }
        return IntPtr.Zero;
    }

    public void Dispose()
    {
        NativeMethods.UnregisterHotKey(_source.Handle, Id);
        _source.Dispose();
    }
}
