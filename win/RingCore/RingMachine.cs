// リングの状態遷移: hikyaku からのイベント → 表示の状態と、アプリがすること（実行・閉じる・ノブの 1 段階）。
// Mac 版（mac/HikyakuRing/Sources/RingCore/RingMachine.swift）の移植。時刻は呼び出し側から渡す（秒、単調増加）。

namespace Hikyaku.RingCore;

public abstract record RingEffect
{
    /// <summary>リングを表示する（カーソルの位置付近）</summary>
    public sealed record Show : RingEffect;
    /// <summary>表示を消す</summary>
    public sealed record Hide : RingEffect;
    /// <summary>普通の項目を実行する</summary>
    public sealed record Run(int Item) : RingEffect;
    /// <summary>ノブ操作モードに入った（hikyaku のリングは開いたまま）</summary>
    public sealed record EnterKnob(int Item) : RingEffect;
    /// <summary>ノブを回した（+ = 右回り）</summary>
    public sealed record KnobStep(int Item, int Steps) : RingEffect;
    /// <summary>hikyaku に「閉じる」（01）を送る</summary>
    public sealed record SendClose : RingEffect;
}

public enum RingModeKind
{
    Closed,
    Selecting,
    Knob,
}

/// <summary>Selecting: Item = 指の方向の項目（キャンセル範囲や触れていなければ null）。Knob: Item = ノブの項目。</summary>
public readonly record struct RingMode(RingModeKind Kind, int? Item = null)
{
    public static readonly RingMode Closed = new(RingModeKind.Closed);
}

public sealed class RingMachine(RingTuning tuning, IReadOnlyList<RingAction> actions)
{
    public RingTuning Tuning { get; set; } = tuning;
    /// <summary>8 か所それぞれの割り当て（ノブの項目か・空きかの判定に使う）</summary>
    public IReadOnlyList<RingAction> Actions { get; set; } = actions;

    public RingMode Mode { get; private set; } = RingMode.Closed;
    /// <summary>最後に受け取った指の状態（表示用）</summary>
    public RingSample? Finger { get; private set; }
    /// <summary>ノブ: 右回りを + とした、積算中の回転（度。1 段階に満たない端数）</summary>
    public double KnobRemainder { get; private set; }

    private bool _touching;
    private readonly List<(double Time, RingSample Sample)> _history = [];
    private (double Time, RingSample Sample)? _touchStart;
    private bool _touchMovedFar;
    private double? _knobReference;

    public IReadOnlyList<RingEffect> Handle(RingEvent ev, double time)
    {
        switch (ev)
        {
            case RingEvent.Opened:
                Reset(new RingMode(RingModeKind.Selecting));
                return [new RingEffect.Show()];
            case RingEvent.Closed:
            {
                var wasOpen = Mode.Kind != RingModeKind.Closed;
                Reset(RingMode.Closed);
                return wasOpen ? [new RingEffect.Hide()] : [];
            }
            case RingEvent.Position(var s):
                Finger = s;
                return Mode.Kind switch
                {
                    RingModeKind.Selecting => Select(s, Mode.Item, time),
                    RingModeKind.Knob => TurnKnob(s, Mode.Item!.Value, time),
                    _ => [],
                };
            default:
                // 設定のイベントはリングの状態と関係ない
                return [];
        }
    }

    /// <summary>アプリ側から閉じる（メニューなど）。</summary>
    public IReadOnlyList<RingEffect> Close()
    {
        if (Mode.Kind == RingModeKind.Closed) return [];
        Reset(RingMode.Closed);
        return [new RingEffect.SendClose(), new RingEffect.Hide()];
    }

    private IReadOnlyList<RingEffect> Select(RingSample s, int? hover, double time)
    {
        if (s.Touching)
        {
            _touching = true;
            _history.Add((time, s));
            _history.RemoveAll(h => time - h.Time > 0.5);
            Mode = new RingMode(RingModeKind.Selecting, ItemAt(s, hover));
            return [];
        }
        // 離すと「触れていない」が 5 回続けて来る。最初の 1 回だけを「離した」とする
        if (!_touching) return [];
        _touching = false;
        var released = ReleasedSample();
        var chosen = released is { } r ? ItemAt(r, hover) : null;
        _history.Clear();
        if (chosen is not { } c || c >= Actions.Count || Actions[c].Kind == ActionKind.None)
        {
            Reset(RingMode.Closed);
            return [new RingEffect.SendClose(), new RingEffect.Hide()];
        }
        if (Actions[c].IsKnob)
        {
            Reset(new RingMode(RingModeKind.Knob, c));
            return [new RingEffect.EnterKnob(c)];
        }
        Reset(RingMode.Closed);
        return [new RingEffect.Run(c), new RingEffect.SendClose(), new RingEffect.Hide()];
    }

    /// <summary>離す直前は位置がぶれるので、最後に触れていた時刻から少し前の位置を使う。</summary>
    private RingSample? ReleasedSample()
    {
        if (_history.Count == 0) return null;
        var target = _history[^1].Time - Tuning.ReleaseLookbackMs / 1000;
        for (var i = _history.Count - 1; i >= 0; i--)
        {
            if (_history[i].Time <= target) return _history[i].Sample;
        }
        return _history[0].Sample;
    }

    /// <summary>位置 → 項目（キャンセル範囲なら null）。previous の項目は、境目をヒステリシス分越えるまで保つ。</summary>
    public int? ItemAt(RingSample s, int? previous)
    {
        if (s.Radius < Tuning.CancelRadius) return null;
        var width = 360.0 / RingConfig.SlotCount;
        if (previous is { } p && Math.Abs(AngleDelta(p * width, s.Angle)) <= width / 2 + Tuning.HysteresisDegrees)
        {
            return p;
        }
        return (int)Math.Floor((s.Angle + width / 2) / width) % RingConfig.SlotCount;
    }

    private IReadOnlyList<RingEffect> TurnKnob(RingSample s, int item, double time)
    {
        if (!s.Touching)
        {
            if (!_touching) return [];
            _touching = false;
            _knobReference = null;
            // 中心を短くタップしたら抜ける
            if (_touchStart is { } start && start.Sample.Radius < Tuning.CancelRadius
                && time - start.Time <= Tuning.TapMaxMs / 1000 && !_touchMovedFar)
            {
                Reset(RingMode.Closed);
                return [new RingEffect.SendClose(), new RingEffect.Hide()];
            }
            return [];
        }
        if (!_touching)
        {
            // 触れ直した: 離していた間の角度の差は回転に数えない
            _touching = true;
            _touchStart = (time, s);
            _touchMovedFar = false;
            _knobReference = null;
        }
        else if (_touchStart is { } st && Distance(st.Sample, s) > Tuning.TapMaxMove)
        {
            _touchMovedFar = true;
        }
        // 中心付近では角度が暴れるので数えない
        if (s.Radius < Tuning.CancelRadius)
        {
            _knobReference = null;
            return [];
        }
        var reference = _knobReference;
        _knobReference = s.Angle;
        if (reference is not { } refAngle) return [];
        KnobRemainder += AngleDelta(refAngle, s.Angle);
        var steps = (int)(KnobRemainder / Tuning.KnobStepDegrees); // 0 に向かって切り捨て
        if (steps == 0) return [];
        KnobRemainder -= steps * Tuning.KnobStepDegrees;
        return [new RingEffect.KnobStep(item, steps)];
    }

    private void Reset(RingMode mode)
    {
        Mode = mode;
        _touching = false;
        _history.Clear();
        _touchStart = null;
        _touchMovedFar = false;
        _knobReference = null;
        KnobRemainder = 0;
        if (mode.Kind == RingModeKind.Closed) Finger = null;
    }

    /// <summary>from → to の角度の差（-180〜180。右回りが +）。0°/360° をまたいでも正しく出す。</summary>
    public static double AngleDelta(double from, double to)
    {
        var d = (to - from + 540) % 360;
        return (d < 0 ? d + 360 : d) - 180;
    }

    /// <summary>パッド上の 2 点の距離（縁 ≒ 1000 の単位）。</summary>
    public static double Distance(RingSample a, RingSample b)
    {
        static (double X, double Y) P(RingSample s)
        {
            var t = s.Angle * Math.PI / 180;
            return (s.Radius * Math.Sin(t), s.Radius * Math.Cos(t));
        }
        var (ax, ay) = P(a);
        var (bx, by) = P(b);
        return Math.Sqrt((ax - bx) * (ax - bx) + (ay - by) * (ay - by));
    }
}
