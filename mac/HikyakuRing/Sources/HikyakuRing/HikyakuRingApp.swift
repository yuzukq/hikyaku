// hikyaku のショートカットリング: メニューバーに常駐し、F23 でリングを出す。仕様: docs/design/ring.md

import RingCore
import SwiftUI

@main
struct HikyakuRingApp: App {
    @State private var controller = RingController()

    var body: some Scene {
        MenuBarExtra {
            MenuContent(controller: controller)
        } label: {
            Image(systemName: controller.status.isReady ? "circle.circle.fill" : "circle.circle")
        }
        // Settings シーンと SettingsLink は、Dock に出ない（LSUIElement）アプリでは開かないことがあるので、普通のウィンドウにする
        Window("hikyaku リングの設定", id: "settings") {
            SettingsView(controller: controller)
        }
        .windowResizability(.contentSize)
    }
}

private struct MenuContent: View {
    let controller: RingController
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Text(controller.status.label)
        Divider()
        if controller.mode == .closed {
            Button("リングを開く") { controller.openRing() }
                .disabled(!controller.status.isReady)
        } else {
            Button("リングを閉じる") { controller.closeRing() }
        }
        Button("設定…") {
            openWindow(id: "settings")
            NSApp.activate()
        }
        .keyboardShortcut(",")
        Divider()
        Button("終了") {
            controller.closeRing()
            NSApp.terminate(nil)
        }
        .keyboardShortcut("q")
    }
}
