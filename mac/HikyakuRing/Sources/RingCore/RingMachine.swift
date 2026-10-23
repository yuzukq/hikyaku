// リングの状態遷移: hikyaku からのイベント → 表示の状態と、アプリがすること（実行・閉じる・ノブの 1 段階）。
//
// 時刻は呼び出し側から渡す（秒。単調増加なら起点は問わない）。テストで時間を進められるように。

import Foundation

public enum RingEffect: Equatable, Sendable {
    /// リングを表示する（カーソルの位置付近）
    case show
    /// 表示を消す
    case hide
    /// 普通の項目を実行する
    case run(item: Int)
    /// ノブ操作モードに入った（表示を切り替える。hikyaku のリングは開いたまま）
    case enterKnob(item: Int)
    /// ノブを回した（+ = 右回り）
    case knobStep(item: Int, steps: Int)
    /// hikyaku に「閉じる」（01）を送る
    case sendClose
}

public enum RingMode: Equatable, Sendable {
    case closed
    /// 項目を選んでいる。hover = 指の方向の項目（キャンセル範囲や触れていなければ nil）
    case selecting(hover: Int?)
    case knob(item: Int)
}

public struct RingMachine: Sendable {
    public var tuning: RingTuning
    /// 8 か所それぞれの割り当て（ノブの項目か・空きかの判定に使う）
    public var actions: [RingAction]

    public private(set) var mode: RingMode = .closed
    /// 最後に受け取った指の状態（表示用）
    public private(set) var finger: RingSample?
    /// ノブ: 右回りを + とした、積算中の回転（度。1 段階に満たない端数）
    public private(set) var knobRemainder: Double = 0

    private var touching = false
    /// 離した位置を判定するための、少し前までの位置（時刻つき）
    private var history: [(time: Double, sample: RingSample)] = []
    // ノブ: 今の接触の始まりと、回転の基準
    private var touchStart: (time: Double, sample: RingSample)?
    private var touchMovedFar = false
    private var knobReference: Double?

    public init(tuning: RingTuning = RingTuning(), actions: [RingAction]) {
        self.tuning = tuning
        self.actions = actions
    }

    public mutating func handle(_ event: RingEvent, at time: Double) -> [RingEffect] {
        switch event {
        case .opened:
            // 開いたまま F23 などで開き直されたときも、選び直しから始める
            reset(to: .selecting(hover: nil))
            return [.show]
        case .closed:
            // hikyaku が自分で閉じた（F23・Esc・5 秒触れない）。ノブモードの途中でも閉じる
            let wasOpen = mode != .closed
            reset(to: .closed)
            return wasOpen ? [.hide] : []
        case .settings:
            // リングの状態とは関係ない（設定画面が使う）
            return []
        case .position(let s):
            finger = s
            switch mode {
            case .closed: return []
            case .selecting(let hover): return select(s, hover: hover, at: time)
            case .knob(let item): return turnKnob(s, item: item, at: time)
            }
        }
    }

    /// アプリ側から閉じる（メニューなど）。
    public mutating func close() -> [RingEffect] {
        guard mode != .closed else { return [] }
        reset(to: .closed)
        return [.sendClose, .hide]
    }

    // MARK: - 項目の選択

    private mutating func select(_ s: RingSample, hover: Int?, at time: Double) -> [RingEffect] {
        if s.touching {
            touching = true
            history.append((time, s))
            history.removeAll { time - $0.time > 0.5 }
            mode = .selecting(hover: item(at: s, previous: hover))
            return []
        }
        // 離すと「触れていない」が 5 回続けて来る。最初の 1 回だけを「離した」とする
        guard touching else { return [] }
        touching = false
        let chosen = releasedSample().flatMap { item(at: $0, previous: hover) }
        history.removeAll()
        guard let chosen, actions.indices.contains(chosen), actions[chosen] != .none else {
            reset(to: .closed)
            return [.sendClose, .hide]
        }
        if actions[chosen].isKnob {
            reset(to: .knob(item: chosen))
            return [.enterKnob(item: chosen)]
        }
        reset(to: .closed)
        return [.run(item: chosen), .sendClose, .hide]
    }

    /// 離す直前は位置がぶれるので、最後に触れていた時刻から少し前の位置を使う。
    private func releasedSample() -> RingSample? {
        guard let last = history.last else { return nil }
        let target = last.time - tuning.releaseLookbackMs / 1000
        return history.last(where: { $0.time <= target })?.sample ?? history.first?.sample
    }

    /// 位置 → 項目（キャンセル範囲なら nil）。previous の項目は、境目をヒステリシス分越えるまで保つ。
    public func item(at s: RingSample, previous: Int?) -> Int? {
        guard s.radius >= tuning.cancelRadius else { return nil }
        let width = 360.0 / Double(RingConfig.slotCount)
        if let previous {
            let center = Double(previous) * width
            if abs(RingMachine.angleDelta(from: center, to: s.angle)) <= width / 2 + tuning.hysteresisDegrees {
                return previous
            }
        }
        return Int(((s.angle + width / 2) / width).rounded(.down)) % RingConfig.slotCount
    }

    // MARK: - ノブ

    private mutating func turnKnob(_ s: RingSample, item: Int, at time: Double) -> [RingEffect] {
        guard s.touching else {
            guard touching else { return [] }
            touching = false
            knobReference = nil
            // 中心を短くタップしたら抜ける
            if let start = touchStart, start.sample.radius < tuning.cancelRadius,
               time - start.time <= tuning.tapMaxMs / 1000, !touchMovedFar {
                reset(to: .closed)
                return [.sendClose, .hide]
            }
            return []
        }
        if !touching {
            // 触れ直した: 離していた間の角度の差は回転に数えない
            touching = true
            touchStart = (time, s)
            touchMovedFar = false
            knobReference = nil
        } else if let start = touchStart, RingMachine.distance(start.sample, s) > Double(tuning.tapMaxMove) {
            touchMovedFar = true
        }
        // 中心付近では角度が暴れるので数えない
        guard s.radius >= tuning.cancelRadius else {
            knobReference = nil
            return []
        }
        defer { knobReference = s.angle }
        guard let reference = knobReference else { return [] }
        knobRemainder += RingMachine.angleDelta(from: reference, to: s.angle)
        let steps = Int(knobRemainder / tuning.knobStepDegrees) // 0 に向かって切り捨て
        guard steps != 0 else { return [] }
        knobRemainder -= Double(steps) * tuning.knobStepDegrees
        return [.knobStep(item: item, steps: steps)]
    }

    private mutating func reset(to mode: RingMode) {
        self.mode = mode
        touching = false
        history.removeAll()
        touchStart = nil
        touchMovedFar = false
        knobReference = nil
        knobRemainder = 0
        if mode == .closed { finger = nil }
    }

    // MARK: - 角度の計算

    /// from → to の角度の差（-180〜180。右回りが +）。0°/360° をまたいでも正しく出す。
    public static func angleDelta(from: Double, to: Double) -> Double {
        let d = (to - from + 540).truncatingRemainder(dividingBy: 360)
        return (d < 0 ? d + 360 : d) - 180
    }

    /// パッド上の 2 点の距離（縁 ≒ 1000 の単位）。
    static func distance(_ a: RingSample, _ b: RingSample) -> Double {
        func point(_ s: RingSample) -> (Double, Double) {
            let t = s.angle * .pi / 180
            return (Double(s.radius) * sin(t), Double(s.radius) * cos(t))
        }
        let (ax, ay) = point(a), (bx, by) = point(b)
        return ((ax - bx) * (ax - bx) + (ay - by) * (ay - by)).squareRoot()
    }
}
