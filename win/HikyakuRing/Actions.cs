// 項目の実行。キーとメディアキーは hikyaku に頼んで本物のキーボード入力として送る（Mac と同じ）。
// 明るさは WMI（内蔵画面）、シークは Windows のメディア操作（GSMTC）で変える。音量の表示は Core Audio で読む。

using System;
using System.Linq;
using System.Management;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Hikyaku.RingCore;
using Windows.Media.Control;

namespace Hikyaku.Ring;

/// <summary>
/// ノブの表示に使う、今の値。回すたびに読み直すと表示が指に遅れるので、1 段階ごとに見込みの値へすぐ進め（Stepped）、
/// 回し終えてから本当の値を読み直す。
/// </summary>
public sealed record KnobReadout(double Value, double Maximum, double Step, bool IsClock = false, string? Message = null)
{
    /// <summary>0〜1（分からなければ null）。partialSteps は 1 段階に満たない回転（段階の何割か）</summary>
    public double? Fraction(double partialSteps = 0) =>
        Maximum > 0 ? Math.Clamp((Value + partialSteps * Step) / Maximum, 0, 1) : null;

    public string Text => Message ?? (IsClock ? $"{Clock(Value)} / {Clock(Maximum)}" : $"{Math.Round(Value / Maximum * 100)}%");

    public KnobReadout Stepped(int steps) => Maximum > 0 ? this with { Value = Math.Clamp(Value + steps * Step, 0, Maximum) } : this;

    private static string Clock(double seconds)
    {
        var s = Math.Max((int)seconds, 0);
        return $"{s / 60}:{s % 60:00}";
    }
}

public sealed class ActionRunner(Action<byte[]> send)
{
    public void Run(RingAction action)
    {
        switch (action.Kind)
        {
            case ActionKind.Keys:
                send(RingCommand.Key(action.Keys));
                break;
            case ActionKind.Media:
                send(RingCommand.Consumer(action.Media.Usage()));
                break;
        }
    }

    public void Turn(KnobKind kind, int steps, RingTuning tuning)
    {
        switch (kind)
        {
            case KnobKind.Volume:
                for (var i = 0; i < Math.Abs(steps); i++)
                {
                    send(RingCommand.Consumer(steps > 0 ? ConsumerUsage.VolumeUp : ConsumerUsage.VolumeDown));
                }
                break;
            case KnobKind.Brightness:
                _ = Task.Run(() => Brightness.Change(steps * tuning.BrightnessStepPercent));
                break;
            case KnobKind.Seek:
                _ = MediaSessions.SeekByAsync(steps * tuning.SeekStepSeconds);
                break;
        }
    }

    /// <summary>今の値（音量・明るさ・再生位置）。</summary>
    public async Task<KnobReadout?> ReadoutAsync(KnobKind kind, RingTuning tuning)
    {
        switch (kind)
        {
            case KnobKind.Volume:
                // Windows の音量キー 1 回 = 2%
                return SystemVolume.Read() is { } v ? new KnobReadout(v, 1, 0.02) : null;
            case KnobKind.Brightness:
                var b = await Task.Run(Brightness.Read);
                return b is { } pct
                    ? new KnobReadout(pct, 100, tuning.BrightnessStepPercent)
                    : new KnobReadout(0, 0, 0, Message: "この画面は明るさを変えられない（外部モニタ）");
            default:
                return await MediaSessions.ReadoutAsync(tuning.SeekStepSeconds);
        }
    }
}

/// <summary>既定の出力機器の音量（0〜1）。Core Audio の IAudioEndpointVolume。</summary>
public static class SystemVolume
{
    public static double? Read()
    {
        try
        {
            var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumeratorCom();
            Marshal.ThrowExceptionForHR(enumerator.GetDefaultAudioEndpoint(0 /* eRender */, 1 /* eMultimedia */, out var device));
            var iid = typeof(IAudioEndpointVolume).GUID;
            Marshal.ThrowExceptionForHR(device.Activate(ref iid, 23 /* CLSCTX_ALL */, IntPtr.Zero, out var obj));
            var volume = (IAudioEndpointVolume)obj;
            Marshal.ThrowExceptionForHR(volume.GetMasterVolumeLevelScalar(out var level));
            return level;
        }
        catch (Exception e)
        {
            Log.Error($"volume: {e.Message}");
            return null;
        }
    }

    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
    private class MMDeviceEnumeratorCom;

    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDeviceEnumerator
    {
        int EnumAudioEndpoints(int dataFlow, int stateMask, out IntPtr devices);
        int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice device);
    }

    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDevice
    {
        int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
    }

    [ComImport, Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IAudioEndpointVolume
    {
        int RegisterControlChangeNotify(IntPtr notify);
        int UnregisterControlChangeNotify(IntPtr notify);
        int GetChannelCount(out uint count);
        int SetMasterVolumeLevel(float levelDb, ref Guid context);
        int SetMasterVolumeLevelScalar(float level, ref Guid context);
        int GetMasterVolumeLevel(out float levelDb);
        int GetMasterVolumeLevelScalar(out float level);
    }
}

/// <summary>内蔵画面の明るさ（%）。WMI の WmiMonitorBrightness。外部モニタ（DDC/CI）は未対応。</summary>
public static class Brightness
{
    private static readonly ManagementScope Scope = new(@"root\WMI");

    public static int? Read()
    {
        try
        {
            using var searcher = new ManagementObjectSearcher(Scope, new SelectQuery("WmiMonitorBrightness"));
            foreach (var o in searcher.Get().Cast<ManagementObject>())
            {
                return Convert.ToInt32(o["CurrentBrightness"]);
            }
        }
        catch (Exception e)
        {
            Log.Error($"brightness read: {e.Message}");
        }
        return null;
    }

    public static void Change(int deltaPercent)
    {
        if (Read() is not { } now) return;
        var target = (byte)Math.Clamp(now + deltaPercent, 0, 100);
        try
        {
            using var searcher = new ManagementObjectSearcher(Scope, new SelectQuery("WmiMonitorBrightnessMethods"));
            foreach (var o in searcher.Get().Cast<ManagementObject>())
            {
                o.InvokeMethod("WmiSetBrightness", new object[] { 1u, target });
            }
        }
        catch (Exception e)
        {
            Log.Error($"brightness set: {e.Message}");
        }
    }
}

/// <summary>再生中のアプリ（Spotify・メディア プレーヤーなど）の再生位置。Windows のメディア操作（GSMTC）。</summary>
public static class MediaSessions
{
    private static async Task<GlobalSystemMediaTransportControlsSession?> CurrentAsync()
    {
        try
        {
            var manager = await GlobalSystemMediaTransportControlsSessionManager.RequestAsync();
            return manager.GetCurrentSession();
        }
        catch (Exception e)
        {
            Log.Error($"media session: {e.Message}");
            return null;
        }
    }

    /// <summary>今の位置（秒）と長さ（秒）。位置は最後に更新された時刻から、再生中なら経過分を足す。</summary>
    private static (double Position, double Duration) Timeline(GlobalSystemMediaTransportControlsSession s)
    {
        var t = s.GetTimelineProperties();
        var position = t.Position;
        if (s.GetPlaybackInfo().PlaybackStatus == GlobalSystemMediaTransportControlsSessionPlaybackStatus.Playing)
        {
            position += DateTimeOffset.Now - t.LastUpdatedTime;
        }
        return (position.TotalSeconds, (t.EndTime - t.StartTime).TotalSeconds);
    }

    public static async Task SeekByAsync(double seconds)
    {
        if (await CurrentAsync() is not { } s) return;
        var (position, duration) = Timeline(s);
        var target = Math.Clamp(position + seconds, 0, Math.Max(duration, 0));
        var ok = await s.TryChangePlaybackPositionAsync(TimeSpan.FromSeconds(target).Ticks);
        if (!ok) Log.Error($"seek: {s.SourceAppUserModelId} did not accept the new position");
    }

    public static async Task<KnobReadout> ReadoutAsync(double step)
    {
        if (await CurrentAsync() is not { } s) return new KnobReadout(0, 0, 0, Message: "再生中のアプリなし");
        var (position, duration) = Timeline(s);
        return duration > 0 ? new KnobReadout(position, duration, step, IsClock: true) : new KnobReadout(0, 0, 0, Message: "位置が分からない");
    }
}
