import SwiftUI

/// Touch controls: cross d-pad on the left, diagonal A/B cluster on the
/// right, shoulder tabs, slim Start/Select pills. All input goes through
/// TouchCapture (raw UIKit touches) — never SwiftUI gestures.
struct ControlsOverlay: View {
    let core: EmulatorCore
    /// A physical pad is attached, so the on-screen buttons only get in the way.
    let controllerConnected: Bool
    /// Bumped whenever a slot is written. Slot labels come from the file system
    /// via `core`, which SwiftUI cannot observe, so without this the menu keeps
    /// showing "Empty" for a slot that was just saved.
    let stateRevision: Int
    let onSaveState: (Int) -> Void
    let onLoadState: (Int) -> Void
    let onEject: () -> Void

    @State private var turbo = false

    var body: some View {
        GeometryReader { geo in
            let landscape = geo.size.width > geo.size.height
            layout(landscape: landscape)
        }
    }

    @ViewBuilder
    private func layout(landscape: Bool) -> some View {
        if controllerConnected {
            // Keep the menu reachable, drop everything the pad replaces.
            VStack {
                Spacer()
                HStack(spacing: 12) {
                    Spacer()
                    turboButton
                    menuButton
                }
                .padding(.horizontal, 26)
                .padding(.bottom, 10)
            }
        } else if landscape {
            landscapeLayout
        } else {
            portraitLayout
        }
    }

    /// Fast-forward toggle. Extra frames are emulated per audio frame, so the
    /// game speeds up while audio keeps pacing at real time.
    private var turboButton: some View {
        Button {
            turbo.toggle()
            core.setSpeed(turbo ? 3 : 1)
        } label: {
            Image(systemName: "forward.fill")
                .font(.footnote.weight(.bold))
                .foregroundStyle(turbo ? Color.white : .white.opacity(0.45))
                .frame(width: 34, height: 26)
                .background(
                    Capsule().fill(turbo ? Color.indigo.opacity(0.85) : Color.white.opacity(0.08))
                )
                .overlay(
                    Capsule().strokeBorder(Color.white.opacity(turbo ? 0.3 : 0.10))
                )
        }
    }

    private var menuButton: some View {
        Menu {
            Menu("Save State") {
                ForEach(1...EmulatorCore.stateSlots, id: \.self) { slot in
                    Button(slotLabel(slot)) { onSaveState(slot) }
                }
            }
            Menu("Load State") {
                ForEach(1...EmulatorCore.stateSlots, id: \.self) { slot in
                    Button(slotLabel(slot)) { onLoadState(slot) }
                        .disabled(core.stateDate(slot) == nil)
                }
            }
            Button("Eject ROM", role: .destructive, action: onEject)
        } label: {
            Image(systemName: "ellipsis")
                .font(.footnote.weight(.bold))
                .foregroundStyle(.white.opacity(0.45))
                .frame(width: 34, height: 26)
                .background(
                    Capsule().fill(Color.white.opacity(0.08))
                )
                .overlay(
                    Capsule().strokeBorder(Color.white.opacity(0.10))
                )
        }
    }

    /// "Slot 2 · Today 3:14 PM", or "Slot 2 · Empty" when nothing is stored.
    private func slotLabel(_ slot: Int) -> String {
        guard let date = core.stateDate(slot) else { return "Slot \(slot) · Empty" }
        return "Slot \(slot) · \(date.formatted(date: .abbreviated, time: .shortened))"
    }

    /// Landscape: controls live in the gutters beside the screen.
    private var landscapeLayout: some View {
        HStack {
            VStack(spacing: 22) {
                HoldButton(label: "L", keys: .l, core: core, style: .shoulder(leading: true))
                DPad(core: core)
                HoldButton(label: "SELECT", keys: .select, core: core, style: .pill)
            }
            Spacer()
            VStack(spacing: 22) {
                HoldButton(label: "R", keys: .r, core: core, style: .shoulder(leading: false))
                ABCluster(core: core)
                // Stacked, not in one row: a row of Start plus both small
                // buttons is wider than the gutter and spills over the screen.
                VStack(spacing: 10) {
                    HoldButton(label: "START", keys: .start, core: core, style: .pill)
                    HStack(spacing: 10) {
                        turboButton
                        menuButton
                    }
                }
            }
        }
        .padding(.horizontal, 14)
    }

    private var portraitLayout: some View {
        VStack {
            Spacer()
            HStack {
                HoldButton(label: "L", keys: .l, core: core, style: .shoulder(leading: true))
                Spacer()
                HoldButton(label: "R", keys: .r, core: core, style: .shoulder(leading: false))
            }
            .padding(.horizontal, 20)
            HStack(alignment: .center) {
                DPad(core: core)
                Spacer()
                ABCluster(core: core)
            }
            .padding(.horizontal, 22)
            .padding(.top, 10)
            HStack(spacing: 12) {
                HoldButton(label: "SELECT", keys: .select, core: core, style: .pill)
                HoldButton(label: "START", keys: .start, core: core, style: .pill)
                Spacer()
                turboButton
                menuButton
            }
            .padding(.horizontal, 26)
            .padding(.top, 16)
            .padding(.bottom, 10)
        }
    }
}

// MARK: - A/B cluster

/// A sits high, B low, on the GBA's natural thumb diagonal.
private struct ABCluster: View {
    let core: EmulatorCore

    var body: some View {
        HStack(alignment: .center, spacing: 18) {
            HoldButton(label: "B", keys: .b, core: core, style: .round)
                .offset(y: 16)
            HoldButton(label: "A", keys: .a, core: core, style: .round)
                .offset(y: -16)
        }
    }
}

// MARK: - Buttons

private struct HoldButton: View {
    enum Style {
        case round
        case pill
        case shoulder(leading: Bool)
    }

    let label: String
    let keys: GBAKeys
    let core: EmulatorCore
    var style: Style = .round
    @State private var held = false

    var body: some View {
        face
            .overlay(
                TouchCapture { point in
                    let down = point != nil
                    guard down != held else { return }
                    held = down
                    if down {
                        core.press(keys)
                    } else {
                        core.release(keys)
                    }
                }
            )
            .animation(.easeOut(duration: 0.08), value: held)
    }

    @ViewBuilder
    private var face: some View {
        switch style {
        case .round:
            Text(label)
                .font(.system(size: 22, weight: .heavy, design: .rounded))
                .foregroundStyle(.white.opacity(held ? 0.95 : 0.7))
                .frame(width: 66, height: 66)
                .background(
                    Circle().fill(
                        RadialGradient(
                            colors: held
                                ? [Color.indigo.opacity(0.95), Color.indigo.opacity(0.55)]
                                : [Color.white.opacity(0.16), Color.white.opacity(0.07)],
                            center: .init(x: 0.5, y: 0.35),
                            startRadius: 2, endRadius: 52
                        )
                    )
                )
                .overlay(
                    Circle().strokeBorder(
                        Color.white.opacity(held ? 0.35 : 0.14), lineWidth: 1
                    )
                )
                .scaleEffect(held ? 0.93 : 1)
                .shadow(color: held ? Color.indigo.opacity(0.5) : .clear, radius: 10)
        case .pill:
            Text(label)
                .font(.system(size: 10, weight: .bold, design: .rounded))
                .tracking(1.6)
                .foregroundStyle(.white.opacity(held ? 0.95 : 0.55))
                .frame(width: 78, height: 26)
                .background(
                    Capsule().fill(held ? Color.indigo.opacity(0.8) : Color.white.opacity(0.09))
                )
                .overlay(
                    Capsule().strokeBorder(Color.white.opacity(held ? 0.3 : 0.10))
                )
                .scaleEffect(held ? 0.95 : 1)
        case .shoulder(let leading):
            let shape = UnevenRoundedRectangle(
                topLeadingRadius: leading ? 18 : 6,
                bottomLeadingRadius: 6,
                bottomTrailingRadius: 6,
                topTrailingRadius: leading ? 6 : 18
            )
            Text(label)
                .font(.system(size: 14, weight: .heavy, design: .rounded))
                .foregroundStyle(.white.opacity(held ? 0.95 : 0.6))
                .frame(width: 64, height: 34)
                .background(
                    shape.fill(held ? Color.indigo.opacity(0.8) : Color.white.opacity(0.10))
                )
                .overlay(
                    shape.strokeBorder(Color.white.opacity(held ? 0.3 : 0.12))
                )
                .scaleEffect(held ? 0.95 : 1)
        }
    }
}

// MARK: - D-pad

private struct DPad: View {
    let core: EmulatorCore
    @State private var active: GBAKeys = []

    private let size: CGFloat = 156
    private let arm: CGFloat = 50
    /// Bigger than it is to leave, so resting near the middle doesn't chatter.
    private let engageRadius: CGFloat = 20
    private let releaseRadius: CGFloat = 12
    /// Cardinals own 70 degrees each, diagonals the 20 between them.
    private let cardinalHalfWidth: CGFloat = 35
    /// Extra wedge for the direction already held, so a slide has to be meant.
    private let stickiness: CGFloat = 12

    var body: some View {
        ZStack {
            DPadCross(armWidth: arm, cornerRadius: 12)
                .fill(Color.white.opacity(0.10))
            DPadCross(armWidth: arm, cornerRadius: 12)
                .strokeBorder(Color.white.opacity(0.14), lineWidth: 1)

            armHighlight(.up, x: 0, y: -1)
            armHighlight(.down, x: 0, y: 1)
            armHighlight(.left, x: -1, y: 0)
            armHighlight(.right, x: 1, y: 0)

            arrow("chevron.up", key: .up, x: 0, y: -1)
            arrow("chevron.down", key: .down, x: 0, y: 1)
            arrow("chevron.left", key: .left, x: -1, y: 0)
            arrow("chevron.right", key: .right, x: 1, y: 0)

            Circle()
                .fill(Color.white.opacity(0.10))
                .frame(width: 16, height: 16)
        }
        .frame(width: size, height: size)
        .overlay(
            TouchCapture { point in
                guard let point else {
                    setKeys([])
                    return
                }
                setKeys(direction(at: point))
            }
        )
        .animation(.easeOut(duration: 0.08), value: active)
    }

    @ViewBuilder
    private func armHighlight(_ key: GBAKeys, x: CGFloat, y: CGFloat) -> some View {
        if active.contains(key) {
            RoundedRectangle(cornerRadius: 10)
                .fill(Color.indigo.opacity(0.75))
                .frame(
                    width: x == 0 ? arm - 8 : (size - arm) / 2 - 4,
                    height: y == 0 ? arm - 8 : (size - arm) / 2 - 4
                )
                .offset(
                    x: x * (size / 2 - (size - arm) / 4 - 4),
                    y: y * (size / 2 - (size - arm) / 4 - 4)
                )
        }
    }

    private func arrow(_ name: String, key: GBAKeys, x: CGFloat, y: CGFloat) -> some View {
        Image(systemName: name)
            .font(.system(size: 15, weight: .heavy))
            .foregroundStyle(.white.opacity(active.contains(key) ? 0.95 : 0.4))
            .offset(x: x * (size / 2 - 20), y: y * (size / 2 - 20))
    }

    /// Which way the thumb is pointing, by angle rather than by two
    /// independent axis thresholds.
    ///
    /// The old version added a direction whenever the touch drifted 14 points
    /// off centre on either axis, so holding left with the thumb a little high
    /// also pressed up — which in a platformer means jumping when you meant to
    /// walk. Here the four cardinals own a wide 70-degree wedge each and the
    /// diagonals only the 20 degrees between them, so a diagonal has to be
    /// deliberate. The wedge for the direction already held is widened, and
    /// the centre dead zone is larger to leave than to enter, so a thumb
    /// sliding across the cross doesn't flicker between directions.
    private func direction(at point: CGPoint) -> GBAKeys {
        let dx = point.x - size / 2
        let dy = point.y - size / 2
        let radius = sqrt(dx * dx + dy * dy)
        if radius < (active.isEmpty ? engageRadius : releaseRadius) { return [] }

        var angle = atan2(dy, dx) * 180 / .pi // 0 is right, 90 is down
        if angle < 0 { angle += 360 }

        let cardinals: [(center: CGFloat, keys: GBAKeys)] = [
            (0, .right), (90, .down), (180, .left), (270, .up),
        ]
        for cardinal in cardinals {
            let halfWidth = active == cardinal.keys ? cardinalHalfWidth + stickiness
                                                    : cardinalHalfWidth
            if angleGap(angle, cardinal.center) <= halfWidth { return cardinal.keys }
        }

        let diagonals: [(center: CGFloat, keys: GBAKeys)] = [
            (45, [.down, .right]), (135, [.down, .left]),
            (225, [.up, .left]), (315, [.up, .right]),
        ]
        return diagonals.min { angleGap(angle, $0.center) < angleGap(angle, $1.center) }?.keys ?? []
    }

    private func angleGap(_ a: CGFloat, _ b: CGFloat) -> CGFloat {
        let d = abs(a - b).truncatingRemainder(dividingBy: 360)
        return d > 180 ? 360 - d : d
    }

    private func setKeys(_ keys: GBAKeys) {
        guard keys != active else { return }
        let all: GBAKeys = [.up, .down, .left, .right]
        core.release(all.subtracting(keys))
        core.press(keys)
        active = keys
    }
}

/// Plus/cross silhouette built from two rounded bars.
private struct DPadCross: InsettableShape {
    var armWidth: CGFloat
    var cornerRadius: CGFloat
    var inset: CGFloat = 0

    func inset(by amount: CGFloat) -> DPadCross {
        var s = self
        s.inset += amount
        return s
    }

    func path(in rect: CGRect) -> Path {
        let r = rect.insetBy(dx: inset, dy: inset)
        let w = armWidth - inset * 2
        var p = Path()
        p.addRoundedRect(
            in: CGRect(x: r.midX - w / 2, y: r.minY, width: w, height: r.height),
            cornerSize: CGSize(width: cornerRadius, height: cornerRadius)
        )
        p.addRoundedRect(
            in: CGRect(x: r.minX, y: r.midY - w / 2, width: r.width, height: w),
            cornerSize: CGSize(width: cornerRadius, height: cornerRadius)
        )
        return p
    }
}
