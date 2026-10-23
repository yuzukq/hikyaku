// hikyaku からのイベントを RingMachine に通し、表示・実行・hikyaku への命令につなぐ。

import AppKit
import Observation
import RingCore

@MainActor
@Observable
final class RingController {
    var config: RingConfig {
        didSet {
            machine.tuning = config.tuning
            machine.actions = config.items.map(\.action)
            do {
                try ConfigStore.save(config)
            } catch {
                log.error("saving config failed: \(error, privacy: .public)")
            }
        }
    }

    private(set) var status: LinkStatus = .starting
    private(set) var mode: RingMode = .closed
    private(set) var finger: RingSample?
    /// ノブ: 1 段階に満たない回転（度）。表示で、回している途中が分かるように
    private(set) var knobRemainder: Double = 0
    private(set) var knobReadout: KnobReadout?
    /// hikyaku 本体の置き方（hikyaku から届いた値）。つながっていない・ファームが古くて返事がないなら nil
    private(set) var rotation: DeviceRotation?

    @ObservationIgnored private var machine: RingMachine
    @ObservationIgnored private let link = RingLink()
    @ObservationIgnored private lazy var overlay = OverlayPanel(controller: self)
    @ObservationIgnored private lazy var settingsWindow = SettingsWindow(controller: self)
    @ObservationIgnored private lazy var actions = ActionRunner { [link] in link.send($0) }
    @ObservationIgnored private var readoutRefresh: DispatchWorkItem?

    init() {
        let config = ConfigStore.load()
        self.config = config
        machine = RingMachine(tuning: config.tuning, actions: config.items.map(\.action))
        link.onStatus = { [weak self] status in
            self?.status = status
            if !status.isReady { self?.rotation = nil }
        }
        link.onEvent = { [weak self] in self?.handle($0) }
        link.start()
    }

    /// F23 と同じ（アプリのメニューや試験用）。
    func openRing() {
        link.send(.open)
    }

    func toggleRing() {
        mode == .closed ? openRing() : closeRing()
    }

    func closeRing() {
        apply(machine.close())
    }

    func showSettings() {
        settingsWindow.show()
    }

    /// 本体の置き方を変える（hikyaku が保存して、settings のイベントで返す）。
    func setRotation(_ rotation: DeviceRotation) {
        link.send(.setRotation(rotation))
    }

    /// 今の設定を問い合わせる（HELLO の返事を取りこぼしたときや、設定画面を開いたとき）。
    func requestSettings() {
        link.send(.getSettings)
    }

    private func handle(_ event: RingEvent) {
        if case .settings(let rotation) = event {
            log.info("settings: rotation \(rotation.rawValue * 90)°")
            self.rotation = rotation
            return
        }
        let effects = machine.handle(event, at: ProcessInfo.processInfo.systemUptime)
        if case .position = event, effects.isEmpty {} else {
            log.info("event \(String(describing: event), privacy: .public) → \(String(describing: effects), privacy: .public)")
        }
        mode = machine.mode
        finger = machine.finger
        knobRemainder = machine.knobRemainder
        apply(effects)
    }

    private func apply(_ effects: [RingEffect]) {
        mode = machine.mode
        for effect in effects {
            switch effect {
            case .show:
                knobReadout = nil
                overlay.show(near: NSEvent.mouseLocation)
            case .hide:
                readoutRefresh?.cancel()
                overlay.hide()
            case .run(let item):
                if config.items[item].action == .openSettings {
                    showSettings()
                } else {
                    actions.run(config.items[item].action)
                }
            case .enterKnob(let item):
                if case .knob(let kind) = config.items[item].action {
                    knobReadout = actions.readout(kind, tuning: config.tuning)
                }
            case .knobStep(let item, let steps):
                if case .knob(let kind) = config.items[item].action {
                    actions.turn(kind, steps: steps, tuning: config.tuning)
                    knobReadout = knobReadout?.stepped(steps)
                    refreshReadoutSoon(kind)
                }
            case .sendClose:
                link.send(.close)
            }
        }
    }

    /// 回し終えてから本当の値を読み直す（メディアキーは hikyaku 経由で少し遅れて効く。回している間は見込みの値を出す）。
    private func refreshReadoutSoon(_ kind: KnobKind) {
        readoutRefresh?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self, case .knob = self.mode else { return }
            self.knobReadout = self.actions.readout(kind, tuning: self.config.tuning)
        }
        readoutRefresh = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }
}
