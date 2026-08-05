import Foundation
import SwiftUI

/// The app's home screen: every ROM the app knows about, most recent first.
/// Tapping a row starts it; ejecting a game comes back here.
struct LibraryView: View {
    let games: [Game]
    let onPlay: (Game) -> Void
    let onImport: () -> Void
    let onDelete: (Game) -> Void

    @State private var pendingDelete: Game?

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            VStack(spacing: 0) {
                header
                if games.isEmpty {
                    emptyState
                } else {
                    list
                }
            }
        }
        .alert("Delete \(pendingDelete?.title ?? "")?", isPresented: deleteAlertBinding) {
            Button("Cancel", role: .cancel) { pendingDelete = nil }
            Button("Delete", role: .destructive) {
                if let game = pendingDelete { onDelete(game) }
                pendingDelete = nil
            }
        } message: {
            Text(deleteMessage)
        }
    }

    // MARK: - Pieces

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Library")
                    .font(.system(size: 34, weight: .heavy, design: .rounded))
                    .foregroundStyle(.white)
                if !games.isEmpty {
                    Text(games.count == 1 ? "1 game" : "\(games.count) games")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            Button(action: onImport) {
                Label("Import", systemImage: "plus")
                    .font(.subheadline.bold())
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.indigo, in: Capsule())
                    .foregroundStyle(.white)
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 12)
        .padding(.bottom, 16)
    }

    private var list: some View {
        List {
            ForEach(Array(games.enumerated()), id: \.element.id) { index, game in
                Button { onPlay(game) } label: {
                    GameRow(game: game, isContinue: index == 0 && game.lastPlayed != nil)
                }
                .listRowBackground(Color.black)
                .listRowSeparatorTint(Color.white.opacity(0.08))
                .listRowInsets(EdgeInsets(top: 10, leading: 20, bottom: 10, trailing: 20))
                .swipeActions(edge: .trailing) {
                    Button(role: .destructive) { pendingDelete = game } label: {
                        Label("Delete", systemImage: "trash")
                    }
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    private var emptyState: some View {
        VStack(spacing: 14) {
            Spacer()
            Image(systemName: "tray")
                .font(.system(size: 44, weight: .light))
                .foregroundStyle(.secondary)
            Text("No games yet")
                .font(.title3.bold())
                .foregroundStyle(.white)
            Text("Import a .gba or .zip file to get started.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Spacer()
            Spacer()
        }
        .padding(.horizontal, 40)
    }

    // MARK: - Delete confirmation

    private var deleteAlertBinding: Binding<Bool> {
        Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } })
    }

    private var deleteMessage: String {
        guard let game = pendingDelete else { return "" }
        if game.hasSave || game.hasState {
            return "This also deletes your in-game save and save state for this game. It can't be undone."
        }
        return "This removes the ROM from the app. It can't be undone."
    }
}

/// A single library row: cartridge tile, title, and what's on disk for it.
private struct GameRow: View {
    let game: Game
    let isContinue: Bool

    var body: some View {
        HStack(spacing: 14) {
            cartridge
            VStack(alignment: .leading, spacing: 3) {
                Text(game.title)
                    .font(.system(size: 17, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
                    .lineLimit(1)
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            if isContinue {
                Text("CONTINUE")
                    .font(.system(size: 10, weight: .bold, design: .rounded))
                    .tracking(0.8)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(Color.indigo.opacity(0.25), in: Capsule())
                    .foregroundStyle(Color.indigo)
            }
            Image(systemName: "chevron.right")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.tertiary)
        }
        .contentShape(Rectangle())
    }

    private var cartridge: some View {
        RoundedRectangle(cornerRadius: 10, style: .continuous)
            .fill(
                LinearGradient(
                    colors: [Color.indigo.opacity(0.9), Color.indigo.opacity(0.45)],
                    startPoint: .topLeading, endPoint: .bottomTrailing
                )
            )
            .frame(width: 46, height: 46)
            .overlay(
                Text(game.code.isEmpty ? game.initials : game.code)
                    .font(.system(size: game.code.isEmpty ? 16 : 11, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(Color.white.opacity(0.12), lineWidth: 1)
            )
    }

    private var subtitle: String {
        var parts: [String] = []
        if let note = game.fileNote { parts.append(note) }
        if let played = game.lastPlayed {
            parts.append("Played \(played.formatted(.relative(presentation: .named)))")
        } else {
            parts.append("Never played")
        }
        if game.hasSave { parts.append("Save") }
        if game.hasState { parts.append("State") }
        return parts.joined(separator: " · ")
    }
}
