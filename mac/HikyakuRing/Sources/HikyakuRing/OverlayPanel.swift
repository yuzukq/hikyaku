// リングを出す透明なパネル。フォーカスを奪わず、マウスも受けない（操作はすべてパッドから）。
//
// フルスクリーンのアプリの上にも出す。それには、Dock に出ない（LSUIElement）アプリの nonactivating な NSPanel で、
// レベルを .screenSaver にし、.canJoinAllApplications を付ける（Apple の回答: https://developer.apple.com/forums/thread/826308 ）。
// .floating・.statusBar・.popUpMenu では、フルスクリーンのアプリの裏に隠れて見えない（ファームはリングを開いているので、
// 見えないまま触ると項目が実行されてしまう。RingController が isOnScreen で止める）。

import AppKit
import SwiftUI

@MainActor
final class OverlayPanel {
    static let size: CGFloat = 400

    private let panel: NSPanel

    init(controller: RingController) {
        panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: Self.size, height: Self.size),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        panel.level = .screenSaver
        panel.collectionBehavior = [.canJoinAllSpaces, .canJoinAllApplications, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.ignoresMouseEvents = true
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.contentView = NSHostingView(rootView: RingOverlayView(controller: controller))
    }

    /// 少なくとも一部が画面に見えているか（ほかのウィンドウの裏に隠れている・今のデスクトップにないなら false）。
    var isOnScreen: Bool {
        panel.isVisible && panel.occlusionState.contains(.visible)
    }

    /// カーソルを中心に出す。画面の端では、はみ出さないようにずらす。
    func show(near point: NSPoint) {
        let screen = NSScreen.screens.first(where: { NSMouseInRect(point, $0.frame, false) }) ?? NSScreen.main
        var origin = NSPoint(x: point.x - Self.size / 2, y: point.y - Self.size / 2)
        if let visible = screen?.visibleFrame {
            origin.x = min(max(origin.x, visible.minX), visible.maxX - Self.size)
            origin.y = min(max(origin.y, visible.minY), visible.maxY - Self.size)
        }
        panel.setFrameOrigin(origin)
        panel.alphaValue = 0
        panel.orderFrontRegardless()
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.12
            panel.animator().alphaValue = 1
        }
    }

    func hide() {
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.12
            panel.animator().alphaValue = 0
        }, completionHandler: { [panel] in
            // 消える途中でまた開かれていたら、そのまま出しておく
            MainActor.assumeIsolated {
                if panel.alphaValue == 0 { panel.orderOut(nil) }
            }
        })
    }
}
