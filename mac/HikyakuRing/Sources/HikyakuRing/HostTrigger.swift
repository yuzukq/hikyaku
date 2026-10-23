// Mac 側からリングを開閉する手段（7sPro をつながずに試すとき用。F23 と同じく hikyaku に 02 / 01 を送る）。
//
// - ⌃⌥R: 全体のホットキー（Carbon の RegisterEventHotKey。アクセシビリティの許可が要らない）
// - URL: `open -g hikyaku-ring://open`（close・toggle も。settings は設定画面）。ターミナルやスクリプトから
//
// F23 そのものは使えない: Mac で押したキーは hikyaku には届かない（ファームが F23 を見るのは 7sPro からの USB 入力だけ）うえ、
// Mac のキーボードには F23 がなく、Carbon にも F20 より上のキーコードがない。

import AppKit
import Carbon.HIToolbox

@MainActor
final class HostTrigger: NSObject, NSApplicationDelegate {
    let controller = RingController()
    private var hotKey: EventHotKeyRef?

    func applicationDidFinishLaunching(_ notification: Notification) {
        registerHotKey()
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls where url.scheme == "hikyaku-ring" {
            switch url.host {
            case "open": controller.openRing()
            case "close": controller.closeRing()
            case "toggle": controller.toggleRing()
            case "settings": controller.showSettings()
            default: log.error("unknown URL: \(url.absoluteString, privacy: .public)")
            }
        }
    }

    private func registerHotKey() {
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let me = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, _, me in
            let trigger = Unmanaged<HostTrigger>.fromOpaque(me!).takeUnretainedValue()
            MainActor.assumeIsolated { trigger.controller.toggleRing() }
            return noErr
        }, 1, &spec, me, nil)
        let id = EventHotKeyID(signature: OSType(0x484B_5247), id: 1) // 'HKRG'
        let status = RegisterEventHotKey(UInt32(kVK_ANSI_R), UInt32(controlKey | optionKey), id,
                                         GetApplicationEventTarget(), 0, &hotKey)
        if status != noErr {
            log.error("registering ⌃⌥R failed: \(status)")
        }
    }
}
