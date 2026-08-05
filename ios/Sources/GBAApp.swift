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

struct ContentView: View {
    @State private var core: EmulatorCore?
    @State private var audio: AudioEngine?
    @State private var games: [Game] = []
    @State private var showPicker = false
    @State private var loadError: String?

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            if let core {
                GeometryReader { geo in
                    let landscape = geo.size.width > geo.size.height
                    if landscape {
                        // Screen fills the height between the control gutters.
                        EmulatorScreen(core: core)
                            .aspectRatio(CGFloat(3) / 2, contentMode: .fit)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .padding(.horizontal, 205)
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
                .ignoresSafeArea(edges: .horizontal)
                ControlsOverlay(core: core, onSaveState: { core.saveState() },
                                onLoadState: { core.loadState() },
                                onEject: { eject() })
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
                loadError = error.localizedDescription
            }
        }
        .alert("Can't load that file", isPresented: .constant(loadError != nil)) {
            Button("OK") { loadError = nil }
        } message: {
            Text(loadError ?? "")
        }
        .onAppear { refresh() }
    }

    private func refresh() {
        games = GameLibrary.scan()
    }

    private func play(_ game: Game) {
        do {
            let rom = try ROMLoader.load(from: game.url)
            guard let c = EmulatorCore(rom: rom, romURL: game.url) else {
                loadError = "That game couldn't be started."
                return
            }
            GameLibrary.markPlayed(game.url)
            core = c
            audio = AudioEngine(core: c)
        } catch {
            loadError = error.localizedDescription
        }
    }

    private func delete(_ game: Game) {
        GameLibrary.delete(game)
        refresh()
    }

    private func eject() {
        audio?.stop()
        audio = nil
        core = nil
        // Ordering matters: the core writes its battery save on deinit, so
        // rescan afterwards to pick up a save file created just now.
        refresh()
    }
}
