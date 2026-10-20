// リングとノブの表示。角度は 0° = 上・時計回り（hikyaku から届く角度と同じ）。

import RingCore
import SwiftUI

struct RingOverlayView: View {
    let controller: RingController

    /// リングの外周の半径（パッドの縁 = 距離 1000 に当たる）
    private let outer: CGFloat = 180

    var body: some View {
        ZStack {
            Circle()
                .fill(.regularMaterial)
                .overlay(Circle().strokeBorder(.white.opacity(0.18), lineWidth: 1))
                .frame(width: outer * 2, height: outer * 2)
                .shadow(color: .black.opacity(0.25), radius: 16, y: 6)

            switch controller.mode {
            case .knob(let item):
                KnobView(item: controller.config.items[item], readout: controller.knobReadout,
                         remainder: controller.knobRemainder, stepDegrees: controller.config.tuning.knobStepDegrees,
                         cancelRadius: cancelZoneRadius, outer: outer, finger: controller.finger)
            default:
                selectionRing
            }
        }
        .frame(width: OverlayPanel.size, height: OverlayPanel.size)
        .environment(\.colorScheme, .dark)
    }

    private var hover: Int? {
        if case .selecting(let hover) = controller.mode { hover } else { nil }
    }

    private var cancelZoneRadius: CGFloat {
        outer * CGFloat(controller.config.tuning.cancelRadius) / 1000
    }

    private var fingerInCancelZone: Bool {
        guard let f = controller.finger, f.touching else { return false }
        return f.radius < controller.config.tuning.cancelRadius
    }

    private var selectionRing: some View {
        ZStack {
            // キャンセル範囲
            Circle()
                .fill(fingerInCancelZone ? Color.white.opacity(0.22) : Color.white.opacity(0.06))
                .frame(width: cancelZoneRadius * 2, height: cancelZoneRadius * 2)
            Image(systemName: "xmark")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(fingerInCancelZone ? .primary : .tertiary)

            ForEach(0..<RingConfig.slotCount, id: \.self) { i in
                let item = controller.config.items[i]
                ItemBadge(item: item, highlighted: hover == i)
                    .offset(point(angle: Double(i) * 45, radius: outer * 0.68))
            }

            FingerDot(finger: controller.finger, outer: outer)
        }
        .animation(.snappy(duration: 0.15), value: hover)
    }
}

private struct ItemBadge: View {
    let item: RingItem
    let highlighted: Bool

    var body: some View {
        VStack(spacing: 3) {
            Image(systemName: item.symbol)
                .font(.system(size: 20, weight: .medium))
                .frame(width: 42, height: 42)
                .background(Circle().fill(highlighted ? Color.accentColor : Color.white.opacity(0.08)))
                .foregroundStyle(highlighted ? .white : .primary)
            Text(item.name)
                .font(.caption)
                .lineLimit(1)
                .frame(maxWidth: 96)
                .foregroundStyle(highlighted ? .primary : .secondary)
        }
        .opacity(item.action == .none ? 0.35 : 1)
        .scaleEffect(highlighted ? 1.12 : 1)
    }
}

private struct FingerDot: View {
    let finger: RingSample?
    let outer: CGFloat

    var body: some View {
        if let f = finger, f.touching {
            Circle()
                .fill(.white)
                .frame(width: 10, height: 10)
                .shadow(color: .black.opacity(0.4), radius: 2)
                .offset(point(angle: f.angle, radius: outer * min(CGFloat(f.radius) / 1000, 1.02)))
        }
    }
}

private struct KnobView: View {
    let item: RingItem
    let readout: KnobReadout?
    let remainder: Double
    let stepDegrees: Double
    let cancelRadius: CGFloat
    let outer: CGFloat
    let finger: RingSample?

    var body: some View {
        ZStack {
            // 今の値の円弧（値が分かるときだけ）
            Circle()
                .stroke(Color.white.opacity(0.1), lineWidth: 6)
                .frame(width: outer * 1.7, height: outer * 1.7)
            // 1 段階に満たない回転の分も足して、指に合わせて動かす
            if let fraction = readout?.fraction(partialSteps: remainder / stepDegrees) {
                Circle()
                    .trim(from: 0, to: fraction)
                    .stroke(Color.accentColor, style: StrokeStyle(lineWidth: 6, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .frame(width: outer * 1.7, height: outer * 1.7)
            }

            // 1 段階に満たない回転を、指の位置から伸びる短い円弧で見せる
            if let f = finger, f.touching, f.radius >= Int(cancelRadius / outer * 1000), remainder != 0 {
                let start = f.angle - remainder
                Circle()
                    .trim(from: 0, to: abs(remainder) / 360)
                    .stroke(Color.white.opacity(0.5), style: StrokeStyle(lineWidth: 3, lineCap: .round))
                    .rotationEffect(.degrees(min(start, f.angle) - 90))
                    .frame(width: outer * 1.92, height: outer * 1.92)
            }

            VStack(spacing: 6) {
                Image(systemName: item.symbol)
                    .font(.system(size: 30, weight: .medium))
                Text(item.name)
                    .font(.headline)
                if let text = readout?.text {
                    Text(text)
                        .font(.title3.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }

            Text("回して調整・中心をタップで終了")
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .offset(y: outer * 0.62)

            FingerDot(finger: finger, outer: outer)
        }
    }
}

/// 角度（0° = 上・時計回り）と半径 → 中心からのずれ（SwiftUI は下が +）。
private func point(angle: Double, radius: CGFloat) -> CGSize {
    let t = angle * .pi / 180
    return CGSize(width: radius * sin(t), height: -radius * cos(t))
}
