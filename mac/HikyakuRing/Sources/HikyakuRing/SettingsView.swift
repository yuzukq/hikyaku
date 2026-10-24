// 設定画面: 8 か所の割り当てと、調整値（変更はすぐ config.json に保存する）、本体の向き（hikyaku に保存する）。

import AppKit
import RingCore
import SwiftUI

struct SettingsView: View {
    @Bindable var controller: RingController

    var body: some View {
        TabView {
            ItemsSettings(config: $controller.config)
                .tabItem { Label("項目", systemImage: "circle.grid.3x3") }
            TuningSettings(tuning: $controller.config.tuning, status: controller.status)
                .tabItem { Label("調整", systemImage: "slider.horizontal.3") }
            OrientationSettings(controller: controller)
                .tabItem { Label("本体の向き", systemImage: "rotate.right") }
            ProfileSettings(controller: controller)
                .tabItem { Label("送り先", systemImage: "arrow.left.arrow.right") }
        }
        .frame(width: 620, height: 420)
    }
}

private let directions = ["上", "右上", "右", "右下", "下", "左下", "左", "左上"]

private struct ItemsSettings: View {
    @Binding var config: RingConfig
    @State private var selection = 0

    var body: some View {
        HStack(spacing: 0) {
            List(0..<RingConfig.slotCount, id: \.self, selection: $selection) { i in
                let item = config.items[i]
                HStack {
                    Image(systemName: item.symbol)
                        .frame(width: 20)
                    VStack(alignment: .leading) {
                        Text(item.name.isEmpty ? "（空き）" : item.name)
                        Text(directions[i])
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .tag(i)
            }
            .frame(width: 200)

            ItemEditor(item: $config.items[selection])
                .id(selection)
        }
    }
}

/// アクションの種類（Picker 用）。
private enum ActionKind: String, CaseIterable, Identifiable {
    case none, keys, media, knob, settings
    var id: Self { self }

    var title: String {
        switch self {
        case .none: "なし"
        case .keys: "キーの組み合わせ"
        case .media: "メディアキー"
        case .knob: "連続値（ノブで調整）"
        case .settings: "設定画面を開く"
        }
    }

    init(_ action: RingAction) {
        switch action {
        case .none: self = .none
        case .keys: self = .keys
        case .media: self = .media
        case .knob: self = .knob
        case .openSettings: self = .settings
        }
    }

    var defaultAction: RingAction {
        switch self {
        case .none: .none
        case .keys: .keys(KeyCombo(modifiers: KeyCombo.command, usage: 0x2C))
        case .media: .media(.playPause)
        case .knob: .knob(.volume)
        case .settings: .openSettings
        }
    }
}

private struct ItemEditor: View {
    @Binding var item: RingItem

    var body: some View {
        Form {
            TextField("名前", text: $item.name)
            LabeledContent("アイコン") {
                HStack {
                    TextField("SF Symbols の名前", text: $item.symbol)
                        .labelsHidden()
                    Image(systemName: item.symbol)
                        .frame(width: 24)
                }
            }
            Picker("アクション", selection: kind) {
                ForEach(ActionKind.allCases) { Text($0.title).tag($0) }
            }
            switch item.action {
            case .none, .openSettings:
                EmptyView()
            case .keys(let combo):
                KeyComboEditor(combo: Binding(get: { combo }, set: { item.action = .keys($0) }))
            case .media(let key):
                Picker("キー", selection: Binding(get: { key }, set: { item.action = .media($0) })) {
                    ForEach(MediaKey.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            case .knob(let knob):
                Picker("調整するもの", selection: Binding(get: { knob }, set: { item.action = .knob($0) })) {
                    ForEach(KnobKind.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            }
        }
        .formStyle(.grouped)
    }

    private var kind: Binding<ActionKind> {
        Binding(
            get: { ActionKind(item.action) },
            set: { if $0 != ActionKind(item.action) { item.action = $0.defaultAction } }
        )
    }
}

/// キーの組み合わせ: 押して記録するか、修飾キーを切り替える（⌘Space のように Mac が先に取るキーは、キーだけ記録して修飾キーを足す）。
private struct KeyComboEditor: View {
    @Binding var combo: KeyCombo
    @State private var monitor: Any?

    var body: some View {
        LabeledContent("キー") {
            HStack {
                Text(combo.label)
                    .font(.body.monospaced())
                    .frame(minWidth: 80, alignment: .trailing)
                Button(monitor == nil ? "記録" : "キーを押してください…") {
                    monitor == nil ? startRecording() : stopRecording()
                }
            }
        }
        LabeledContent("修飾キー") {
            HStack {
                modifierToggle("⌃", KeyCombo.control)
                modifierToggle("⌥", KeyCombo.option)
                modifierToggle("⇧", KeyCombo.shift)
                modifierToggle("⌘", KeyCombo.command)
            }
        }
        .onDisappear(perform: stopRecording)
    }

    private func modifierToggle(_ title: String, _ bit: UInt8) -> some View {
        Toggle(title, isOn: Binding(
            get: { (combo.modifiers | combo.modifiers >> 4) & bit != 0 },
            set: { combo.modifiers = ($0 ? combo.modifiers | bit : combo.modifiers & ~bit) & ~(bit << 4) }
        ))
        .toggleStyle(.button)
    }

    private func startRecording() {
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard let usage = HIDKey.usage(macKeyCode: event.keyCode) else { return nil }
            // 左右の区別はせず、左側のビットにする
            var modifiers: UInt8 = 0
            let flags = event.modifierFlags
            if flags.contains(.control) { modifiers |= KeyCombo.control }
            if flags.contains(.shift) { modifiers |= KeyCombo.shift }
            if flags.contains(.option) { modifiers |= KeyCombo.option }
            if flags.contains(.command) { modifiers |= KeyCombo.command }
            combo = KeyCombo(modifiers: modifiers, usage: usage)
            stopRecording()
            return nil
        }
    }

    private func stopRecording() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }
}

private struct TuningSettings: View {
    @Binding var tuning: RingTuning
    let status: LinkStatus

    var body: some View {
        Form {
            Section("接続") {
                Text(status.label)
            }
            Section("選択") {
                Stepper("キャンセル範囲（中心からの距離、縁 = 1000）: \(tuning.cancelRadius)",
                        value: $tuning.cancelRadius, in: 100...800, step: 25)
                Stepper("境目のヒステリシス: \(Int(tuning.hysteresisDegrees))°",
                        value: $tuning.hysteresisDegrees, in: 0...20, step: 1)
                Stepper("離した位置の判定（何 ms 前の位置か）: \(Int(tuning.releaseLookbackMs)) ms",
                        value: $tuning.releaseLookbackMs, in: 0...200, step: 10)
            }
            Section("ノブ") {
                Stepper("1 段階の角度: \(Int(tuning.knobStepDegrees))°",
                        value: $tuning.knobStepDegrees, in: 5...60, step: 1)
                Stepper("終了のタップの長さ: \(Int(tuning.tapMaxMs)) ms 以内",
                        value: $tuning.tapMaxMs, in: 100...600, step: 25)
                Stepper("タップとみなす動き: \(tuning.tapMaxMove) 以内",
                        value: $tuning.tapMaxMove, in: 50...400, step: 25)
                Stepper("シークの 1 段階: \(Int(tuning.seekStepSeconds)) 秒",
                        value: $tuning.seekStepSeconds, in: 1...30, step: 1)
            }
            Section {
                Button("標準に戻す") { tuning = RingTuning() }
            }
        }
        .formStyle(.grouped)
    }
}

// MARK: - 本体の向き

/// hikyaku を置いている向きを、本体の絵で選ぶ。机の奥が上。選ぶと hikyaku に送り、hikyaku が保存して返事をくれる。
private struct OrientationSettings: View {
    let controller: RingController

    var body: some View {
        VStack(spacing: 20) {
            Text("hikyaku を置いている向きを選んでください（机の奥が上）。カーソル・スクロール・リングの向きが合います。")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            HStack(spacing: 14) {
                ForEach(DeviceRotation.allCases) { rotation in
                    RotationChoice(rotation: rotation, selected: controller.rotation == rotation) {
                        controller.setRotation(rotation)
                    }
                    .disabled(controller.rotation == nil)
                }
            }
            if controller.rotation == nil {
                Label(controller.status.isReady ? "hikyaku から向きが届いていません（ファームが古い可能性）" : controller.status.label,
                      systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.secondary)
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onAppear { controller.requestSettings() }
    }
}

private struct RotationChoice: View {
    let rotation: DeviceRotation
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 10) {
                HikyakuDrawing()
                    .rotationEffect(.degrees(rotation.degrees))
                    .frame(width: 104, height: 104)
                Text(rotation.title)
                    .font(.callout.weight(selected ? .semibold : .regular))
            }
            .padding(10)
            .background(
                RoundedRectangle(cornerRadius: 12)
                    .fill(selected ? Color.accentColor.opacity(0.15) : Color.secondary.opacity(0.06))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 12)
                    .strokeBorder(selected ? Color.accentColor : Color.secondary.opacity(0.25), lineWidth: selected ? 2 : 1)
            )
            .contentShape(RoundedRectangle(cornerRadius: 12))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("本体の向き: \(rotation.title)")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// hikyaku 本体の簡単な絵（基準の向き）: 青いトッププレート、中央に黒い円のパッド、左下にリセットボタン、右下にオレンジのキースイッチ。
/// 寸法は見た目の目安（縦 100 に対する比率）。
struct HikyakuDrawing: View {
    var body: some View {
        Canvas { ctx, size in
            let s = min(size.width, size.height) / 100
            let origin = CGPoint(x: (size.width - 80 * s) / 2, y: (size.height - 100 * s) / 2)
            func rect(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat) -> CGRect {
                CGRect(x: origin.x + x * s, y: origin.y + y * s, width: w * s, height: h * s)
            }
            // トッププレート
            let plate = Path(roundedRect: rect(0, 0, 80, 100), cornerRadius: 9 * s)
            ctx.fill(plate, with: .color(Color(red: 0.16, green: 0.40, blue: 0.86)))
            ctx.stroke(plate, with: .color(.black.opacity(0.25)), lineWidth: 1)
            // トラックパッド
            ctx.fill(Path(ellipseIn: rect(12, 12, 56, 56)), with: .color(Color(white: 0.08)))
            ctx.stroke(Path(ellipseIn: rect(12, 12, 56, 56)), with: .color(.white.opacity(0.15)), lineWidth: 1)
            // リセットボタン（左下・小さい）
            ctx.fill(Path(ellipseIn: rect(9, 84, 7, 7)), with: .color(Color(white: 0.85)))
            ctx.stroke(Path(ellipseIn: rect(9, 84, 7, 7)), with: .color(.black.opacity(0.4)), lineWidth: 0.8)
            // キースイッチ（右下・オレンジ。上から見た MX の軸の十字）
            ctx.fill(Path(roundedRect: rect(55, 76, 17, 17), cornerRadius: 2.5 * s), with: .color(Color(red: 0.98, green: 0.55, blue: 0.12)))
            let stem = Color(red: 0.80, green: 0.38, blue: 0.05)
            ctx.fill(Path(rect(62.5, 79.5, 2, 10)), with: .color(stem))
            ctx.fill(Path(rect(58.5, 83.5, 10, 2)), with: .color(stem))
        }
        .accessibilityHidden(true)
    }
}

// MARK: - 送り先

/// 送り先（1 台目 hikyaku / 2 台目 hikyaku2）の切り替えと、送り先ごとのスクロールの向き。どちらも hikyaku に保存される。
/// 仕様: docs/design/multi-host.md
private struct ProfileSettings: View {
    let controller: RingController

    var body: some View {
        Form {
            Section("接続") {
                Text(controller.status.label)
                if let profile = controller.profile {
                    LabeledContent("今の送り先", value: profile == 0 ? "1 台目（hikyaku）" : "2 台目（hikyaku2）")
                }
            }
            Section("送り先の切り替え") {
                HStack {
                    Button("1 台目（hikyaku）に切り替え") { controller.switchProfile(0) }
                        .disabled(controller.profile != 1)
                    Button("2 台目（hikyaku2）に切り替え") { controller.switchProfile(1) }
                        .disabled(controller.profile != 0)
                }
                Text("hikyaku は送り先ごとに別の機器として、1 台ずつつながります。切り替えると hikyaku は再起動して、もう一方の送り先につなぎ直します（数秒かかり、この Mac との接続は切れます）。キーボードの F21 / F22、または本体のボタンをパッドに触れずに 3 秒押し続けても切り替えられます。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Section("スクロール") {
                Toggle("この送り先でスクロールを反転する（Mac のナチュラルスクロールと同じ向き）", isOn: Binding(
                    get: { controller.scrollInvert ?? false },
                    set: { controller.setScrollInvert($0) }
                ))
                .disabled(controller.scrollInvert == nil)
            }
        }
        .formStyle(.grouped)
        .onAppear { controller.requestSettings() }
    }
}
