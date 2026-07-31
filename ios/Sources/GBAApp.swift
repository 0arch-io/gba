import SwiftUI
import UniformTypeIdentifiers

@main
struct GBAApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
                .preferredColorScheme(.dark)
                .statusBarHidden()
        }
    }
}

struct ContentView: View {
    @State private var core: EmulatorCore?
    @State private var audio: AudioEngine?
    @State private var showPicker = false
    @State private var lastROM: URL?

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            if let core {
                EmulatorScreen(core: core)
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
            // Copy into Documents so saves live beside a stable location.
            let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            let local = docs.appendingPathComponent(url.lastPathComponent)
            if !FileManager.default.fileExists(atPath: local.path) {
                try? FileManager.default.copyItem(at: url, to: local)
            }
            start(rom: local)
        }
        .onAppear { autoStart() }
    }

    private func autoStart() {
        // Resume the most recently used ROM if present.
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let roms = (try? FileManager.default.contentsOfDirectory(at: docs, includingPropertiesForKeys: [.contentModificationDateKey]))?
            .filter { $0.pathExtension.lowercased() == "gba" }
            .sorted { a, b in
                let da = (try? a.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                let db = (try? b.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                return da > db
            } ?? []
        if let rom = roms.first {
            start(rom: rom)
        }
    }

    private func start(rom: URL) {
        guard let c = EmulatorCore(romURL: rom) else { return }
        lastROM = rom
        core = c
        audio = AudioEngine(core: c)
    }

    private func eject() {
        audio?.stop()
        audio = nil
        core = nil
    }
}
