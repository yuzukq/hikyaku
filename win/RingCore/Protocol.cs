// hikyaku の独自サービス（ring）の形式。ファーム側: crates/hikyaku-pointer/src/ring.rs、仕様: docs/design/ring.md

namespace Hikyaku.RingCore;

public static class RingUuid
{
    public static readonly Guid Service = Guid.Parse("6b9a0010-6f1e-4b8e-9d57-7f0d0a5b0001");
    /// <summary>hikyaku → アプリ（read / notify、12 バイト固定）</summary>
    public static readonly Guid State = Guid.Parse("6b9a0011-6f1e-4b8e-9d57-7f0d0a5b0001");
    /// <summary>アプリ → hikyaku（write / write without response）</summary>
    public static readonly Guid Ctrl = Guid.Parse("6b9a0012-6f1e-4b8e-9d57-7f0d0a5b0001");
}

/// <summary>パッドに触れている指の状態（state の種類 3）。角度は 0° = 上・時計回り、距離はパッドの縁 ≒ 1000。</summary>
public readonly record struct RingSample(bool Touching, double Angle, int Radius, int X = 0, int Y = 0, int Z = 0, byte Seq = 0);

/// <summary>hikyaku 本体の置き方。基準の向きから時計回りに 90° 単位（ファーム側: hikyaku_pointer::Rotation）。</summary>
public enum DeviceRotation
{
    R0 = 0,
    R90 = 1,
    R180 = 2,
    R270 = 3,
}

public static class DeviceRotationExtensions
{
    public static double Degrees(this DeviceRotation r) => (int)r * 90;

    public static string Title(this DeviceRotation r) => r switch
    {
        DeviceRotation.R0 => "基準",
        DeviceRotation.R90 => "右に 90°",
        DeviceRotation.R180 => "180°",
        _ => "左に 90°",
    };
}

public abstract record RingEvent
{
    public sealed record Opened : RingEvent;
    public sealed record Closed : RingEvent;
    public sealed record Position(RingSample Sample) : RingEvent;
    /// <summary>今の設定（HELLO・問い合わせ・変更への返事）。Profile = 今の送り先（0 = hikyaku、1 = hikyaku2）。</summary>
    public sealed record Settings(DeviceRotation Rotation, int Profile, bool ScrollInvert) : RingEvent;

    /// <summary>state の 12 バイトを読む。形式が違えば null。</summary>
    public static RingEvent? Parse(ReadOnlySpan<byte> b)
    {
        if (b.Length < 12) return null;
        static int U16(ReadOnlySpan<byte> b, int i) => b[i] | b[i + 1] << 8;
        return b[0] switch
        {
            1 => new Opened(),
            2 => new Closed(),
            3 => new Position(new RingSample((b[1] & 1) != 0, U16(b, 2) / 100.0, U16(b, 4), U16(b, 6), U16(b, 8), b[10], b[11])),
            4 when b[1] <= 3 => new Settings((DeviceRotation)b[1], b[2], b[3] != 0),
            _ => null,
        };
    }
}

/// <summary>アプリ → hikyaku の命令（ctrl に書くバイト列）。</summary>
public static class RingCommand
{
    /// <summary>接続したら最初に送る。これを受け取るまで F23 でリングは開かない</summary>
    public static byte[] Hello => [0x00];
    public static byte[] Close => [0x01];
    public static byte[] Open => [0x02];
    /// <summary>キーの組み合わせを押して離す（hikyaku から本物のキーボード入力として届く）</summary>
    public static byte[] Key(KeyCombo combo) => [0x10, combo.Modifiers, combo.Usage];
    /// <summary>メディアキー（Consumer の Usage）を押して離す</summary>
    public static byte[] Consumer(ushort usage) => [0x11, (byte)(usage & 0xFF), (byte)(usage >> 8)];
    public static byte[] SetRotation(DeviceRotation r) => [0x20, (byte)r];
    public static byte[] GetSettings => [0x21];
    /// <summary>今の送り先でスクロールを反転するか（hikyaku に保存）</summary>
    public static byte[] SetScrollInvert(bool on) => [0x22, (byte)(on ? 1 : 0)];
    /// <summary>送り先を切り替える。hikyaku は再起動して、その送り先の ID でつなぎ直す（今の接続は切れる）</summary>
    public static byte[] SwitchProfile(int profile) => [0x23, (byte)profile];
}

/// <summary>HID Consumer Page の Usage ID。</summary>
public static class ConsumerUsage
{
    public const ushort VolumeUp = 0x00E9;
    public const ushort VolumeDown = 0x00EA;
    public const ushort Mute = 0x00E2;
    public const ushort PlayPause = 0x00CD;
    public const ushort NextTrack = 0x00B5;
    public const ushort PreviousTrack = 0x00B6;
}
