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
    @State private var showPicker = false
    @State private var lastROM: URL?
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
                VStack(spacing: 24) {
                    Text("GBA")
                        .font(.system(size: 56, weight: .heavy, design: .rounded))
                        .foregroundStyle(.white)
                    Text("A Game Boy Advance emulator")
                        .foregroundStyle(.secondary)
                    Button {
                        showPicker = true
                    } label: {
                        Label("Open ROM", systemImage: "folder")
                            .font(.title3.bold())
                            .padding(.horizontal, 24)
                            .padding(.vertical, 12)
                            .background(Color.indigo, in: Capsule())
                            .foregroundStyle(.white)
                    }
                }
            }
        }
        .fileImporter(isPresented: $showPicker, allowedContentTypes: [.data]) { result in
            guard case .success(let url) = result else { return }
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            do {
                // Unzips and validates before anything touches the core.
                let rom = try ROMLoader.load(from: url)
                // Store the extracted image in Documents so saves and the
                // resume-last-ROM path have a stable location.
                let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
                let name = (url.deletingPathExtension().lastPathComponent) + ".gba"
                let local = docs.appendingPathComponent(name)
                try rom.write(to: local)
                start(rom: rom, url: local)
            } catch {
                loadError = error.localizedDescription
            }
        }
        .alert("Can't load that file", isPresented: .constant(loadError != nil)) {
            Button("OK") { loadError = nil }
        } message: {
            Text(loadError ?? "")
        }
        .onAppear { autoStart() }
    }

    private func autoStart() {
        // Resume the most recently used ROM if present.
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let roms = (try? FileManager.default.contentsOfDirectory(at: docs, includingPropertiesForKeys: [.contentModificationDateKey]))?
            .filter { ["gba", "zip"].contains($0.pathExtension.lowercased()) }
            .sorted { a, b in
                let da = (try? a.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                let db = (try? b.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                return da > db
            } ?? []
        if let url = roms.first, let data = try? ROMLoader.load(from: url) {
            start(rom: data, url: url)
        }
    }

    private func start(rom: Data, url: URL) {
        guard let c = EmulatorCore(rom: rom, romURL: url) else { return }
        lastROM = url
        core = c
        audio = AudioEngine(core: c)
    }

    private func eject() {
        audio?.stop()
        audio = nil
        core = nil
    }
}
