import Compression
import Foundation

/// Loads a ROM from a user-picked file: accepts a raw .gba/.gbc/.gb image or
/// a .zip containing one, and validates it before handing bytes to the core.
enum ROMLoader {
    enum LoadError: LocalizedError {
        case unreadable
        case zipHasNoROM
        case notAGBAROM

        var errorDescription: String? {
            switch self {
            case .unreadable: "That file couldn't be read."
            case .zipHasNoROM: "That zip doesn't contain a .gba ROM."
            case .notAGBAROM: "That file isn't a Game Boy Advance ROM."
            }
        }
    }

    static func load(from url: URL) throws -> Data {
        guard let data = try? Data(contentsOf: url) else { throw LoadError.unreadable }
        let rom = data.starts(with: [0x50, 0x4B, 0x03, 0x04]) ? try unzipROM(data) : data
        guard isGBAROM(rom) else { throw LoadError.notAGBAROM }
        return rom
    }

    /// A GBA ROM header has a fixed 0x96 at offset 0xB2 and the Nintendo logo
    /// checksum region; the fixed byte plus a plausible size is enough here.
    private static func isGBAROM(_ d: Data) -> Bool {
        d.count >= 0xC0 && d[0xB2] == 0x96
    }

    // MARK: - Minimal zip reader (stored + deflate entries)

    private static func unzipROM(_ zip: Data) throws -> Data {
        // Locate the End Of Central Directory record (scan back for its signature).
        let eocdSig: [UInt8] = [0x50, 0x4B, 0x05, 0x06]
        var eocd: Int?
        let start = max(0, zip.count - 66_000)
        var i = zip.count - 22
        while i >= start {
            if Array(zip[i..<i + 4]) == eocdSig {
                eocd = i
                break
            }
            i -= 1
        }
        guard let eocd else { throw LoadError.zipHasNoROM }
        let entryCount = Int(u16(zip, eocd + 10))
        var offset = Int(u32(zip, eocd + 16)) // start of central directory

        for _ in 0..<entryCount {
            guard offset + 46 <= zip.count, Array(zip[offset..<offset + 4]) == [0x50, 0x4B, 0x01, 0x02]
            else { break }
            let method = u16(zip, offset + 10)
            let compressedSize = Int(u32(zip, offset + 20))
            let uncompressedSize = Int(u32(zip, offset + 24))
            let nameLen = Int(u16(zip, offset + 28))
            let extraLen = Int(u16(zip, offset + 30))
            let commentLen = Int(u16(zip, offset + 32))
            let localOffset = Int(u32(zip, offset + 42))
            let name = String(decoding: zip[offset + 46..<offset + 46 + nameLen], as: UTF8.self)
            offset += 46 + nameLen + extraLen + commentLen

            let ext = (name as NSString).pathExtension.lowercased()
            guard ["gba", "gbc", "gb"].contains(ext), !name.hasPrefix("__MACOSX") else { continue }

            // Local header: data begins after its own variable-length fields.
            guard localOffset + 30 <= zip.count else { continue }
            let lNameLen = Int(u16(zip, localOffset + 26))
            let lExtraLen = Int(u16(zip, localOffset + 28))
            let dataStart = localOffset + 30 + lNameLen + lExtraLen
            guard dataStart + compressedSize <= zip.count else { continue }
            let payload = zip.subdata(in: dataStart..<dataStart + compressedSize)

            if method == 0 {
                return payload
            }
            if method == 8, let out = inflate(payload, capacity: uncompressedSize) {
                return out
            }
        }
        throw LoadError.zipHasNoROM
    }

    /// Raw DEFLATE via the Compression framework (zip stores raw deflate).
    private static func inflate(_ data: Data, capacity: Int) -> Data? {
        guard capacity > 0 else { return nil }
        var out = Data(count: capacity)
        let written = out.withUnsafeMutableBytes { dst -> Int in
            data.withUnsafeBytes { src -> Int in
                compression_decode_buffer(
                    dst.bindMemory(to: UInt8.self).baseAddress!, capacity,
                    src.bindMemory(to: UInt8.self).baseAddress!, data.count,
                    nil, COMPRESSION_ZLIB
                )
            }
        }
        guard written > 0 else { return nil }
        return out.prefix(written)
    }

    private static func u16(_ d: Data, _ i: Int) -> UInt16 {
        UInt16(d[i]) | UInt16(d[i + 1]) << 8
    }

    private static func u32(_ d: Data, _ i: Int) -> UInt32 {
        UInt32(d[i]) | UInt32(d[i + 1]) << 8 | UInt32(d[i + 2]) << 16 | UInt32(d[i + 3]) << 24
    }
}
