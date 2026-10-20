// 項目の実行。キーとメディアキーは hikyaku に頼んで本物のキーボード入力として送る（アクセシビリティの許可が要らない）。
// シークは Spotify・ミュージックを AppleScript で操作する（どのアプリでも効く MediaRemote は一般のアプリから使えない）。

import AppKit
import AudioToolbox
import CoreAudio
import RingCore

/// ノブの表示に使う、今の値。
struct KnobReadout: Equatable {
    var text: String
    /// 0〜1（分からなければ nil）
    var fraction: Double?
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
    func readout(_ kind: KnobKind) -> KnobReadout? {
        switch kind {
        case .volume:
            guard let v = SystemVolume.read() else { return nil }
            return KnobReadout(text: "\(Int((v * 100).rounded()))%", fraction: v)
        case .brightness:
            return nil
        case .seek:
            guard let player = MusicPlayer.current(), let (position, duration) = player.position() else {
                return KnobReadout(text: "再生中のアプリなし", fraction: nil)
            }
            let fraction = duration > 0 ? min(max(position / duration, 0), 1) : nil
            return KnobReadout(text: "\(clock(position)) / \(clock(duration))", fraction: fraction)
        }
    }

    private func clock(_ seconds: Double) -> String {
        let s = max(Int(seconds), 0)
        return String(format: "%d:%02d", s / 60, s % 60)
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
