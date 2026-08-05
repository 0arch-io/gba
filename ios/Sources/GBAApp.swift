import SwiftUI
import UniformTypeIdentifiers

@main
struct GBAApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
                .preferredColorScheme(.dark)
                .statusBarHidden()
                // Controls live at the screen edges; make system edge
                // gestures (home indicator etc.) require a second swipe
                // instead of stealing/delaying the first touch.
                .defersSystemGestures(on: .all)
        }
    }
}

/// A one-off message with its own title, so import failures and save-state
/// failures don't have to share wording.
private struct Notice: Identifiable {
    let id = UUID()
    let title: String
    let message: String
}

struct ContentView: View {
    @State private var core: EmulatorCore?
    @State private var audio: AudioEngine?
    @State private var controller: ControllerInput?
    @State private var controllerConnected = false
    @State private var games: [Game] = []
    @State private var showPicker = false
    @State private var notice: Notice?
    /// Set when Save State would overwrite a slot that already holds one.
    @State private var pendingOverwrite: Int?
    @State private var stateRevision = 0

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            if let core {
                GeometryReader { geo in
                    let landscape = geo.size.width > geo.size.height
                    if landscape {
                        EmulatorScreen(core: core)
                            .aspectRatio(CGFloat(3) / 2, contentMode: .fit)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .padding(.horizontal, gutter(for: geo.size))
                    } else {
                        // Centered in the space above the controls so the
                        // screen doesn't float in a void.
                        VStack(spacing: 0) {
                            Spacer(minLength: 0)
                            EmulatorScreen(core: core)
                                .aspectRatio(CGFloat(3) / 2, contentMode: .fit)
                                .frame(maxWidth: .infinity)
                            Spacer(minLength: 0)
                        }
                        .padding(.bottom, geo.size.height * 0.30)
                    }
                }
                // No .ignoresSafeArea here: the controls honour the safe area,
                // so if the screen ignored it the two would be measured in
                // different widths and the controls would sit on top of the
                // picture on whichever side the notch is.
                ControlsOverlay(
                    core: core,
                    controllerConnected: controllerConnected,
                    stateRevision: stateRevision,
                    onSaveState: { requestSave(slot: $0) },
                    onLoadState: { loadState(slot: $0) },
                    onEject: { eject() }
                )
            } else {
                LibraryView(
                    games: games,
                    onPlay: { play($0) },
                    onImport: { showPicker = true },
                    onDelete: { delete($0) }
                )
            }
        }
        .fileImporter(isPresented: $showPicker, allowedContentTypes: [.data]) { result in
            guard case .success(let url) = result else { return }
            do {
                // Unzips and validates before anything is written or played.
                let stored = try GameLibrary.importROM(from: url)
                // Use the fresh scan directly: reading `games` back straight
                // after assigning it isn't guaranteed to see the new value.
                let scanned = GameLibrary.scan()
                games = scanned
                if let game = scanned.first(where: { $0.url == stored }) { play(game) }
            } catch {
                notice = Notice(title: "Can't load that file", message: error.localizedDescription)
            }
        }
        .alert(item: $notice) { notice in
            Alert(title: Text(notice.title), message: Text(notice.message),
                  dismissButton: .default(Text("OK")))
        }
        .alert("Overwrite slot \(pendingOverwrite ?? 0)?",
               isPresented: overwriteBinding) {
            Button("Cancel", role: .cancel) { pendingOverwrite = nil }
            Button("Overwrite", role: .destructive) {
                if let slot = pendingOverwrite { saveState(slot: slot) }
                pendingOverwrite = nil
            }
        } message: {
            Text("The save state already in this slot will be replaced. This can't be undone.")
        }
        .onAppear { refresh() }
    }

    /// Landscape gutters used to be a hardcoded 205 points per side, which
    /// squeezed the screen on smaller phones and wasted room on iPad. Give the
    /// screen whatever the height allows, and only take what the controls need.
    private func gutter(for size: CGSize) -> CGFloat {
        let screenWidthAtFullHeight = size.height * 1.5 // the GBA's 3:2
        let leftOver = (size.width - screenWidthAtFullHeight) / 2
        // 185 is the widest control column (the 156pt d-pad) plus its padding
        // and a little slack, so the buttons never touch the picture.
        return Swift.max(185, leftOver)
    }

    private var overwriteBinding: Binding<Bool> {
        Binding(get: { pendingOverwrite != nil }, set: { if !$0 { pendingOverwrite = nil } })
    }

    private func refresh() {
        games = GameLibrary.scan()
    }

    private func play(_ game: Game) {
        do {
            let rom = try ROMLoader.load(from: game.url)
            guard let c = EmulatorCore(rom: rom, romURL: game.url) else {
                notice = Notice(title: "Can't start that game",
                                message: "The ROM loaded but the emulator couldn't start it.")
                return
            }
            GameLibrary.markPlayed(game.url)
            core = c
            audio = AudioEngine(core: c)
            controller = ControllerInput(core: c) { connected in
                controllerConnected = connected
            }
        } catch {
            notice = Notice(title: "Can't load that game", message: error.localizedDescription)
        }
    }

    /// Confirm first when the slot is occupied; a mis-tap here used to wipe
    /// the only save state with no warning.
    private func requestSave(slot: Int) {
        guard let core else { return }
        if core.stateDate(slot) == nil {
            saveState(slot: slot)
        } else {
            pendingOverwrite = slot
        }
    }

    private func saveState(slot: Int) {
        guard let core else { return }
        let ok = core.saveState(slot: slot)
        stateRevision += 1 // relabel the slot in the menu
        guard !ok else { return }
        notice = Notice(title: "Save state failed",
                        message: "Slot \(slot) couldn't be written. Your in-game save is untouched.")
    }

    private func loadState(slot: Int) {
        guard let core, !core.loadState(slot: slot) else { return }
        notice = Notice(title: "Load state failed",
                        message: "Slot \(slot) couldn't be read.")
    }

    private func delete(_ game: Game) {
        GameLibrary.delete(game)
        refresh()
    }

    private func eject() {
        audio?.stop()
        audio = nil
        controller = nil
        controllerConnected = false
        core = nil
        // Ordering matters: the core writes its battery save on deinit, so
        // rescan afterwards to pick up a save file created just now.
        refresh()
    }
}
