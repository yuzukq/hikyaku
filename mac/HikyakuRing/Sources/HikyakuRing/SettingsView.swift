// 設定画面: 8 か所の割り当てと、調整値。変更はすぐ config.json に保存する。

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
    case none, keys, media, knob
    var id: Self { self }

    var title: String {
        switch self {
        case .none: "なし"
        case .keys: "キーの組み合わせ"
        case .media: "メディアキー"
        case .knob: "連続値（ノブで調整）"
        }
    }

    init(_ action: RingAction) {
        switch action {
        case .none: self = .none
        case .keys: self = .keys
        case .media: self = .media
        case .knob: self = .knob
        }
    }

    var defaultAction: RingAction {
        switch self {
        case .none: .none
        case .keys: .keys(KeyCombo(modifiers: KeyCombo.command, usage: 0x2C))
        case .media: .media(.playPause)
        case .knob: .knob(.volume)
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
            case .none:
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
