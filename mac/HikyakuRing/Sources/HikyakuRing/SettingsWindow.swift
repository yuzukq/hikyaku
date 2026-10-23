// 設定画面のウィンドウ。メニューの「設定…」からも、リングの項目（設定画面を開く）からも開くので、
// SwiftUI の Window シーン（ビューの中の openWindow でしか開けない）ではなく、AppKit のウィンドウで持つ。

import AppKit
import SwiftUI

@MainActor
final class SettingsWindow {
    private let window: NSWindow

    init(controller: RingController) {
        window = NSWindow(contentViewController: NSHostingController(rootView: SettingsView(controller: controller)))
        window.title = "hikyaku リングの設定"
        window.styleMask = [.titled, .closable, .miniaturizable]
        window.isReleasedWhenClosed = false
        window.center()
    }

    func show() {
        // Dock に出ない（LSUIElement）アプリなので、前に出すには自分を有効にする
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }
}
