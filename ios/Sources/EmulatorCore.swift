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

    /// How many save state slots each game gets.
    static let stateSlots = 3

    private var handle: UnsafeMutableRawPointer
    private let savURL: URL
    private let stateBase: URL
    private var framesSinceSave = 0
    /// Emulated frames per frame of audio output. >1 fast-forwards: the extra
    /// frames' audio is dropped, so the audio clock still paces at real time.
    private var speed = 1
    let lock = NSLock()
    private var pressed = GBAKeys()
    // A tap can press+release faster than one emulated frame, so releases are
    // deferred until the key has been visible to the game for a few frames.
    private var releaseRequested = GBAKeys()
    private var framesHeld: [UInt16: Int] = [:]
    private let minHoldFrames = 3

    /// Mark keys down. Safe to call from any thread.
    func press(_ keys: GBAKeys) {
        lock.lock()
        pressed.insert(keys)
        releaseRequested.remove(keys)
        for bit in 0..<10 where keys.rawValue & (1 << bit) != 0 {
            if framesHeld[1 << bit] == nil { framesHeld[1 << bit] = 0 }
        }
        lock.unlock()
    }

    /// Mark keys up; the actual release happens once `minHoldFrames` have run.
    func release(_ keys: GBAKeys) {
        lock.lock()
        releaseRequested.insert(pressed.intersection(keys))
        lock.unlock()
    }

    /// `rom` is the validated image (already unzipped if needed); `romURL`
    /// only supplies the name used for save files.
    init?(rom: Data, romURL: URL) {
        let base = romURL.deletingPathExtension().lastPathComponent
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        savURL = docs.appendingPathComponent(base + ".sav")
        stateBase = docs.appendingPathComponent(base)
        // Slots replaced the old single ".state" file; keep that save by
        // promoting it to slot 1 the first time this game is opened.
        let legacy = docs.appendingPathComponent(base + ".state")
        let slotOne = docs.appendingPathComponent(base + ".state1")
        if FileManager.default.fileExists(atPath: legacy.path),
           !FileManager.default.fileExists(atPath: slotOne.path) {
            try? FileManager.default.moveItem(at: legacy, to: slotOne)
        }
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

    /// Fast-forward multiplier; 1 is real time. Safe to call from any thread.
    func setSpeed(_ multiplier: Int) {
        lock.lock()
        speed = Swift.max(1, multiplier)
        lock.unlock()
    }

    /// Run a frame (or several, when fast-forwarding) and drain the last
    /// frame's audio into `out`. Returns samples written.
    func pump(into out: UnsafeMutablePointer<Float>, max: Int) -> Int {
        lock.lock()
        defer { lock.unlock() }
        // Audio from the skipped-ahead frames is read and thrown away; if it
        // were left in the core's buffer it would back up and play late.
        for _ in 1..<speed {
            advanceFrame()
            _ = gba_audio_read(handle, out, max)
        }
        advanceFrame()
        return gba_audio_read(handle, out, max)
    }

    /// One emulated frame plus the bookkeeping that rides along with it.
    /// Caller must hold `lock`.
    private func advanceFrame() {
        gba_run_frame(handle, ~pressed.rawValue & 0x3FF)
        for (bit, n) in framesHeld {
            let key = GBAKeys(rawValue: bit)
            if n + 1 >= minHoldFrames, releaseRequested.contains(key) {
                pressed.remove(key)
                releaseRequested.remove(key)
                framesHeld[bit] = nil
            } else {
                framesHeld[bit] = n + 1
            }
        }
        framesSinceSave += 1
        if framesSinceSave >= 60 {
            framesSinceSave = 0
            writeSaveIfDirty()
        }
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

    private func stateURL(_ slot: Int) -> URL {
        URL(fileURLWithPath: stateBase.path + ".state\(slot)")
    }

    /// When each slot was last written, for labelling the menu.
    func stateDate(_ slot: Int) -> Date? {
        try? FileManager.default
            .attributesOfItem(atPath: stateURL(slot).path)[.modificationDate] as? Date
    }

    /// Returns false if the state could not be written; the caller surfaces
    /// that rather than letting a failed save look like it worked.
    @discardableResult
    func saveState(slot: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let size = gba_state_save(handle, nil, 0)
        guard size > 0 else { return false }
        var buf = [UInt8](repeating: 0, count: size)
        let n = gba_state_save(handle, &buf, size)
        guard n > 0 else { return false }
        do {
            try Data(buf[..<n]).write(to: stateURL(slot))
            return true
        } catch {
            NSLog("save state slot \(slot) failed: \(error)")
            return false
        }
    }

    @discardableResult
    func loadState(slot: Int) -> Bool {
        guard let data = try? Data(contentsOf: stateURL(slot)) else { return false }
        lock.lock()
        defer { lock.unlock() }
        return data.withUnsafeBytes {
            gba_state_load(handle, $0.bindMemory(to: UInt8.self).baseAddress, data.count)
        }
    }
}
