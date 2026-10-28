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
        // リングから開くときは、ほかのアプリを使っている最中（この Mac で最後に操作されたのはそのアプリ）なので、
        // NSApp.activate() は断られることがあり（macOS 14 からの協調的なアクティベーション）、
        // makeKeyAndOrderFront だけでは、すでに開いているウィンドウがそのアプリの裏に残る。自分が有効でなくても前に出す
        window.orderFrontRegardless()
        // Dock に出ない（LSUIElement）アプリなので、キー入力を受けるには自分を有効にする
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }
}
