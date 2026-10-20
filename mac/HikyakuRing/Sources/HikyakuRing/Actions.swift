// 項目の実行。キーとメディアキーは hikyaku に頼んで本物のキーボード入力として送る（アクセシビリティの許可が要らない）。
// シークは Spotify・ミュージックを AppleScript で操作する（どのアプリでも効く MediaRemote は一般のアプリから使えない）。

import AppKit
import AudioToolbox
import CoreAudio
import RingCore

/// ノブの表示に使う、今の値。
///
/// 回すたびに読み直すと、hikyaku 経由のキーが効くまで待つことになり表示が指に遅れる。
/// そこで 1 段階ごとに見込みの値へすぐ進め（`stepped`）、回し終えてから本当の値を読み直す。
struct KnobReadout: Equatable {
    /// 音量は 0〜1、シークは秒
    var value: Double
    /// 0 なら値を出さない（message だけ）
    var maximum: Double
    /// 1 段階で変わる量
    var step: Double
    var isClock = false
    var message: String?

    /// 0〜1（分からなければ nil）。partialSteps は 1 段階に満たない回転（段階の何割か）で、ゲージを指に合わせて動かす
    func fraction(partialSteps: Double = 0) -> Double? {
        guard maximum > 0 else { return nil }
        return min(max((value + partialSteps * step) / maximum, 0), 1)
    }

    var text: String {
        if let message { return message }
        if isClock { return "\(clock(value)) / \(clock(maximum))" }
        return "\(Int((value / maximum * 100).rounded()))%"
    }

    func stepped(_ steps: Int) -> KnobReadout {
        var r = self
        r.value = min(max(value + Double(steps) * step, 0), maximum)
        return r
    }

    private func clock(_ seconds: Double) -> String {
        let s = max(Int(seconds), 0)
        return String(format: "%d:%02d", s / 60, s % 60)
    }
}

@MainActor
struct ActionRunner {
    let send: (RingCommand) -> Void

    func run(_ action: RingAction) {
        switch action {
        case .none, .knob: break
        case .keys(let combo): send(.key(combo))
        case .media(let key): send(.consumer(key.usage))
        }
    }

    func turn(_ kind: KnobKind, steps: Int, tuning: RingTuning) {
        switch kind {
        case .volume:
            press(steps > 0 ? ConsumerUsage.volumeUp : ConsumerUsage.volumeDown, times: abs(steps))
        case .brightness:
            press(steps > 0 ? ConsumerUsage.brightnessUp : ConsumerUsage.brightnessDown, times: abs(steps))
        case .seek:
            MusicPlayer.current()?.seek(by: Double(steps) * tuning.seekStepSeconds)
        }
    }

    private func press(_ usage: UInt16, times: Int) {
        for _ in 0..<times {
            send(.consumer(usage))
        }
    }

    /// 今の値（音量・再生位置）。明るさは読めないので nil。
    func readout(_ kind: KnobKind, tuning: RingTuning) -> KnobReadout? {
        switch kind {
        case .volume:
            // 音量キー 1 回 = 1/16
            guard let v = SystemVolume.read() else { return nil }
            return KnobReadout(value: v, maximum: 1, step: 1.0 / 16)
        case .brightness:
            return nil
        case .seek:
            guard let player = MusicPlayer.current(), let (position, duration) = player.position() else {
                return KnobReadout(value: 0, maximum: 0, step: 0, message: "再生中のアプリなし")
            }
            return KnobReadout(value: position, maximum: duration, step: tuning.seekStepSeconds, isClock: true)
        }
    }
}

/// 既定の出力機器の音量（0〜1）。
enum SystemVolume {
    static func read() -> Double? {
        var device = AudioObjectID(0)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device) == noErr
        else { return nil }
        var volume = Float32(0)
        size = UInt32(MemoryLayout<Float32>.size)
        address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwareServiceDeviceProperty_VirtualMainVolume,
            mScope: kAudioDevicePropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain
        )
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &volume) == noErr else { return nil }
        return Double(volume)
    }
}

/// Spotify・ミュージックの再生位置。`tell application` は対象が動いていないと起動してしまうので、動いているものだけを使う。
@MainActor
struct MusicPlayer {
    let bundleID: String
    /// Spotify の duration はミリ秒、ミュージックは秒
    let durationScale: Double

    private static let known = [
        MusicPlayer(bundleID: "com.spotify.client", durationScale: 1000),
        MusicPlayer(bundleID: "com.apple.Music", durationScale: 1),
    ]

    /// 動いているもののうち、再生中のものを優先する。
    static func current() -> MusicPlayer? {
        let running = known.filter { !NSRunningApplication.runningApplications(withBundleIdentifier: $0.bundleID).isEmpty }
        return running.first(where: \.isPlaying) ?? running.first
    }

    var isPlaying: Bool {
        run("return (player state is playing)")?.booleanValue ?? false
    }

    func seek(by seconds: Double) {
        _ = run("set player position to (player position + (\(seconds)))")
    }

    func position() -> (Double, Double)? {
        guard let list = run("return {player position, duration of current track}"), list.numberOfItems == 2,
              let position = list.atIndex(1)?.doubleValue, let duration = list.atIndex(2)?.doubleValue
        else { return nil }
        return (position, duration / durationScale)
    }

    private func run(_ body: String) -> NSAppleEventDescriptor? {
        let source = "tell application id \"\(bundleID)\"\n\(body)\nend tell"
        var error: NSDictionary?
        let result = NSAppleScript(source: source)?.executeAndReturnError(&error)
        if let error {
            log.error("AppleScript (\(bundleID, privacy: .public)): \(error, privacy: .public)")
        }
        return result
    }
}
