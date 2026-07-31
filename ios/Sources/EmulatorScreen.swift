import SwiftUI

/// Displays emulator frames on a CADisplayLink, nearest-neighbor scaled so
/// pixels stay crisp.
struct EmulatorScreen: UIViewRepresentable {
    let core: EmulatorCore

    func makeUIView(context: Context) -> ScreenView {
        let v = ScreenView()
        v.core = core
        v.start()
        return v
    }

    func updateUIView(_ view: ScreenView, context: Context) {
        view.core = core
    }

    static func dismantleUIView(_ view: ScreenView, coordinator: ()) {
        view.stop()
    }
}

final class ScreenView: UIView {
    var core: EmulatorCore?
    private var link: CADisplayLink?

    override init(frame: CGRect) {
        super.init(frame: frame)
        layer.magnificationFilter = .nearest
        contentMode = .scaleAspectFit
        backgroundColor = .black
    }

    required init?(coder: NSCoder) { fatalError() }

    func start() {
        let l = CADisplayLink(target: self, selector: #selector(tick))
        l.add(to: .main, forMode: .common)
        link = l
    }

    func stop() {
        link?.invalidate()
        link = nil
    }

    @objc private func tick() {
        guard let core, let img = core.frameImage() else { return }
        layer.contents = img
        layer.contentsGravity = .resizeAspect
    }
}
