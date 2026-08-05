import GameController

/// Bridges a physical controller (Xbox, PlayStation, Backbone, MFi) to the
/// emulator. Reads the whole pad on every change and diffs it against the last
/// state, so held buttons and simultaneous presses behave like the touch path.
final class ControllerInput {
    private weak var core: EmulatorCore?
    /// Fires when a pad connects or disconnects, so the UI can hide the
    /// on-screen buttons a physical pad makes redundant.
    private let onConnectionChange: (Bool) -> Void
    private var isConnected = false
    private var observers: [NSObjectProtocol] = []
    private var held: GBAKeys = []

    private static let allKeys: GBAKeys = [
        .a, .b, .select, .start, .right, .left, .up, .down, .r, .l,
    ]

    init(core: EmulatorCore, onConnectionChange: @escaping (Bool) -> Void) {
        self.core = core
        self.onConnectionChange = onConnectionChange
        let nc = NotificationCenter.default
        observers.append(nc.addObserver(
            forName: .GCControllerDidConnect, object: nil, queue: .main
        ) { [weak self] note in
            if let pad = note.object as? GCController { self?.attach(pad) }
        })
        observers.append(nc.addObserver(
            forName: .GCControllerDidDisconnect, object: nil, queue: .main
        ) { [weak self] _ in
            self?.refreshConnection()
        })
        GCController.controllers().forEach(attach)
    }

    private func attach(_ controller: GCController) {
        guard let pad = controller.extendedGamepad else { return }
        NSLog("controller attached: \(controller.vendorName ?? "unnamed") "
              + "category=\(controller.productCategory)")
        pad.valueChangedHandler = { [weak self] pad, _ in
            self?.apply(Self.keys(from: pad))
        }
    }

    /// The on-screen buttons hide only once a pad has actually sent input, not
    /// merely by being present. Presence alone is not trustworthy — the
    /// Simulator reports a phantom pad — and hiding the touch controls for a
    /// device that never sends anything leaves no way to play at all.
    private func noteInUse() {
        guard !isConnected else { return }
        isConnected = true
        onConnectionChange(true)
    }

    private func refreshConnection() {
        let stillThere = GCController.controllers().contains { $0.extendedGamepad != nil }
        guard !stillThere, isConnected else { return }
        // A pad yanked mid-press would otherwise leave keys stuck down.
        apply([])
        isConnected = false
        onConnectionChange(false)
    }

    /// The GBA's A is the right face button and B is the left one, which is
    /// the opposite arrangement to an Xbox pad. Bottom maps to A and left maps
    /// to B (the usual emulator convention), with right also firing B so the
    /// natural two-finger grip works either way.
    private static func keys(from pad: GCExtendedGamepad) -> GBAKeys {
        var keys: GBAKeys = []
        if pad.dpad.up.isPressed || pad.leftThumbstick.up.isPressed { keys.insert(.up) }
        if pad.dpad.down.isPressed || pad.leftThumbstick.down.isPressed { keys.insert(.down) }
        if pad.dpad.left.isPressed || pad.leftThumbstick.left.isPressed { keys.insert(.left) }
        if pad.dpad.right.isPressed || pad.leftThumbstick.right.isPressed { keys.insert(.right) }
        if pad.buttonA.isPressed { keys.insert(.a) }
        if pad.buttonX.isPressed || pad.buttonB.isPressed { keys.insert(.b) }
        if pad.leftShoulder.isPressed || pad.leftTrigger.isPressed { keys.insert(.l) }
        if pad.rightShoulder.isPressed || pad.rightTrigger.isPressed { keys.insert(.r) }
        if pad.buttonMenu.isPressed { keys.insert(.start) }
        if pad.buttonOptions?.isPressed == true { keys.insert(.select) }
        return keys
    }

    private func apply(_ keys: GBAKeys) {
        guard keys != held else { return }
        if !keys.isEmpty { noteInUse() }
        core?.release(Self.allKeys.subtracting(keys))
        core?.press(keys)
        held = keys
    }

    deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
    }
}
