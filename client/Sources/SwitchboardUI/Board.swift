import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

// MARK: The model

/// A pin board: one window holding small live tiles of sessions and
/// files from any host, each at a place and size of its own, kept in
/// the presentation state so a board closed and opened again, or an
/// app relaunched, shows the same arrangement. Tiles are placed in
/// fractions of the board, so a board resized keeps every tile's share
/// of it.
public struct Board: Codable, Equatable, Sendable, Identifiable {
    public let id: UUID
    public var name: String
    public var tiles: [BoardTile]
    public var created: Date

    public init(id: UUID = UUID(), name: String, tiles: [BoardTile] = [], created: Date = Date()) {
        self.id = id
        self.name = name
        self.tiles = tiles
        self.created = created
    }
}

/// What a tile shows.
public enum BoardItem: Codable, Equatable, Hashable, Sendable {
    case session(SessionRef)
    case file(FileRef)
}

public struct BoardTile: Codable, Equatable, Sendable, Identifiable {
    public let id: UUID
    public let item: BoardItem
    public var frame: UnitRect

    public init(id: UUID = UUID(), item: BoardItem, frame: UnitRect) {
        self.id = id
        self.item = item
        self.frame = frame
    }
}

/// A rectangle in fractions of the board's width and height.
public struct UnitRect: Codable, Equatable, Sendable {
    public var x: Double
    public var y: Double
    public var w: Double
    public var h: Double

    public init(x: Double, y: Double, w: Double, h: Double) {
        self.x = x
        self.y = y
        self.w = w
        self.h = h
    }

    /// The least a tile can be, as a share of the board.
    public static let minimum = 0.12

    /// The rectangle kept within the board, at least the minimum size.
    public func clamped() -> UnitRect {
        var r = self
        r.w = min(max(r.w, Self.minimum), 1)
        r.h = min(max(r.h, Self.minimum), 1)
        r.x = min(max(r.x, 0), 1 - r.w)
        r.y = min(max(r.y, 0), 1 - r.h)
        return r
    }

    /// The rectangle in points on a board of `size`.
    public func rect(in size: CGSize) -> CGRect {
        CGRect(x: x * size.width, y: y * size.height, width: w * size.width, height: h * size.height)
    }
}

/// Where tiles go and how they move, pure so it is tested.
public enum BoardLayout {
    static let columns = 3
    static let rows = 2

    /// Slot `n` of the six: a third of the board wide and half of it
    /// tall, left to right then down.
    static func slot(_ n: Int) -> UnitRect {
        let n = n % (columns * rows)
        let column = Double(n % columns)
        let row = Double(n / columns)
        return UnitRect(x: column / Double(columns), y: row / Double(rows), w: 1 / Double(columns), h: 1 / Double(rows))
    }

    /// The place of the next tile: the first of the six slots that no
    /// tile already covers by more than a quarter, so a tile taken off
    /// the board leaves a gap the next one fills; when every slot is
    /// covered, a step down and right from the slot after the last
    /// tile's, so the new tile shows on top rather than hiding under
    /// another at the same place.
    public static func place(avoiding tiles: [UnitRect]) -> UnitRect {
        for n in 0..<(columns * rows) {
            let candidate = slot(n)
            let area = candidate.w * candidate.h
            let covered = tiles.contains { tile in
                let w = max(0, min(candidate.x + candidate.w, tile.x + tile.w) - max(candidate.x, tile.x))
                let h = max(0, min(candidate.y + candidate.h, tile.y + tile.h) - max(candidate.y, tile.y))
                return w * h > area / 4
            }
            if !covered { return candidate }
        }
        var r = slot(tiles.count)
        let step = 0.04 * Double(tiles.count / (columns * rows))
        r.x += step
        r.y += step
        return r.clamped()
    }

    /// `frame` moved by `translation` points on a board of `size`.
    public static func moved(_ frame: UnitRect, by translation: CGSize, in size: CGSize) -> UnitRect {
        guard size.width > 0, size.height > 0 else { return frame }
        var r = frame
        r.x += translation.width / size.width
        r.y += translation.height / size.height
        return r.clamped()
    }

    /// `frame` with its far corner dragged by `translation` points.
    public static func resized(_ frame: UnitRect, by translation: CGSize, in size: CGSize) -> UnitRect {
        guard size.width > 0, size.height > 0 else { return frame }
        var r = frame
        r.w += translation.width / size.width
        r.h += translation.height / size.height
        r.w = min(r.w, 1 - r.x)
        r.h = min(r.h, 1 - r.y)
        return r.clamped()
    }
}

/// What a board window presents: the board, by id.
public struct BoardWindowRequest: Codable, Hashable, Sendable {
    public let board: UUID

    public init(board: UUID) {
        self.board = board
    }
}

// MARK: The window

/// A board's window: its tiles over a canvas, each dragged by its bar
/// and resized by its corner, popped out to a window of its own by a
/// tap, and taken off the board by its cross.
public struct BoardView: View {
    @Bindable var model: BrowserModel
    let request: BoardWindowRequest
    @Environment(\.openWindow) private var openWindow
    @State private var renaming = false
    @State private var newName = ""

    public init(model: BrowserModel, request: BoardWindowRequest) {
        self.model = model
        self.request = request
    }

    private var board: Board? { model.boards.first { $0.id == request.board } }

    public var body: some View {
        VStack(spacing: 8) {
            header
            GeometryReader { geometry in
                ZStack(alignment: .topLeading) {
                    Color.clear
                    ForEach(board?.tiles ?? []) { tile in
                        TileView(model: model, board: request.board, tile: tile, size: geometry.size)
                    }
                }
            }
        }
        .padding(12)
        .navigationTitle(board?.name ?? "Board")
        .onAppear { model.boardAppeared(request.board) }
        .onDisappear { model.boardClosed(request.board) }
        .alert("Rename the board", isPresented: $renaming) {
            TextField("Name", text: $newName)
            Button("Rename") { model.renameBoard(request.board, to: newName) }
            Button("Cancel", role: .cancel) {}
        }
    }

    private var header: some View {
        HStack {
            Text(board?.name ?? "Board").font(.title2.bold())
            Spacer()
            AddSessionMenu(model: model, board: request.board).equatable()
            Button {
                newName = board?.name ?? ""
                renaming = true
            } label: {
                Image(systemName: "pencil")
            }
            .accessibilityLabel("Rename")
            Button {
                openWindow(id: MainWindow.id, value: MainWindow.value)
            } label: {
                Image(systemName: "rectangle.grid.2x2")
            }
            .accessibilityLabel("Sessions")
            .help("The main window, with every session and board")
        }
    }
}

/// One tile: a bar to drag it by, with its name, a button to open it
/// as a window, and a cross; the content below it, live; a handle at
/// the far corner to resize it.
struct TileView: View {
    @Bindable var model: BrowserModel
    let board: UUID
    let tile: BoardTile
    let size: CGSize
    @Environment(\.openWindow) private var openWindow
    @State private var dragging: CGSize = .zero
    @State private var resizing: CGSize = .zero

    private var frame: CGRect {
        var unit = tile.frame
        if dragging != .zero { unit = BoardLayout.moved(unit, by: dragging, in: size) }
        if resizing != .zero { unit = BoardLayout.resized(unit, by: resizing, in: size) }
        return unit.rect(in: size)
    }

    var body: some View {
        VStack(spacing: 0) {
            bar
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .clipped()
        }
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
        .overlay(alignment: .bottomTrailing) { handle }
        .frame(width: frame.width, height: frame.height)
        .offset(x: frame.minX, y: frame.minY)
    }

    private var bar: some View {
        HStack(spacing: 6) {
            Image(systemName: "line.3.horizontal").foregroundStyle(.secondary)
            Text(title).font(.caption.bold()).lineLimit(1)
            Spacer(minLength: 0)
            Button {
                switch tile.item {
                case .session(let ref): openWindow(value: model.windowRequest(for: ref))
                case .file(let ref): openWindow(value: model.fileWindowRequest(for: ref, line: nil))
                }
            } label: {
                Image(systemName: "arrow.up.forward.square").font(.caption)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Open as a window")
            Button {
                model.removeTile(tile.id, from: board)
            } label: {
                Image(systemName: "xmark").font(.caption)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Take off the board")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.thinMaterial)
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 4)
                .onChanged { dragging = $0.translation }
                .onEnded { value in
                    dragging = .zero
                    model.moveTile(tile.id, on: board, by: value.translation, in: size)
                })
    }

    private var handle: some View {
        Image(systemName: "arrow.up.left.and.arrow.down.right")
            .font(.caption2)
            .foregroundStyle(.secondary)
            .padding(6)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 4)
                    .onChanged { resizing = $0.translation }
                    .onEnded { value in
                        resizing = .zero
                        model.resizeTile(tile.id, on: board, by: value.translation, in: size)
                    })
            .accessibilityLabel("Resize")
    }

    private var title: String {
        switch tile.item {
        case .session(let ref):
            return model.store.sessions.first { $0.ref == ref }?.session.name ?? ref.recordID
        case .file(let ref):
            return ref.path.split(separator: "/").last.map(String.init) ?? ref.path
        }
    }

    @ViewBuilder
    private var content: some View {
        switch tile.item {
        case .session(let ref):
            CardWindowView(model: model, request: CardWindowRequest(id: tile.id, ref: ref), embedded: true)
        case .file(let ref):
            FileWindowView(model: model, request: FileWindowRequest(id: tile.id, ref: ref), embedded: true)
        }
    }
}

/// A session a board's add menu offers: what the menu shows and
/// nothing a pull changes every two seconds.
public struct SessionChoice: Equatable, Identifiable, Sendable {
    public let ref: SessionRef
    public let name: String
    public var id: SessionRef { ref }
}

/// The choices of one host, in the list's order.
public struct SessionChoiceGroup: Equatable, Identifiable, Sendable {
    public let host: String
    public let sessions: [SessionChoice]
    public var id: String { host }

    public static func groups(_ sessions: [UnifiedSession]) -> [SessionChoiceGroup] {
        let byHost = Dictionary(grouping: sessions, by: \.serverName)
        return byHost.keys.sorted().map { host in
            SessionChoiceGroup(
                host: host,
                sessions: (byHost[host] ?? []).map { SessionChoice(ref: $0.ref, name: $0.session.name) })
        }
    }
}

/// The menu that puts a session on a board. It reads only the choices,
/// never the store: a menu whose body is re-evaluated while open
/// scrolls back to its top, and the store changes on every pull.
struct AddSessionMenu: View, Equatable {
    @Bindable var model: BrowserModel
    let board: UUID

    static func == (lhs: AddSessionMenu, rhs: AddSessionMenu) -> Bool {
        lhs.board == rhs.board && lhs.model.sessionChoices == rhs.model.sessionChoices
    }

    var body: some View {
        Menu {
            ForEach(model.sessionChoices) { group in
                Section(group.host) {
                    ForEach(group.sessions) { choice in
                        Button(choice.name) { model.addTile(.session(choice.ref), to: board) }
                    }
                }
            }
            if model.sessionChoices.isEmpty {
                Text("No sessions from a connected host")
            }
        } label: {
            Label("Add session", systemImage: "plus")
        }
        .help("Put a session on the board; a file goes on from its window")
    }
}

/// The menu that puts a session or file on a board, from a card or a
/// file window: one entry per board, and a new board named after the
/// item.
struct PinToBoardMenu: View {
    @Bindable var model: BrowserModel
    let item: BoardItem
    let name: String

    var body: some View {
        Menu {
            ForEach(model.boards) { board in
                Button(board.name) { model.addTile(item, to: board.id) }
            }
            Divider()
            Button("New board") {
                let board = model.addBoard(named: name)
                model.addTile(item, to: board.id)
            }
        } label: {
            Image(systemName: "pin").font(.caption)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .controlSize(.mini)
        .accessibilityLabel("Pin to a board")
        .help("Put this on a board, where it stays until taken off")
    }
}
