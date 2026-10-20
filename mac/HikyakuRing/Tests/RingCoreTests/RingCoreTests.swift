import Foundation
import Testing
@testable import RingCore

// MARK: - 通信の形式

@Test func parsesPositionEvent() {
    // ring.rs の encodes_events と同じバイト列
    let e = RingEvent(Data([3, 1, 0x28, 0x23, 0x20, 0x03, 0x23, 0x01, 0x56, 0x04, 12, 7]))
    #expect(e == .position(RingSample(touching: true, angle: 90, radius: 800, x: 0x123, y: 0x456, z: 12, seq: 7)))
}

@Test func parsesOpenCloseAndRejectsShortData() {
    #expect(RingEvent(Data([1] + Array(repeating: 0, count: 11))) == .opened)
    #expect(RingEvent(Data([2] + Array(repeating: 0, count: 11))) == .closed)
    #expect(RingEvent(Data([3, 1, 0])) == nil)
    #expect(RingEvent(Data([9] + Array(repeating: 0, count: 11))) == nil)
}

@Test func encodesCommands() {
    #expect(RingCommand.hello.bytes == [0x00])
    #expect(RingCommand.close.bytes == [0x01])
    #expect(RingCommand.open.bytes == [0x02])
    // ⇧⌘4（ring.rs の parses_commands と同じ）
    #expect(RingCommand.key(KeyCombo(modifiers: 0x0A, usage: 0x21)).bytes == [0x10, 0x0A, 0x21])
    #expect(RingCommand.consumer(ConsumerUsage.volumeUp).bytes == [0x11, 0xE9, 0x00])
}

// MARK: - キー

@Test func mapsMacKeyCodesToHIDUsages() {
    #expect(HIDKey.usage(macKeyCode: 0x00) == 0x04) // A
    #expect(HIDKey.usage(macKeyCode: 0x06) == 0x1D) // Z
    #expect(HIDKey.usage(macKeyCode: 0x12) == 0x1E) // 1
    #expect(HIDKey.usage(macKeyCode: 0x1D) == 0x27) // 0
    #expect(HIDKey.usage(macKeyCode: 0x31) == 0x2C) // Space
    #expect(HIDKey.usage(macKeyCode: 0x7E) == 0x52) // ↑
    #expect(HIDKey.usage(macKeyCode: 0x7A) == 0x3A) // F1
    #expect(HIDKey.usage(macKeyCode: 0x5A) == 0x6F) // F20
    #expect(HIDKey.usage(macKeyCode: 0x66) == 0x91) // 英数
    #expect(HIDKey.usage(macKeyCode: 0x3A) == nil) // Option だけ
}

@Test func labelsKeyCombos() {
    #expect(KeyCombo(modifiers: KeyCombo.command | KeyCombo.shift, usage: 0x22).label == "⇧⌘5")
    #expect(KeyCombo(modifiers: 0x80, usage: 0x2C).label == "⌘Space") // 右 Command
    #expect(KeyCombo(modifiers: 0, usage: 0).label == "なし")
}

// MARK: - 設定

@Test func configRoundTripsAndPadsToEightItems() throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent("config.json")
    var config = RingConfig.standard
    config.tuning.knobStepDegrees = 20
    try ConfigStore.save(config, to: url)
    #expect(ConfigStore.load(from: url) == config)

    let short = #"{"items":[{"name":"a","symbol":"star","action":{"none":{}}}]}"#
    try Data(short.utf8).write(to: url)
    let loaded = ConfigStore.load(from: url)
    #expect(loaded.items.count == 8)
    #expect(loaded.items[0].name == "a")
    #expect(loaded.tuning == RingTuning())
}

// MARK: - 選択

private func touch(_ angle: Double, _ radius: Int = 800) -> RingEvent {
    .position(RingSample(touching: true, angle: angle, radius: radius))
}

private let lift = RingEvent.position(RingSample(touching: false, angle: 0, radius: 0))

/// 指を動かしたあと離す（70Hz、離す 5 回）。
private func run(_ m: inout RingMachine, _ events: [RingEvent], from t0: Double = 0) -> [RingEffect] {
    var effects: [RingEffect] = []
    for (i, e) in events.enumerated() {
        effects += m.handle(e, at: t0 + Double(i) / 70)
    }
    return effects
}

private func machine() -> RingMachine {
    RingMachine(actions: RingConfig.standard.items.map(\.action))
}

@Test func picksSectorsClockwiseFromUp() {
    let m = machine()
    let at = { (a: Double) in m.item(at: RingSample(touching: true, angle: a, radius: 800), previous: nil) }
    #expect(at(0) == 0)
    #expect(at(350) == 0)
    #expect(at(22.4) == 0)
    #expect(at(22.6) == 1)
    #expect(at(90) == 2)
    #expect(at(315) == 7)
    #expect(m.item(at: RingSample(touching: true, angle: 90, radius: 349), previous: nil) == nil)
}

@Test func hysteresisKeepsTheCurrentItemNearTheBorder() {
    let m = machine()
    let s = { (a: Double) in RingSample(touching: true, angle: a, radius: 800) }
    #expect(m.item(at: s(26), previous: 0) == 0) // 境目 22.5° + 6° 以内
    #expect(m.item(at: s(29), previous: 0) == 1)
    #expect(m.item(at: s(340), previous: 0) == 0)
    #expect(m.item(at: s(19), previous: 1) == 1)
}

@Test func releaseRunsTheItemUnderTheFingerShortlyBefore() {
    var m = machine()
    // 右（90° = 項目 2）でずっと触れていて、離す瞬間だけ 45°（項目 1）にぶれる
    let events: [RingEvent] = [.opened] + Array(repeating: touch(90), count: 10) + [touch(45)] + Array(repeating: lift, count: 5)
    #expect(run(&m, events) == [.show, .run(item: 2), .sendClose, .hide])
    #expect(m.mode == .closed)
}

@Test func releaseInTheCenterCancels() {
    var m = machine()
    let events: [RingEvent] = [.opened, touch(90)] + Array(repeating: touch(90, 100), count: 8) + [lift, lift]
    #expect(run(&m, events) == [.show, .sendClose, .hide])
}

@Test func releaseOnAnEmptySlotCancels() {
    var m = RingMachine(actions: [.none] + Array(repeating: .media(.playPause), count: 7))
    #expect(run(&m, [.opened] + Array(repeating: touch(0), count: 6) + [lift]) == [.show, .sendClose, .hide])
}

@Test func closedFromTheFirmwareHidesTheRing() {
    var m = machine()
    #expect(run(&m, [.opened, touch(90), .closed]) == [.show, .hide])
    #expect(m.mode == .closed)
    #expect(m.handle(.closed, at: 1) == [])
}

// MARK: - ノブ

/// 項目 0（音量）を選んでノブモードに入る。
private func knobMachine() -> RingMachine {
    var m = machine()
    let effects = run(&m, [.opened] + Array(repeating: touch(0), count: 6) + Array(repeating: lift, count: 5))
    #expect(effects == [.show, .enterKnob(item: 0)])
    #expect(m.mode == .knob(item: 0))
    return m
}

@Test func knobCountsStepsAcrossZeroDegrees() {
    var m = knobMachine()
    // 340° → 20° へ右回り（0° をまたぐ）。40° = 15° × 2 + 端数 10°
    let effects = run(&m, [touch(340), touch(350), touch(0), touch(10), touch(20)], from: 10)
    #expect(effects.reduce(0) { n, e in if case .knobStep(_, let s) = e { n + s } else { n } } == 2)
    #expect(abs(m.knobRemainder - 10) < 0.001)
    // 左回りに 40° 戻すと、端数と合わせて -2
    let back = run(&m, [touch(10), touch(0), touch(340)], from: 11)
    #expect(back.reduce(0) { n, e in if case .knobStep(_, let s) = e { n + s } else { n } } == -2)
    #expect(m.mode == .knob(item: 0))
}

@Test func knobIgnoresTheJumpBetweenTouches() {
    var m = knobMachine()
    _ = run(&m, [touch(0), touch(5)], from: 10)
    // 離して 180° の位置に触れ直しても、回したことにならない
    #expect(run(&m, [lift, lift, touch(180), touch(185)], from: 11) == [])
}

@Test func tapInTheCenterLeavesKnobMode() {
    var m = knobMachine()
    #expect(run(&m, [touch(0, 100), touch(10, 120), lift, lift], from: 10) == [.sendClose, .hide])
    #expect(m.mode == .closed)
}

@Test func longPressOrDragInTheCenterDoesNotLeaveKnobMode() {
    var m = knobMachine()
    // 長く触れている（250ms 超え）
    #expect(run(&m, Array(repeating: touch(0, 100), count: 25) + [lift], from: 10) == [])
    // 中心から外へ動かした
    #expect(run(&m, [touch(0, 100), touch(0, 500), lift], from: 20) == [])
    #expect(m.mode == .knob(item: 0))
}
