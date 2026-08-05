import Foundation

/// One playable ROM sitting in the app's Documents folder.
struct Game: Identifiable, Hashable {
    let url: URL
    /// Title from the ROM header, falling back to the file name.
    let title: String
    /// Four-character header game code (e.g. BPRE for FireRed); "" for zips.
    let code: String
    /// File name, shown only when another game carries the same title.
    let fileNote: String?
    let lastPlayed: Date?
    let hasSave: Bool
    let hasState: Bool

    var id: URL { url }

    /// Base name shared by the ROM, its battery save and its save state.
    var base: String { url.deletingPathExtension().lastPathComponent }

    /// Two letters for the cartridge tile when there's no game code.
    var initials: String {
        let letters = title.split(separator: " ").compactMap(\.first)
        return String(letters.prefix(2)).uppercased()
    }
}

/// Scans, imports and deletes the ROMs in Documents. The emulator names its
/// save files after the ROM's base name, so deletion has to sweep those too.
enum GameLibrary {
    static let playableExtensions = ["gba", "zip"]

    static var documents: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
    }

    // MARK: - Reading

    static func scan() -> [Game] {
        let fm = FileManager.default
        let files = (try? fm.contentsOfDirectory(at: documents, includingPropertiesForKeys: nil)) ?? []
        let found = files
            .filter { playableExtensions.contains($0.pathExtension.lowercased()) }
            .map(game(at:))
        return disambiguate(found)
            // Most recently played first so "continue where I left off" is the
            // top row; never-played ROMs fall to the bottom alphabetically.
            .sorted { a, b in
                switch (a.lastPlayed, b.lastPlayed) {
                case let (x?, y?): return x > y
                case (_?, nil): return true
                case (nil, _?): return false
                case (nil, nil): return a.title.localizedCaseInsensitiveCompare(b.title) == .orderedAscending
                }
            }
    }

    /// Two copies of the same game share a header title, so a plain list shows
    /// two identical rows (a clean ROM and a romhack, say). Tag the colliding
    /// ones with their file name. It goes in the subtitle rather than the
    /// title, which truncates at exactly the part that tells them apart.
    private static func disambiguate(_ games: [Game]) -> [Game] {
        var counts: [String: Int] = [:]
        for game in games { counts[game.title, default: 0] += 1 }
        return games.map { game in
            guard counts[game.title, default: 0] > 1, game.title != game.base else { return game }
            return Game(
                url: game.url,
                title: game.title,
                code: game.code,
                fileNote: game.base,
                lastPlayed: game.lastPlayed,
                hasSave: game.hasSave,
                hasState: game.hasState
            )
        }
    }

    private static func game(at url: URL) -> Game {
        let fm = FileManager.default
        let base = url.deletingPathExtension().lastPathComponent
        let header = readHeader(of: url)
        return Game(
            url: url,
            title: header?.title ?? base,
            code: header?.code ?? "",
            fileNote: nil,
            lastPlayed: lastPlayed(base),
            hasSave: fm.fileExists(atPath: documents.appendingPathComponent(base + ".sav").path),
            hasState: fm.fileExists(atPath: documents.appendingPathComponent(base + ".state").path)
        )
    }

    /// Reads just the first 0xC0 bytes rather than pulling a 16 MB ROM into
    /// memory to get a name. Zips are skipped (their header is compressed).
    private static func readHeader(of url: URL) -> (title: String, code: String)? {
        guard url.pathExtension.lowercased() == "gba",
              let handle = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? handle.close() }
        guard let data = try? handle.read(upToCount: 0xC0), data.count >= 0xC0 else { return nil }

        let title = ascii(data[0xA0..<0xAC])
        let code = ascii(data[0xAC..<0xB0])
        guard !title.isEmpty else { return nil }
        // Header titles are all caps ("POKEMON FIRE"); title case reads better.
        return (title.capitalized, code)
    }

    private static func ascii(_ bytes: Data) -> String {
        let printable = bytes.prefix { $0 >= 0x20 && $0 < 0x7F }
        return String(decoding: printable, as: UTF8.self)
            .trimmingCharacters(in: .whitespaces)
    }

    // MARK: - Last played

    private static let lastPlayedKey = "lastPlayedDates"

    static func markPlayed(_ url: URL) {
        var dates = UserDefaults.standard.dictionary(forKey: lastPlayedKey) as? [String: Date] ?? [:]
        dates[url.deletingPathExtension().lastPathComponent] = Date()
        UserDefaults.standard.set(dates, forKey: lastPlayedKey)
    }

    private static func lastPlayed(_ base: String) -> Date? {
        let dates = UserDefaults.standard.dictionary(forKey: lastPlayedKey) as? [String: Date]
        return dates?[base]
    }

    // MARK: - Writing

    /// Copies a picked file into Documents, unzipping and validating first so
    /// only real ROMs ever reach the core. Returns the stored ROM's location.
    static func importROM(from picked: URL) throws -> URL {
        let scoped = picked.startAccessingSecurityScopedResource()
        defer { if scoped { picked.stopAccessingSecurityScopedResource() } }

        let rom = try ROMLoader.load(from: picked)
        let name = picked.deletingPathExtension().lastPathComponent
        var destination = documents.appendingPathComponent(name + ".gba")
        // Don't clobber an existing game (and its saves) with a same-named import.
        var attempt = 2
        while FileManager.default.fileExists(atPath: destination.path) {
            destination = documents.appendingPathComponent("\(name) \(attempt).gba")
            attempt += 1
        }
        try rom.write(to: destination)
        return destination
    }

    /// Deletes the ROM along with its battery save and save state.
    static func delete(_ game: Game) {
        let fm = FileManager.default
        for url in [game.url,
                    documents.appendingPathComponent(game.base + ".sav"),
                    documents.appendingPathComponent(game.base + ".state")] {
            try? fm.removeItem(at: url)
        }
        var dates = UserDefaults.standard.dictionary(forKey: lastPlayedKey) as? [String: Date] ?? [:]
        dates[game.base] = nil
        UserDefaults.standard.set(dates, forKey: lastPlayedKey)
    }
}
