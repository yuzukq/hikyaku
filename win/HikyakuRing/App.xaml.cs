// hikyaku のショートカットリング（Windows）: タスクトレイに常駐し、F23・本体のボタンのダブルタップ・Ctrl+Alt+R でリングを出す。

using System;
using System.Threading;
using System.Windows;

namespace Hikyaku.Ring;

public partial class App : Application
{
    private Mutex? _single;
    private RingController? _controller;
    private Tray? _tray;
    private HotKey? _hotKey;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        // 二重に起動しない（hikyaku への接続を取り合わないように）
        _single = new Mutex(true, "HikyakuRing.SingleInstance", out var first);
        if (!first)
        {
            Shutdown();
            return;
        }
        Log.Info($"start {Log.Version}");
        DispatcherUnhandledException += (_, args) =>
        {
            Log.Error($"unhandled: {args.Exception}");
            args.Handled = true;
        };

        _controller = new RingController();
        _tray = new Tray(_controller, Quit);
        // 7sPro なしで試すとき・キーボードから開くとき用（F23 と同じく hikyaku に 02 / 01 を送るだけ）
        _hotKey = new HotKey(() => _controller.ToggleRing());
        _controller.Start();
    }

    private void Quit()
    {
        _controller?.CloseRing();
        _hotKey?.Dispose();
        _tray?.Dispose();
        _controller?.Dispose();
        Shutdown();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _single?.Dispose();
        base.OnExit(e);
    }
}
