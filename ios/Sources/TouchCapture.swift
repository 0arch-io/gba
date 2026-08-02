import SwiftUI
import UIKit

/// Raw UIKit touch capture. SwiftUI gestures route through the
/// gesture-recognizer system, which delays/coalesces touches on device and
/// fights multitouch; emulator controls need touchesBegan/Ended directly.
struct TouchCapture: UIViewRepresentable {
    /// Called with the current touch location in the view's coordinates,
    /// or nil when the last finger lifts. Fires on the main thread.
    let onTouch: (CGPoint?) -> Void

    func makeUIView(context: Context) -> TouchTrackingView {
        let v = TouchTrackingView()
        v.onTouch = onTouch
        return v
    }

    func updateUIView(_ uiView: TouchTrackingView, context: Context) {
        uiView.onTouch = onTouch
    }
}

final class TouchTrackingView: UIView {
    var onTouch: ((CGPoint?) -> Void)?
    private var tracked: UITouch?

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        isMultipleTouchEnabled = true
    }

    required init?(coder: NSCoder) { fatalError() }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        if tracked == nil, let t = touches.first {
            tracked = t
            onTouch?(t.location(in: self))
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        if let t = tracked, touches.contains(t) {
            onTouch?(t.location(in: self))
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        endIfTracked(touches)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        endIfTracked(touches)
    }

    private func endIfTracked(_ touches: Set<UITouch>) {
        if let t = tracked, touches.contains(t) {
            tracked = nil
            onTouch?(nil)
        }
    }
}
