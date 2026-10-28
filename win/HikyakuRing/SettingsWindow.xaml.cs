// 設定画面: 8 か所の割り当てと調整値（変更はすぐ config.json に保存する）、本体の向き・送り先・スクロールの向き（hikyaku に保存する）。

using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using Hikyaku.RingCore;
using Microsoft.Win32;

namespace Hikyaku.Ring;

public partial class SettingsWindow : Window
{
    private static readonly string[] Directions = ["上", "右上", "右", "右下", "下", "左下", "左", "左上"];
    private readonly RingController _controller;
    private int _selected;
    private bool _loading;
    private bool _recording;
    private readonly List<(DeviceRotation Rotation, Border Border)> _rotationChoices = [];

    public SettingsWindow(RingController controller)
    {
        _controller = controller;
        InitializeComponent();
        KindBox.ItemsSource = Enum.GetValues<ActionKind>().Select(k => k.Title()).ToList();
        MediaBox.ItemsSource = Enum.GetValues<MediaKey>().Select(k => k.Title()).ToList();
        KnobBox.ItemsSource = Enum.GetValues<KnobKind>().Select(k => k.Title()).ToList();
        BuildTuning();
        BuildRotation();
        AutostartBox.IsChecked = Autostart.Enabled;
        LogText.Text = $"版 {Log.Version}　ログ: {Log.PathName}";
        RefreshItems();
        ItemList.SelectedIndex = 0;
        controller.Changed += () => Dispatcher.BeginInvoke(RefreshDevice);
        RefreshDevice();
        Closing += (_, e) =>
        {
            // 閉じても作り直さずに隠すだけ（次に開くとき速い）
            e.Cancel = true;
            _recording = false;
            Hide();
        };
        PreviewKeyDown += OnPreviewKeyDown;
    }

    public void ShowAndActivate()
    {
        Show();
        if (WindowState == WindowState.Minimized) WindowState = WindowState.Normal;
        BringToFront();
        _controller.RequestSettings();
    }

    /// <summary>
    /// ほかのアプリの窓より前に出して、キー入力も受けられるようにする。リングから開くときは、使っている最中のアプリが前にいて
    /// （最後に入力を受けたのもそのアプリ）、Windows は Activate を断る。すでに開いている窓はそのアプリの裏に残る（タスクバーで点滅するだけ）。
    /// </summary>
    private void BringToFront()
    {
        // 一瞬だけ最前面にして、ほかのアプリの窓の上に出す（重なりの順番を変えるのは断られない）
        Topmost = true;
        Topmost = false;
        // 前にいるアプリのスレッドと入力をつないでいる間は、前に出るのを断られない
        var hwnd = new WindowInteropHelper(this).Handle;
        var foreground = NativeMethods.GetWindowThreadProcessId(NativeMethods.GetForegroundWindow(), out _);
        var mine = NativeMethods.GetCurrentThreadId();
        var attached = foreground != 0 && foreground != mine && NativeMethods.AttachThreadInput(mine, foreground, true);
        NativeMethods.SetForegroundWindow(hwnd);
        if (attached) NativeMethods.AttachThreadInput(mine, foreground, false);
        Activate();
    }

    private RingItem Item => _controller.Config.Items[_selected];

    private void Update(RingItem item)
    {
        _controller.Config = _controller.Config.WithItem(_selected, item);
        RefreshItems();
    }

    // MARK: 項目

    private void RefreshItems()
    {
        var keep = _selected;
        _loading = true;
        ItemList.Items.Clear();
        for (var i = 0; i < RingConfig.SlotCount; i++)
        {
            var it = _controller.Config.Items[i];
            var row = new StackPanel { Orientation = Orientation.Horizontal };
            row.Children.Add(new TextBlock
            {
                Text = it.Glyph, FontFamily = new FontFamily("Segoe Fluent Icons, Segoe MDL2 Assets"), FontSize = 18,
                Width = 28, VerticalAlignment = VerticalAlignment.Center,
            });
            var label = new StackPanel();
            label.Children.Add(new TextBlock { Text = string.IsNullOrEmpty(it.Name) ? "（空き）" : it.Name });
            label.Children.Add(new TextBlock { Text = Directions[i], FontSize = 11, Foreground = Brushes.Gray });
            row.Children.Add(label);
            ItemList.Items.Add(new ListBoxItem { Content = row, Padding = new Thickness(4) });
        }
        ItemList.SelectedIndex = keep;
        _loading = false;
    }

    private void OnItemSelected(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || ItemList.SelectedIndex < 0) return;
        _selected = ItemList.SelectedIndex;
        _recording = false;
        LoadEditor();
    }

    private void LoadEditor()
    {
        _loading = true;
        var it = Item;
        NameBox.Text = it.Name;
        GlyphBox.Text = string.Concat(it.Glyph.Select(ch => ((int)ch).ToString("X4")));
        GlyphPreview.Text = it.Glyph;
        KindBox.SelectedIndex = (int)it.Action.Kind;
        MediaBox.SelectedIndex = (int)it.Action.Media;
        KnobBox.SelectedIndex = (int)it.Action.Knob;
        KeysPanel.Visibility = it.Action.Kind == ActionKind.Keys ? Visibility.Visible : Visibility.Collapsed;
        MediaPanel.Visibility = it.Action.Kind == ActionKind.Media ? Visibility.Visible : Visibility.Collapsed;
        KnobPanel.Visibility = it.Action.Kind == ActionKind.Knob ? Visibility.Visible : Visibility.Collapsed;
        LoadKeys();
        _loading = false;
    }

    private void LoadKeys()
    {
        var combo = Item.Action.Keys;
        var m = (byte)(combo.Modifiers | combo.Modifiers >> 4);
        KeysLabel.Text = combo.Label;
        CtrlToggle.IsChecked = (m & KeyCombo.Ctrl) != 0;
        AltToggle.IsChecked = (m & KeyCombo.Alt) != 0;
        ShiftToggle.IsChecked = (m & KeyCombo.Shift) != 0;
        WinToggle.IsChecked = (m & KeyCombo.Win) != 0;
        RecordButton.Content = _recording ? "キーを押してください…" : "記録";
    }

    private void OnNameChanged(object sender, TextChangedEventArgs e)
    {
        if (!_loading) Update(Item with { Name = NameBox.Text });
    }

    private void OnGlyphChanged(object sender, TextChangedEventArgs e)
    {
        if (_loading) return;
        // 4 桁の 16 進（E767）か、文字そのもの
        var text = GlyphBox.Text.Trim();
        var glyph = text.Length == 4 && int.TryParse(text, NumberStyles.HexNumber, null, out var code) ? ((char)code).ToString() : text;
        GlyphPreview.Text = glyph;
        Update(Item with { Glyph = glyph });
    }

    private void OnKindChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || KindBox.SelectedIndex < 0) return;
        var kind = (ActionKind)KindBox.SelectedIndex;
        if (kind == Item.Action.Kind) return;
        Update(Item with { Action = RingAction.Default(kind) });
        LoadEditor();
    }

    private void OnMediaChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!_loading && MediaBox.SelectedIndex >= 0) Update(Item with { Action = RingAction.ForMedia((MediaKey)MediaBox.SelectedIndex) });
    }

    private void OnKnobChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!_loading && KnobBox.SelectedIndex >= 0) Update(Item with { Action = RingAction.ForKnob((KnobKind)KnobBox.SelectedIndex) });
    }

    private void OnModifierToggled(object sender, RoutedEventArgs e)
    {
        byte m = 0;
        if (CtrlToggle.IsChecked == true) m |= KeyCombo.Ctrl;
        if (AltToggle.IsChecked == true) m |= KeyCombo.Alt;
        if (ShiftToggle.IsChecked == true) m |= KeyCombo.Shift;
        if (WinToggle.IsChecked == true) m |= KeyCombo.Win;
        Update(Item with { Action = RingAction.ForKeys(Item.Action.Keys with { Modifiers = m }) });
        LoadKeys();
    }

    private void OnRecord(object sender, RoutedEventArgs e)
    {
        _recording = !_recording;
        LoadKeys();
    }

    private void OnPreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (!_recording) return;
        var key = e.Key == Key.System ? e.SystemKey : e.Key;
        // 修飾キーだけのときは、次のキーを待つ
        if (key is Key.LeftCtrl or Key.RightCtrl or Key.LeftAlt or Key.RightAlt or Key.LeftShift or Key.RightShift or Key.LWin or Key.RWin)
        {
            return;
        }
        e.Handled = true;
        if (HidKey.FromVirtualKey(KeyInterop.VirtualKeyFromKey(key)) is not { } usage) return;
        // 左右の区別はせず、左側のビットにする
        byte m = 0;
        var mods = Keyboard.Modifiers;
        if (mods.HasFlag(ModifierKeys.Control)) m |= KeyCombo.Ctrl;
        if (mods.HasFlag(ModifierKeys.Shift)) m |= KeyCombo.Shift;
        if (mods.HasFlag(ModifierKeys.Alt)) m |= KeyCombo.Alt;
        if (mods.HasFlag(ModifierKeys.Windows)) m |= KeyCombo.Win;
        _recording = false;
        Update(Item with { Action = RingAction.ForKeys(new KeyCombo(m, usage)) });
        LoadKeys();
    }

    // MARK: 調整

    private void BuildTuning()
    {
        void Row(string title, double min, double max, double step, Func<RingTuning, double> get, Func<RingTuning, double, RingTuning> set, string unit)
        {
            var label = new TextBlock { Margin = new Thickness(0, 8, 0, 2) };
            var slider = new Slider { Minimum = min, Maximum = max, TickFrequency = step, IsSnapToTickEnabled = true, Value = get(_controller.Config.Tuning) };
            void Show() => label.Text = $"{title}: {slider.Value:0.##}{unit}";
            slider.ValueChanged += (_, _) =>
            {
                Show();
                _controller.Config = _controller.Config with { Tuning = set(_controller.Config.Tuning, slider.Value) };
            };
            Show();
            TuningPanel.Children.Add(label);
            TuningPanel.Children.Add(slider);
        }
        Row("キャンセル範囲（中心からの距離、縁 = 1000）", 100, 800, 25, t => t.CancelRadius, (t, v) => t with { CancelRadius = (int)v }, "");
        Row("境目のヒステリシス", 0, 20, 1, t => t.HysteresisDegrees, (t, v) => t with { HysteresisDegrees = v }, "°");
        Row("離した位置の判定（何 ms 前の位置か）", 0, 200, 10, t => t.ReleaseLookbackMs, (t, v) => t with { ReleaseLookbackMs = v }, " ms");
        Row("ノブの 1 段階の角度", 5, 60, 1, t => t.KnobStepDegrees, (t, v) => t with { KnobStepDegrees = v }, "°");
        Row("ノブを抜けるタップの長さ", 100, 600, 25, t => t.TapMaxMs, (t, v) => t with { TapMaxMs = v }, " ms 以内");
        Row("タップとみなす動き", 50, 400, 25, t => t.TapMaxMove, (t, v) => t with { TapMaxMove = (int)v }, " 以内");
        Row("シークの 1 段階", 1, 30, 1, t => t.SeekStepSeconds, (t, v) => t with { SeekStepSeconds = v }, " 秒");
        Row("明るさの 1 段階", 1, 20, 1, t => t.BrightnessStepPercent, (t, v) => t with { BrightnessStepPercent = (int)v }, "%");
        var reset = new Button { Content = "標準に戻す", Margin = new Thickness(0, 16, 0, 0), HorizontalAlignment = HorizontalAlignment.Left, Padding = new Thickness(12, 2, 12, 2) };
        reset.Click += (_, _) =>
        {
            _controller.Config = _controller.Config with { Tuning = new RingTuning() };
            TuningPanel.Children.Clear();
            BuildTuning();
        };
        TuningPanel.Children.Add(reset);
    }

    // MARK: 本体の向き・送り先

    private void BuildRotation()
    {
        foreach (var rotation in Enum.GetValues<DeviceRotation>())
        {
            var drawing = new HikyakuDrawing { Width = 100, Height = 100, LayoutTransform = new RotateTransform(rotation.Degrees()) };
            var content = new StackPanel();
            content.Children.Add(drawing);
            content.Children.Add(new TextBlock { Text = rotation.Title(), HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 8, 0, 0) });
            var border = new Border
            {
                Child = content, Padding = new Thickness(10), Margin = new Thickness(6), CornerRadius = new CornerRadius(10),
                BorderThickness = new Thickness(1), Cursor = Cursors.Hand, Background = Brushes.Transparent,
            };
            var r = rotation;
            border.MouseLeftButtonUp += (_, _) =>
            {
                if (_controller.Rotation is not null) _controller.SetRotation(r);
            };
            _rotationChoices.Add((rotation, border));
            RotationPanel.Children.Add(border);
        }
    }

    private void RefreshDevice()
    {
        var accent = new SolidColorBrush(Color.FromRgb(0x2F, 0x7C, 0xF6));
        foreach (var (rotation, border) in _rotationChoices)
        {
            var selected = _controller.Rotation == rotation;
            border.BorderBrush = selected ? accent : Brushes.LightGray;
            border.BorderThickness = new Thickness(selected ? 2 : 1);
            border.Background = selected ? new SolidColorBrush(Color.FromArgb(0x26, 0x2F, 0x7C, 0xF6)) : Brushes.Transparent;
            border.IsEnabled = _controller.Rotation is not null;
        }
        var status = _controller.Status;
        RotationNote.Text = _controller.Rotation is not null ? ""
            : status.IsReady ? "hikyaku から設定が届いていません（ファームが古い可能性）" : status.Label;
        StatusText.Text = status.Label;
        ProfileText.Text = _controller.Profile is { } p
            ? $"今の送り先: {p + 1} 台目（{(p == 0 ? "hikyaku" : "hikyaku2")}）"
            : "今の送り先: 不明（hikyaku とつながっていない）";
        Profile1Button.IsEnabled = _controller.Profile is 1;
        Profile2Button.IsEnabled = _controller.Profile is 0;
        ScrollInvertBox.IsEnabled = _controller.ScrollInvert is not null;
        ScrollInvertBox.IsChecked = _controller.ScrollInvert == true;
    }

    private void OnSwitchProfile1(object sender, RoutedEventArgs e) => _controller.SwitchProfile(0);
    private void OnSwitchProfile2(object sender, RoutedEventArgs e) => _controller.SwitchProfile(1);

    private void OnScrollInvertClicked(object sender, RoutedEventArgs e) => _controller.SetScrollInvert(ScrollInvertBox.IsChecked == true);

    private void OnAutostartClicked(object sender, RoutedEventArgs e)
    {
        Autostart.Enabled = AutostartBox.IsChecked == true;
        AutostartBox.IsChecked = Autostart.Enabled;
    }
}

/// <summary>Windows の起動時に開始する（HKCU の Run）。</summary>
internal static class Autostart
{
    private const string Key = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string Name = "HikyakuRing";

    public static bool Enabled
    {
        get
        {
            using var key = Registry.CurrentUser.OpenSubKey(Key);
            return key?.GetValue(Name) is string;
        }
        set
        {
            try
            {
                using var key = Registry.CurrentUser.CreateSubKey(Key);
                if (value) key.SetValue(Name, $"\"{Environment.ProcessPath}\"");
                else key.DeleteValue(Name, false);
            }
            catch (Exception e)
            {
                Log.Error($"autostart: {e.Message}");
            }
        }
    }
}
