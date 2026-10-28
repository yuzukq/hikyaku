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
        // 開くたびに今いるデスクトップ（フルスクリーンのアプリの上も）に出す。
        // これがないと、最初に開いたデスクトップに張り付いて、別のデスクトップで開いてもそちらに残る
        window.collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary]
        window.center()
    }

    func show() {
        if window.isMiniaturized { window.deminiaturize(nil) }
        // Dock に出ない（LSUIElement）アプリなので、前に出してキー入力を受けるには自分を有効にする。
        // リングから開くときは、ほかのアプリを使っている最中なので、NSApp.activate()（macOS 14 からの協調的なアクティベーション）は
        // 断られ、orderFrontRegardless を足しても、設定画面はそのアプリのウィンドウの裏に出る（デスクトップで確認）。
        // 非推奨だが、ignoringOtherApps で強く有効にする（フルスクリーンのアプリの上でも前に出ることを確認）
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }
}
