// リングの項目の割り当てと調整値。%APPDATA%\HikyakuRing\config.json に保存する。
// 考え方は Mac 版（mac/HikyakuRing/Sources/RingCore/Config.swift）と同じ。アイコンは Segoe Fluent Icons の文字コード。

using System.Text.Json;
using System.Text.Json.Serialization;

namespace Hikyaku.RingCore;

/// <summary>連続値の項目（指を離すとノブ操作モードになる）。</summary>
public enum KnobKind
{
    Volume,
    Brightness,
    Seek,
}

/// <summary>メディアキーで実行する、1 回きりの項目。</summary>
public enum MediaKey
{
    PlayPause,
    NextTrack,
    PreviousTrack,
    Mute,
}

public enum ActionKind
{
    None,
    Keys,
    Media,
    Knob,
    /// <summary>このアプリの設定画面を開く</summary>
    OpenSettings,
}

public static class ActionText
{
    public static string Title(this KnobKind k) => k switch
    {
        KnobKind.Volume => "音量",
        KnobKind.Brightness => "画面の明るさ（内蔵画面）",
        _ => "シーク（再生中のアプリ）",
    };

    public static string Title(this MediaKey k) => k switch
    {
        MediaKey.PlayPause => "再生 / 一時停止",
        MediaKey.NextTrack => "次の曲",
        MediaKey.PreviousTrack => "前の曲",
        _ => "消音",
    };

    public static ushort Usage(this MediaKey k) => k switch
    {
        MediaKey.PlayPause => ConsumerUsage.PlayPause,
        MediaKey.NextTrack => ConsumerUsage.NextTrack,
        MediaKey.PreviousTrack => ConsumerUsage.PreviousTrack,
        _ => ConsumerUsage.Mute,
    };

    public static string Title(this ActionKind k) => k switch
    {
        ActionKind.None => "なし",
        ActionKind.Keys => "キーの組み合わせ",
        ActionKind.Media => "メディアキー",
        ActionKind.Knob => "連続値（ノブで調整）",
        _ => "設定画面を開く",
    };
}

/// <summary>項目のアクション。Kind に応じて Keys / Media / Knob のどれかを使う。</summary>
public sealed record RingAction
{
    public ActionKind Kind { get; init; }
    public KeyCombo Keys { get; init; }
    public MediaKey Media { get; init; }
    public KnobKind Knob { get; init; }

    public static readonly RingAction None = new();
    public static RingAction ForKeys(KeyCombo c) => new() { Kind = ActionKind.Keys, Keys = c };
    public static RingAction ForMedia(MediaKey m) => new() { Kind = ActionKind.Media, Media = m };
    public static RingAction ForKnob(KnobKind k) => new() { Kind = ActionKind.Knob, Knob = k };
    public static readonly RingAction OpenSettings = new() { Kind = ActionKind.OpenSettings };

    [JsonIgnore]
    public bool IsKnob => Kind == ActionKind.Knob;

    /// <summary>種類を変えたときの初期値。</summary>
    public static RingAction Default(ActionKind kind) => kind switch
    {
        ActionKind.Keys => ForKeys(new KeyCombo(KeyCombo.Win, 0x2C)),
        ActionKind.Media => ForMedia(MediaKey.PlayPause),
        ActionKind.Knob => ForKnob(KnobKind.Volume),
        ActionKind.OpenSettings => OpenSettings,
        _ => None,
    };
}

public sealed record RingItem(string Name, string Glyph, RingAction Action)
{
    public static readonly RingItem Empty = new("", Glyphs.Empty, RingAction.None);
}

/// <summary>Segoe Fluent Icons（Windows 11）/ Segoe MDL2 Assets（Windows 10）の文字。</summary>
public static class Glyphs
{
    public const string Volume = "";
    public const string Next = "";
    public const string Previous = "";
    public const string Play = "";
    public const string Camera = "";
    public const string Clock = "";
    public const string TaskView = "";
    public const string Brightness = "";
    public const string Search = "";
    public const string Settings = "";
    public const string Mute = "";
    public const string Empty = "";
}

/// <summary>アプリ側で持つ調整値（ファームの書き換えなしで試せる）。</summary>
public sealed record RingTuning
{
    /// <summary>これより中心に近い位置はキャンセル範囲（パッドの縁 ≒ 1000）</summary>
    public int CancelRadius { get; init; } = 350;
    /// <summary>項目の境目のヒステリシス（度）</summary>
    public double HysteresisDegrees { get; init; } = 6;
    /// <summary>指を離したとき、この時間だけ前の位置で項目を判定する（ミリ秒）</summary>
    public double ReleaseLookbackMs { get; init; } = 50;
    /// <summary>ノブ操作で 1 段階に要する回転（度）</summary>
    public double KnobStepDegrees { get; init; } = 15;
    /// <summary>ノブモードを抜けるタップの長さの上限（ミリ秒）</summary>
    public double TapMaxMs { get; init; } = 250;
    /// <summary>タップとみなす動きの上限（パッドの縁 ≒ 1000 の単位）</summary>
    public int TapMaxMove { get; init; } = 150;
    /// <summary>シークの 1 段階（秒）</summary>
    public double SeekStepSeconds { get; init; } = 5;
    /// <summary>明るさの 1 段階（%）</summary>
    public int BrightnessStepPercent { get; init; } = 5;
}

public sealed record RingConfig
{
    public const int SlotCount = 8;

    /// <summary>8 か所。0 = 上、時計回りに 45° ずつ</summary>
    public IReadOnlyList<RingItem> Items { get; init; } = Padded([]);
    public RingTuning Tuning { get; init; } = new();

    public RingConfig WithItem(int index, RingItem item)
    {
        var items = Items.ToArray();
        items[index] = item;
        return this with { Items = items };
    }

    public static IReadOnlyList<RingItem> Padded(IEnumerable<RingItem> items) =>
        items.Concat(Enumerable.Repeat(RingItem.Empty, SlotCount)).Take(SlotCount).ToArray();

    public static readonly RingConfig Standard = new()
    {
        Items = Padded([
            new RingItem("音量", Glyphs.Volume, RingAction.ForKnob(KnobKind.Volume)),
            new RingItem("次の曲", Glyphs.Next, RingAction.ForMedia(MediaKey.NextTrack)),
            new RingItem("スクリーンショット", Glyphs.Camera, RingAction.ForKeys(new KeyCombo(KeyCombo.Win | KeyCombo.Shift, 0x16))), // Win+Shift+S
            new RingItem("シーク", Glyphs.Clock, RingAction.ForKnob(KnobKind.Seek)),
            new RingItem("再生 / 停止", Glyphs.Play, RingAction.ForMedia(MediaKey.PlayPause)),
            new RingItem("タスクビュー", Glyphs.TaskView, RingAction.ForKeys(new KeyCombo(KeyCombo.Win, 0x2B))), // Win+Tab
            new RingItem("明るさ", Glyphs.Brightness, RingAction.ForKnob(KnobKind.Brightness)),
            new RingItem("設定", Glyphs.Settings, RingAction.OpenSettings),
        ]),
    };
}

public static class ConfigStore
{
    public static string DefaultPath => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "HikyakuRing", "config.json");

    private static readonly JsonSerializerOptions Options = new()
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) },
    };

    /// <summary>読めなければ（初回・壊れている）標準の割り当て。項目が 8 個に満たなければ空きで埋める。</summary>
    public static RingConfig Load(string? path = null)
    {
        try
        {
            var json = File.ReadAllText(path ?? DefaultPath);
            var c = JsonSerializer.Deserialize<RingConfig>(json, Options);
            if (c is null) return RingConfig.Standard;
            return c with { Items = RingConfig.Padded(c.Items ?? []), Tuning = c.Tuning ?? new RingTuning() };
        }
        catch (Exception)
        {
            return RingConfig.Standard;
        }
    }

    public static void Save(RingConfig config, string? path = null)
    {
        path ??= DefaultPath;
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var tmp = path + ".tmp";
        File.WriteAllText(tmp, JsonSerializer.Serialize(config, Options));
        File.Move(tmp, path, overwrite: true);
    }
}
