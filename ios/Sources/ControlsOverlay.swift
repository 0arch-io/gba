import SwiftUI

/// Touch controls: floating d-pad on the left, A/B on the right,
/// Start/Select plus emulator functions along the bottom.
struct ControlsOverlay: View {
    let core: EmulatorCore
    let onSaveState: () -> Void
    let onLoadState: () -> Void
    let onEject: () -> Void

    var body: some View {
        GeometryReader { geo in
            let landscape = geo.size.width > geo.size.height
            layout(landscape: landscape)
        }
    }

    @ViewBuilder
    private func layout(landscape: Bool) -> some View {
        if landscape {
            landscapeLayout
        } else {
            portraitLayout
        }
    }

    /// Landscape: controls live in the gutters beside the screen.
    private var landscapeLayout: some View {
        HStack {
            VStack(spacing: 18) {
                DPad(core: core).scaleEffect(0.8)
                HoldButton(label: "SELECT", keys: .select, core: core, pill: true)
            }
            Spacer()
            VStack(spacing: 14) {
                HStack(spacing: 12) {
                    HoldButton(label: "L", keys: .l, core: core, small: true)
                    HoldButton(label: "R", keys: .r, core: core, small: true)
                }
                HStack(spacing: 14) {
                    HoldButton(label: "B", keys: .b, core: core)
                    HoldButton(label: "A", keys: .a, core: core)
                }
                HStack(spacing: 12) {
                    HoldButton(label: "START", keys: .start, core: core, pill: true)
                    Menu {
                        Button("Save State", action: onSaveState)
                        Button("Load State", action: onLoadState)
                        Button("Eject ROM", role: .destructive, action: onEject)
                    } label: {
                        Image(systemName: "ellipsis.circle.fill")
                            .font(.title2)
                            .foregroundStyle(.white.opacity(0.5))
                    }
                }
            }
        }
        .padding(.horizontal, 10)
    }

    private var portraitLayout: some View {
        VStack {
            Spacer()
            HStack(alignment: .bottom) {
                DPad(core: core)
                Spacer()
                VStack(spacing: 14) {
                    HStack(spacing: 14) {
                        HoldButton(label: "B", keys: .b, core: core)
                        HoldButton(label: "A", keys: .a, core: core)
                    }
                    HStack(spacing: 10) {
                        HoldButton(label: "L", keys: .l, core: core, small: true)
                        HoldButton(label: "R", keys: .r, core: core, small: true)
                    }
                }
            }
            .padding(.horizontal, 24)
            HStack(spacing: 18) {
                HoldButton(label: "SELECT", keys: .select, core: core, pill: true)
                HoldButton(label: "START", keys: .start, core: core, pill: true)
                Spacer()
                Menu {
                    Button("Save State", action: onSaveState)
                    Button("Load State", action: onLoadState)
                    Button("Eject ROM", role: .destructive, action: onEject)
                } label: {
                    Image(systemName: "ellipsis.circle.fill")
                        .font(.title)
                        .foregroundStyle(.white.opacity(0.5))
                }
            }
            .padding(.horizontal, 28)
            .padding(.bottom, 12)
        }
    }
}

private struct HoldButton: View {
    let label: String
    let keys: GBAKeys
    let core: EmulatorCore
    var small = false
    var pill = false
    @GestureState private var held = false

    var body: some View {
        let size: CGFloat = small ? 44 : 64
        Text(label)
            .font(pill ? .caption.bold() : .title2.bold())
            .foregroundStyle(.white.opacity(held ? 1 : 0.75))
            .frame(width: pill ? 84 : size, height: pill ? 30 : size)
            .background(
                Capsule().fill(held ? Color.indigo : Color.white.opacity(0.14))
            )
            .gesture(
                DragGesture(minimumDistance: 0)
                    .updating($held) { _, s, _ in s = true }
            )
            .onChange(of: held) { _, down in
                core.lock.lock()
                if down {
                    core.pressed.insert(keys)
                } else {
                    core.pressed.remove(keys)
                }
                core.lock.unlock()
            }
    }
}

private struct DPad: View {
    let core: EmulatorCore
    @State private var active: GBAKeys = []

    var body: some View {
        ZStack {
            Circle().fill(Color.white.opacity(0.10))
            Image(systemName: "dpad.fill")
                .font(.system(size: 64))
                .foregroundStyle(.white.opacity(active.isEmpty ? 0.6 : 0.9))
        }
        .frame(width: 150, height: 150)
        .contentShape(Circle())
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { g in
                    let dx = g.location.x - 75
                    let dy = g.location.y - 75
                    var keys: GBAKeys = []
                    if abs(dx) > 14 {
                        keys.insert(dx > 0 ? .right : .left)
                    }
                    if abs(dy) > 14 {
                        keys.insert(dy > 0 ? .down : .up)
                    }
                    setKeys(keys)
                }
                .onEnded { _ in setKeys([]) }
        )
    }

    private func setKeys(_ keys: GBAKeys) {
        guard keys != active else { return }
        core.lock.lock()
        core.pressed.remove([.up, .down, .left, .right])
        core.pressed.insert(keys)
        core.lock.unlock()
        active = keys
    }
}
