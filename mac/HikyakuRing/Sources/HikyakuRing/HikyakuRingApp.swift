// hikyaku のショートカットリング: メニューバーに常駐し、F23 でリングを出す。仕様: docs/design/ring.md

import RingCore
import SwiftUI

@main
struct HikyakuRingApp: App {
    // URL と全体のホットキーを受けるために AppDelegate を使う。controller もそこで持つ
    @NSApplicationDelegateAdaptor private var trigger: HostTrigger
    private var controller: RingController { trigger.controller }

    var body: some Scene {
        MenuBarExtra {
            MenuContent(controller: controller)
        } label: {
            Image(systemName: controller.status.isReady ? "circle.circle.fill" : "circle.circle")
        }
        // 設定画面は AppKit のウィンドウ（SettingsWindow）。リングの項目からも開けるように
    }
}

private struct MenuContent: View {
    let controller: RingController

    var body: some View {
        Text(controller.status.label)
        // 版（build.sh が Info.plist の CFBundleVersion に git のコミットを入れる）
        Text("版 \(Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "?")")
        Divider()
        if controller.mode == .closed {
            Button("リングを開く") { controller.openRing() }
                .keyboardShortcut("r", modifiers: [.control, .option])
                .disabled(!controller.status.isReady)
        } else {
            Button("リングを閉じる") { controller.closeRing() }
        }
        Button("設定…") { controller.showSettings() }
        .keyboardShortcut(",")
        Divider()
        Button("終了") {
            controller.closeRing()
            NSApp.terminate(nil)
        }
        .keyboardShortcut("q")
    }
}
