// Mac 版のテスト（mac/HikyakuRing/Tests/RingCoreTests）の移植と、Windows 版で足したもの。
// バイト列はファームのテスト（crates/hikyaku-pointer/src/ring.rs）と同じ。

using Xunit;

namespace Hikyaku.RingCore.Tests;

public class ProtocolTests
{
    [Fact]
    public void ParsesPositionEvent()
    {
        var e = RingEvent.Parse(new byte[] { 3, 1, 0x28, 0x23, 0x20, 0x03, 0x23, 0x01, 0x56, 0x04, 12, 7 });
        Assert.Equal(new RingEvent.Position(new RingSample(true, 90, 800, 0x123, 0x456, 12, 7)), e);
    }

    [Fact]
    public void ParsesOpenCloseAndRejectsShortData()
    {
        Assert.Equal(new RingEvent.Opened(), RingEvent.Parse(new byte[] { 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 }));
        Assert.Equal(new RingEvent.Closed(), RingEvent.Parse(new byte[] { 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 }));
        Assert.Null(RingEvent.Parse(new byte[] { 3, 1, 0 }));
        Assert.Null(RingEvent.Parse(new byte[] { 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 }));
    }

    [Fact]
    public void ParsesSettingsEvent()
    {
        // ring.rs の encodes_events と同じ（右に 90°、2 台目、スクロール反転）
        Assert.Equal(new RingEvent.Settings(DeviceRotation.R90, 1, true),
            RingEvent.Parse(new byte[] { 4, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 5 }));
        Assert.Null(RingEvent.Parse(new byte[] { 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 }));
    }

    [Fact]
    public void EncodesCommands()
    {
        Assert.Equal(new byte[] { 0x00 }, RingCommand.Hello);
        Assert.Equal(new byte[] { 0x01 }, RingCommand.Close);
        Assert.Equal(new byte[] { 0x02 }, RingCommand.Open);
        Assert.Equal(new byte[] { 0x10, 0x0A, 0x21 }, RingCommand.Key(new KeyCombo(0x0A, 0x21)));
        Assert.Equal(new byte[] { 0x11, 0xE9, 0x00 }, RingCommand.Consumer(ConsumerUsage.VolumeUp));
        Assert.Equal(new byte[] { 0x20, 3 }, RingCommand.SetRotation(DeviceRotation.R270));
        Assert.Equal(new byte[] { 0x21 }, RingCommand.GetSettings);
        Assert.Equal(new byte[] { 0x22, 1 }, RingCommand.SetScrollInvert(true));
        Assert.Equal(new byte[] { 0x23, 1 }, RingCommand.SwitchProfile(1));
    }
}

public class KeyTests
{
    [Theory]
    [InlineData(0x41, 0x04)] // A
    [InlineData(0x5A, 0x1D)] // Z
    [InlineData(0x31, 0x1E)] // 1
    [InlineData(0x30, 0x27)] // 0
    [InlineData(0x20, 0x2C)] // Space
    [InlineData(0x26, 0x52)] // ↑
    [InlineData(0x70, 0x3A)] // F1
    [InlineData(0x83, 0x6F)] // F20
    [InlineData(0x87, 0x73)] // F24
    [InlineData(0x61, 0x59)] // テンキー 1
    [InlineData(0x1D, 0x8B)] // 無変換
    public void MapsVirtualKeysToHidUsages(int vk, int usage)
    {
        Assert.Equal((byte)usage, HidKey.FromVirtualKey(vk));
    }

    [Fact]
    public void ModifierKeysAloneHaveNoUsage()
    {
        Assert.Null(HidKey.FromVirtualKey(0x10)); // Shift
        Assert.Null(HidKey.FromVirtualKey(0x5B)); // 左 Win
    }

    [Fact]
    public void LabelsKeyCombos()
    {
        Assert.Equal("Shift+Win+S", new KeyCombo(KeyCombo.Win | KeyCombo.Shift, 0x16).Label);
        Assert.Equal("Win+Space", new KeyCombo(0x80, 0x2C).Label); // 右 Win
        Assert.Equal("なし", new KeyCombo(0, 0).Label);
    }
}

public class ConfigTests
{
    [Fact]
    public void ConfigRoundTripsAndPadsToEightItems()
    {
        var dir = Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString());
        var path = Path.Combine(dir, "config.json");
        var config = RingConfig.Standard with { Tuning = new RingTuning { KnobStepDegrees = 20 } };
        ConfigStore.Save(config, path);
        var loaded = ConfigStore.Load(path);
        Assert.Equal(config.Items, loaded.Items);
        Assert.Equal(config.Tuning, loaded.Tuning);

        File.WriteAllText(path, """{"items":[{"name":"a","glyph":"x","action":{"kind":"none"}}]}""");
        var shortOne = ConfigStore.Load(path);
        Assert.Equal(8, shortOne.Items.Count);
        Assert.Equal("a", shortOne.Items[0].Name);
        Assert.Equal(new RingTuning(), shortOne.Tuning);
    }

    [Fact]
    public void BrokenConfigFallsBackToStandard()
    {
        var path = Path.Combine(Path.GetTempPath(), Guid.NewGuid() + ".json");
        File.WriteAllText(path, "{ not json");
        Assert.Equal(RingConfig.Standard.Items, ConfigStore.Load(path).Items);
    }
}

public class MachineTests
{
    private static RingEvent Touch(double angle, int radius = 800) => new RingEvent.Position(new RingSample(true, angle, radius));
    private static readonly RingEvent Lift = new RingEvent.Position(new RingSample(false, 0, 0));
    private static RingEvent[] Repeat(RingEvent e, int n) => Enumerable.Repeat(e, n).ToArray();

    private static RingMachine Machine() => new(new RingTuning(), RingConfig.Standard.Items.Select(i => i.Action).ToArray());

    /// <summary>70Hz でイベントを流す。</summary>
    private static List<RingEffect> Run(RingMachine m, IEnumerable<RingEvent> events, double t0 = 0)
    {
        var effects = new List<RingEffect>();
        var i = 0;
        foreach (var e in events) effects.AddRange(m.Handle(e, t0 + i++ / 70.0));
        return effects;
    }

    private static int Steps(IEnumerable<RingEffect> effects) => effects.OfType<RingEffect.KnobStep>().Sum(k => k.Steps);

    [Fact]
    public void PicksSectorsClockwiseFromUp()
    {
        var m = Machine();
        int? At(double a) => m.ItemAt(new RingSample(true, a, 800), null);
        Assert.Equal(0, At(0));
        Assert.Equal(0, At(350));
        Assert.Equal(0, At(22.4));
        Assert.Equal(1, At(22.6));
        Assert.Equal(2, At(90));
        Assert.Equal(7, At(315));
        Assert.Null(m.ItemAt(new RingSample(true, 90, 349), null));
    }

    [Fact]
    public void HysteresisKeepsTheCurrentItemNearTheBorder()
    {
        var m = Machine();
        RingSample S(double a) => new(true, a, 800);
        Assert.Equal(0, m.ItemAt(S(26), 0));
        Assert.Equal(1, m.ItemAt(S(29), 0));
        Assert.Equal(0, m.ItemAt(S(340), 0));
        Assert.Equal(1, m.ItemAt(S(19), 1));
    }

    [Fact]
    public void ReleaseRunsTheItemUnderTheFingerShortlyBefore()
    {
        var m = Machine();
        // 右（90° = 項目 2）でずっと触れていて、離す瞬間だけ 45°（項目 1）にぶれる
        var events = new[] { new RingEvent.Opened() }.Concat(Repeat(Touch(90), 10)).Append(Touch(45)).Concat(Repeat(Lift, 5));
        Assert.Equal(new RingEffect[] { new RingEffect.Show(), new RingEffect.Run(2), new RingEffect.SendClose(), new RingEffect.Hide() }, Run(m, events));
        Assert.Equal(RingModeKind.Closed, m.Mode.Kind);
    }

    [Fact]
    public void ReleaseInTheCenterCancels()
    {
        var m = Machine();
        var events = new[] { new RingEvent.Opened(), Touch(90) }.Concat(Repeat(Touch(90, 100), 8)).Concat(Repeat(Lift, 2));
        Assert.Equal(new RingEffect[] { new RingEffect.Show(), new RingEffect.SendClose(), new RingEffect.Hide() }, Run(m, events));
    }

    [Fact]
    public void ReleaseOnAnEmptySlotCancels()
    {
        var m = new RingMachine(new RingTuning(), new[] { RingAction.None }.Concat(Enumerable.Repeat(RingAction.ForMedia(MediaKey.PlayPause), 7)).ToArray());
        var events = new[] { new RingEvent.Opened() }.Concat(Repeat(Touch(0), 6)).Append(Lift);
        Assert.Equal(new RingEffect[] { new RingEffect.Show(), new RingEffect.SendClose(), new RingEffect.Hide() }, Run(m, events));
    }

    [Fact]
    public void ClosedFromTheFirmwareHidesTheRing()
    {
        var m = Machine();
        Assert.Equal(new RingEffect[] { new RingEffect.Show(), new RingEffect.Hide() }, Run(m, [new RingEvent.Opened(), Touch(90), new RingEvent.Closed()]));
        Assert.Equal(RingModeKind.Closed, m.Mode.Kind);
        Assert.Empty(m.Handle(new RingEvent.Closed(), 1));
    }

    [Fact]
    public void SettingsEventDoesNotTouchTheRing()
    {
        var m = Machine();
        m.Handle(new RingEvent.Opened(), 0);
        Assert.Empty(m.Handle(new RingEvent.Settings(DeviceRotation.R270, 1, false), 0.01));
        Assert.Equal(RingModeKind.Selecting, m.Mode.Kind);
    }

    [Fact]
    public void OpenSettingsItemRunsLikeANormalItem()
    {
        var m = Machine();
        // 左上（315° = 項目 7）は標準で「設定」
        var events = new[] { new RingEvent.Opened() }.Concat(Repeat(Touch(315), 6)).Append(Lift);
        Assert.Contains(new RingEffect.Run(7), Run(m, events));
    }

    /// <summary>項目 0（音量）を選んでノブモードに入る。</summary>
    private static RingMachine KnobMachine()
    {
        var m = Machine();
        var effects = Run(m, new[] { new RingEvent.Opened() }.Concat(Repeat(Touch(0), 6)).Concat(Repeat(Lift, 5)));
        Assert.Equal(new RingEffect[] { new RingEffect.Show(), new RingEffect.EnterKnob(0) }, effects);
        Assert.Equal(new RingMode(RingModeKind.Knob, 0), m.Mode);
        return m;
    }

    [Fact]
    public void KnobCountsStepsAcrossZeroDegrees()
    {
        var m = KnobMachine();
        // 340° → 20° へ右回り（0° をまたぐ）。40° = 15° × 2 + 端数 10°
        Assert.Equal(2, Steps(Run(m, [Touch(340), Touch(350), Touch(0), Touch(10), Touch(20)], 10)));
        Assert.True(Math.Abs(m.KnobRemainder - 10) < 0.001);
        // 左回りに 40° 戻すと、端数と合わせて -2
        Assert.Equal(-2, Steps(Run(m, [Touch(10), Touch(0), Touch(340)], 11)));
        Assert.Equal(RingModeKind.Knob, m.Mode.Kind);
    }

    [Fact]
    public void KnobIgnoresTheJumpBetweenTouches()
    {
        var m = KnobMachine();
        Run(m, [Touch(0), Touch(5)], 10);
        Assert.Empty(Run(m, [Lift, Lift, Touch(180), Touch(185)], 11));
    }

    [Fact]
    public void TapInTheCenterLeavesKnobMode()
    {
        var m = KnobMachine();
        Assert.Equal(new RingEffect[] { new RingEffect.SendClose(), new RingEffect.Hide() }, Run(m, [Touch(0, 100), Touch(10, 120), Lift, Lift], 10));
        Assert.Equal(RingModeKind.Closed, m.Mode.Kind);
    }

    [Fact]
    public void LongPressOrDragInTheCenterDoesNotLeaveKnobMode()
    {
        var m = KnobMachine();
        Assert.Empty(Run(m, Repeat(Touch(0, 100), 25).Append(Lift), 10));
        Assert.Empty(Run(m, [Touch(0, 100), Touch(0, 500), Lift], 20));
        Assert.Equal(RingModeKind.Knob, m.Mode.Kind);
    }

    [Theory]
    [InlineData(350, 10, 20)]
    [InlineData(10, 350, -20)]
    [InlineData(0, 180, -180)]
    [InlineData(90, 45, -45)]
    public void AngleDeltaWrapsAround(double from, double to, double want)
    {
        Assert.Equal(want, RingMachine.AngleDelta(from, to), 6);
    }
}
