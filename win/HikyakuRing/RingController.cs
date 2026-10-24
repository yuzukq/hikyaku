// hikyaku からのイベントを RingMachine に通し、表示・実行・hikyaku への命令につなぐ（UI のスレッドで動く）。

using System;
using System.Linq;
using System.Windows;
using System.Windows.Threading;
using Hikyaku.RingCore;

namespace Hikyaku.Ring;

public sealed class RingController : IDisposable
{
    /// <summary>表示を描き直すべき変化があった（リング・ノブ・設定画面が聞く）。</summary>
    public event Action? Changed;

    private RingConfig _config;
    private readonly RingMachine _machine;
    private readonly RingLink _link;
    private readonly ActionRunner _actions;
    private OverlayWindow? _overlay;
    private SettingsWindow? _settings;
    private readonly DispatcherTimer _readoutRefresh;
    private KnobKind? _refreshKind;

    public RingController()
    {
        _config = ConfigStore.Load();
        _machine = new RingMachine(_config.Tuning, _config.Items.Select(i => i.Action).ToArray());
        _link = new RingLink(Application.Current.Dispatcher);
        _link.StatusChanged += s =>
        {
            Log.Info($"status: {s.Label}");
            if (!s.IsReady)
            {
                Rotation = null;
                Profile = null;
                ScrollInvert = null;
            }
            Changed?.Invoke();
        };
        _link.EventReceived += Handle;
        _actions = new ActionRunner(_link.Send);
        _readoutRefresh = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(300) };
        _readoutRefresh.Tick += async (_, _) =>
        {
            _readoutRefresh.Stop();
            if (_refreshKind is { } kind && Mode.Kind == RingModeKind.Knob)
            {
                KnobReadout = await _actions.ReadoutAsync(kind, _config.Tuning);
                Changed?.Invoke();
            }
        };
    }

    public void Start() => _link.Start();

    public RingConfig Config
    {
        get => _config;
        set
        {
            _config = value;
            _machine.Tuning = value.Tuning;
            _machine.Actions = value.Items.Select(i => i.Action).ToArray();
            try
            {
                ConfigStore.Save(value);
            }
            catch (Exception e)
            {
                Log.Error($"saving config failed: {e.Message}");
            }
            Changed?.Invoke();
        }
    }

    public LinkStatus Status => _link.Status;
    public RingMode Mode => _machine.Mode;
    public RingSample? Finger => _machine.Finger;
    /// <summary>ノブ: 1 段階に満たない回転（度）</summary>
    public double KnobRemainder => _machine.KnobRemainder;
    public KnobReadout? KnobReadout { get; private set; }

    /// <summary>hikyaku から届いた設定（つながっていない・ファームが古ければ null）</summary>
    public DeviceRotation? Rotation { get; private set; }
    public int? Profile { get; private set; }
    public bool? ScrollInvert { get; private set; }

    /// <summary>F23 と同じ（メニュー・ホットキー）。</summary>
    public void OpenRing() => _link.Send(RingCommand.Open);

    public void CloseRing() => Apply(_machine.Close());

    public void ToggleRing()
    {
        if (Mode.Kind == RingModeKind.Closed) OpenRing();
        else CloseRing();
    }

    public void ShowSettings()
    {
        _settings ??= new SettingsWindow(this);
        _settings.ShowAndActivate();
    }

    public void SetRotation(DeviceRotation r) => _link.Send(RingCommand.SetRotation(r));
    public void SetScrollInvert(bool on) => _link.Send(RingCommand.SetScrollInvert(on));
    /// <summary>送り先を切り替える。hikyaku は再起動して、もう一方の ID でつなぎ直す（この PC との接続は切れる）。</summary>
    public void SwitchProfile(int profile) => _link.Send(RingCommand.SwitchProfile(profile));
    public void RequestSettings() => _link.Send(RingCommand.GetSettings);

    private void Handle(RingEvent ev)
    {
        if (ev is RingEvent.Settings s)
        {
            Log.Info($"settings: rotation {s.Rotation}, profile {s.Profile + 1}, scroll invert {s.ScrollInvert}");
            Rotation = s.Rotation;
            Profile = s.Profile;
            ScrollInvert = s.ScrollInvert;
            Changed?.Invoke();
            return;
        }
        var effects = _machine.Handle(ev, Environment.TickCount64 / 1000.0);
        if (ev is not RingEvent.Position || effects.Count > 0)
        {
            Log.Info($"event {ev} -> {string.Join(", ", effects)}");
        }
        Apply(effects);
        Changed?.Invoke();
    }

    private async void Apply(System.Collections.Generic.IReadOnlyList<RingEffect> effects)
    {
        foreach (var effect in effects)
        {
            switch (effect)
            {
                case RingEffect.Show:
                    KnobReadout = null;
                    _overlay ??= new OverlayWindow(this);
                    _overlay.ShowNearCursor();
                    break;
                case RingEffect.Hide:
                    _readoutRefresh.Stop();
                    _overlay?.Hide();
                    break;
                case RingEffect.Run(var item):
                    var action = _config.Items[item].Action;
                    if (action.Kind == ActionKind.OpenSettings) ShowSettings();
                    else _actions.Run(action);
                    break;
                case RingEffect.EnterKnob(var item):
                    KnobReadout = await _actions.ReadoutAsync(_config.Items[item].Action.Knob, _config.Tuning);
                    break;
                case RingEffect.KnobStep(var item, var steps):
                    var kind = _config.Items[item].Action.Knob;
                    _actions.Turn(kind, steps, _config.Tuning);
                    KnobReadout = KnobReadout?.Stepped(steps);
                    // 回し終えてから本当の値を読み直す（回している間は見込みの値を出す）
                    _refreshKind = kind;
                    _readoutRefresh.Stop();
                    _readoutRefresh.Start();
                    break;
                case RingEffect.SendClose:
                    _link.Send(RingCommand.Close);
                    break;
            }
        }
        Changed?.Invoke();
    }

    public void Dispose()
    {
        _link.Dispose();
        _overlay?.Close();
    }
}
