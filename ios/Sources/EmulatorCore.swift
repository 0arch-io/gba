import Foundation
import QuartzCore

/// GBA key bits (active low in hardware; this type tracks *pressed* keys and
/// converts when passed to the core).
struct GBAKeys: OptionSet {
    let rawValue: UInt16
    static let a = GBAKeys(rawValue: 1 << 0)
    static let b = GBAKeys(rawValue: 1 << 1)
    static let select = GBAKeys(rawValue: 1 << 2)
    static let start = GBAKeys(rawValue: 1 << 3)
    static let right = GBAKeys(rawValue: 1 << 4)
    static let left = GBAKeys(rawValue: 1 << 5)
    static let up = GBAKeys(rawValue: 1 << 6)
    static let down = GBAKeys(rawValue: 1 << 7)
    static let r = GBAKeys(rawValue: 1 << 8)
    static let l = GBAKeys(rawValue: 1 << 9)
}

/// Owns the Rust emulator core and its save files. Not thread-safe; the
/// audio engine drives emulation from its render thread via `pump`.
final class EmulatorCore {
    static let width = 240
    static let height = 160

    private var handle: UnsafeMutableRawPointer
    private let savURL: URL
    private let stateURL: URL
    private var framesSinceSave = 0
    let lock = NSLock()
    var pressed = GBAKeys()

    /// `rom` is the validated image (already unzipped if needed); `romURL`
    /// only supplies the name used for save files.
    init?(rom: Data, romURL: URL) {
        let base = romURL.deletingPathExtension().lastPathComponent
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        savURL = docs.appendingPathComponent(base + ".sav")
        stateURL = docs.appendingPathComponent(base + ".state")
        let sav = (try? Data(contentsOf: savURL)) ?? Data()
        handle = rom.withUnsafeBytes { romBuf in
            sav.withUnsafeBytes { savBuf in
                gba_create(
                    romBuf.bindMemory(to: UInt8.self).baseAddress, rom.count,
                    sav.isEmpty ? nil : savBuf.bindMemory(to: UInt8.self).baseAddress, sav.count
                )
            }
        }
    }

    deinit {
        writeSaveIfDirty()
        gba_destroy(handle)
    }

    /// Run one frame and drain its audio into `out`. Returns samples written.
    func pump(into out: UnsafeMutablePointer<Float>, max: Int) -> Int {
        lock.lock()
        defer { lock.unlock() }
        gba_run_frame(handle, ~pressed.rawValue & 0x3FF)
        framesSinceSave += 1
        if framesSinceSave >= 60 {
            framesSinceSave = 0
            writeSaveIfDirty()
        }
        return gba_audio_read(handle, out, max)
    }

    /// Copy the current framebuffer into a CGImage for display.
    func frameImage() -> CGImage? {
        lock.lock()
        let fb = gba_framebuffer(handle)
        let count = Self.width * Self.height
        let data = Data(bytes: fb!, count: count * 4)
        lock.unlock()
        let provider = CGDataProvider(data: data as CFData)!
        return CGImage(
            width: Self.width, height: Self.height,
            bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: Self.width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipFirst.rawValue)
                .union(.byteOrder32Little),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent
        )
    }

    private func writeSaveIfDirty() {
        guard gba_flash_dirty(handle) else { return }
        var buf = [UInt8](repeating: 0, count: 0x20000)
        let n = gba_flash_read(handle, &buf, buf.count)
        try? Data(buf[..<n]).write(to: savURL)
    }

    func saveState() {
        lock.lock()
        defer { lock.unlock() }
        let size = gba_state_save(handle, nil, 0)
        guard size > 0 else { return }
        var buf = [UInt8](repeating: 0, count: size)
        let n = gba_state_save(handle, &buf, size)
        if n > 0 {
            try? Data(buf[..<n]).write(to: stateURL)
        }
    }

    func loadState() {
        guard let data = try? Data(contentsOf: stateURL) else { return }
        lock.lock()
        defer { lock.unlock() }
        _ = data.withUnsafeBytes {
            gba_state_load(handle, $0.bindMemory(to: UInt8.self).baseAddress, data.count)
        }
    }
}
