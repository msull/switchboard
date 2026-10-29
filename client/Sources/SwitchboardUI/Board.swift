import SwiftUI
import SwitchboardClientCore
import SwitchboardProtocol

// MARK: The model

/// A pin board: one window holding live tiles of sessions and files
/// from any host, all one size in a grid, in an order the user sets by
/// dragging, kept in the presentation state so a board closed and
/// opened again, or an app relaunched, shows the same tiles in the
/// same order.
public struct Board: Codable, Equatable, Sendable, Identifiable {
    public let id: UUID
    public var name: String
    public var tiles: [BoardTile]
    public var created: Date
    /// Tiles across; the rows follow from the count.
    public var columns: Int

    public init(id: UUID = UUID(), name: String, tiles: [BoardTile] = [], created: Date = Date(), columns: Int = 3) {
        self.id = id
        self.name = name
        self.tiles = tiles
        self.created = created
        self.columns = columns
    }

    enum CodingKeys: String, CodingKey {
        case id, name, tiles, created, columns
    }

    /// A board saved before the grid had a `columns` field.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UUID.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        tiles = try c.decodeIfPresent([BoardTile].self, forKey: .tiles) ?? []
        created = try c.decodeIfPresent(Date.self, forKey: .created) ?? Date()
        columns = try c.decodeIfPresent(Int.self, forKey: .columns) ?? 3
    }

    /// The column counts a board may have.
    public static let columnChoices = [2, 3, 4]
}

/// What a tile shows.
public enum BoardItem: Codable, Equatable, Hashable, Sendable {
    case session(SessionRef)
    case file(FileRef)
}

public struct BoardTile: Codable, Equatable, Sendable, Identifiable {
    public let id: UUID
    public let item: BoardItem

    public init(id: UUID = UUID(), item: BoardItem) {
        self.id = id
        self.item = item
    }

    enum CodingKeys: String, CodingKey {
        case id, item
    }
}

/// The grid and the reorder, pure so it is tested.
public enum BoardLayout {
    /// Rows the tiles take at `columns` across.
    public static func rows(count: Int, columns: Int) -> Int {
        max(1, (count + max(columns, 1) - 1) / max(columns, 1))
    }

    /// The size of every tile on a board of `size`: a column's share
    /// of the width, and the height shared by the rows, at most two
    /// in view, so a board with more rows scrolls.
    public static func tileSize(count: Int, columns: Int, in size: CGSize, spacing: CGFloat) -> CGSize {
        let columns = CGFloat(max(columns, 1))
        let rows = CGFloat(min(Self.rows(count: count, columns: Int(columns)), 2))
        return CGSize(
            width: max(0, (size.width - spacing * (columns - 1)) / columns),
            height: max(0, (size.height - spacing * (rows - 1)) / rows))
    }

    /// The top-left of slot `index` in the grid.
    public static func origin(of index: Int, columns: Int, tile: CGSize, spacing: CGFloat) -> CGPoint {
        let columns = max(columns, 1)
        return CGPoint(
            x: CGFloat(index % columns) * (tile.width + spacing),
            y: CGFloat(index / columns) * (tile.height + spacing))
    }

    /// The slot whose cell holds `point`, clamped to the tiles there
    /// are: where a tile dragged to `point` would land.
    public static func slot(at point: CGPoint, count: Int, columns: Int, tile: CGSize, spacing: CGFloat) -> Int {
        guard count > 0, tile.width > 0, tile.height > 0 else { return 0 }
        let columns = max(columns, 1)
        let column = Int((point.x / (tile.width + spacing)).rounded(.down))
        let row = Int((point.y / (tile.height + spacing)).rounded(.down))
        let index = max(row, 0) * columns + min(max(column, 0), columns - 1)
        return min(max(index, 0), count - 1)
    }

    /// `tiles` with the one at `from` moved to `to`.
    public static func reordered<T>(_ tiles: [T], from: Int, to: Int) -> [T] {
        guard tiles.indices.contains(from), tiles.indices.contains(to), from != to else { return tiles }
        var out = tiles
        let moved = out.remove(at: from)
        out.insert(moved, at: to)
        return out
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

/// A board's window: its tiles in a grid, each dragged by its bar to a
/// new place in the order, popped out to a window of its own by a tap,
/// and taken off the board by its cross.
public struct BoardView: View {
    @Bindable var model: BrowserModel
    let request: BoardWindowRequest
    @Environment(\.openWindow) private var openWindow
    @State private var renaming = false
    @State private var newName = ""
    /// The tile being dragged, where its bar was taken, and how far it
    /// has moved; the others slide to show where it will land.
    @State private var drag: Drag?

    struct Drag: Equatable {
        let tile: UUID
        let from: Int
        var translation: CGSize
        var to: Int
    }

    static let spacing: CGFloat = 12
    /// The grid's coordinate space, which a drag reports its place in.
    static let space = "board"

    public init(model: BrowserModel, request: BoardWindowRequest) {
        self.model = model
        self.request = request
    }

    private var board: Board? { model.boards.first { $0.id == request.board } }

    public var body: some View {
        VStack(spacing: 8) {
            header
            GeometryReader { geometry in
                let tiles = board?.tiles ?? []
                let columns = board?.columns ?? 3
                let tile = BoardLayout.tileSize(count: tiles.count, columns: columns, in: geometry.size, spacing: Self.spacing)
                let rows = BoardLayout.rows(count: tiles.count, columns: columns)
                let shown = order(tiles)
                let grid = ZStack(alignment: .topLeading) {
                    Color.clear.frame(
                        width: geometry.size.width,
                        height: CGFloat(rows) * tile.height + CGFloat(rows - 1) * Self.spacing)
                    ForEach(tiles) { item in
                        let index = shown.firstIndex { $0.id == item.id } ?? 0
                        let origin = BoardLayout.origin(of: index, columns: columns, tile: tile, spacing: Self.spacing)
                        let lifted = drag?.tile == item.id
                        TileView(model: model, board: request.board, tile: item, lifted: lifted) { gesture in
                            dragged(item, gesture, tiles: tiles, columns: columns, tile: tile)
                        } ended: {
                            dropped(tiles: tiles)
                        }
                        .frame(width: tile.width, height: tile.height)
                        .offset(
                            x: origin.x + (lifted ? drag?.translation.width ?? 0 : 0),
                            y: origin.y + (lifted ? drag?.translation.height ?? 0 : 0))
                        .zIndex(lifted ? 1 : 0)
                        .animation(lifted ? nil : .snappy, value: index)
                    }
                }
                .coordinateSpace(name: Self.space)
                // A scroll view clips what leaves its content, which is
                // where a dragged tile goes, so the grid scrolls only
                // when it must.
                if rows > 2 {
                    ScrollView { grid }
                } else {
                    grid
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

    /// The tiles in the order they are shown: the saved order, with the
    /// dragged one moved to where it would land.
    private func order(_ tiles: [BoardTile]) -> [BoardTile] {
        guard let drag else { return tiles }
        return BoardLayout.reordered(tiles, from: drag.from, to: drag.to)
    }

    private func dragged(_ item: BoardTile, _ gesture: DragGesture.Value, tiles: [BoardTile], columns: Int, tile: CGSize) {
        guard let from = tiles.firstIndex(where: { $0.id == item.id }) else { return }
        // The slot under the finger is where the tile lands.
        let to = BoardLayout.slot(
            at: gesture.location, count: tiles.count, columns: columns, tile: tile, spacing: Self.spacing)
        if drag?.tile != item.id {
            drag = Drag(tile: item.id, from: from, translation: gesture.translation, to: to)
        } else {
            drag?.translation = gesture.translation
            if drag?.to != to { withAnimation(.snappy) { drag?.to = to } }
        }
    }

    private func dropped(tiles: [BoardTile]) {
        guard let drag else { return }
        withAnimation(.snappy) {
            model.moveTile(drag.tile, on: request.board, to: drag.to)
            self.drag = nil
        }
    }

    private var header: some View {
        HStack {
            Text(board?.name ?? "Board").font(.title2.bold())
            Spacer()
            Picker("Columns", selection: columns) {
                ForEach(Board.columnChoices, id: \.self) { n in
                    Text("\(n)").tag(n)
                }
            }
            .pickerStyle(.segmented)
            .frame(width: 150)
            .help("Tiles across")
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

    private var columns: Binding<Int> {
        Binding(
            get: { board?.columns ?? 3 },
            set: { model.setColumns($0, on: request.board) })
    }
}

/// One tile: a bar to drag it by, with its name, a button to open it
/// as a window, and a cross; the content below it, live. Lifted, it
/// grows a little, casts a shadow and takes a border, so the one in
/// hand is unmistakable.
struct TileView: View {
    @Bindable var model: BrowserModel
    let board: UUID
    let tile: BoardTile
    let lifted: Bool
    let changed: (DragGesture.Value) -> Void
    let ended: () -> Void
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(spacing: 0) {
            bar
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .clipped()
        }
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
        .overlay {
            RoundedRectangle(cornerRadius: 14)
                .strokeBorder(Color.accentColor, lineWidth: lifted ? 3 : 0)
        }
        .scaleEffect(lifted ? 1.04 : 1)
        .shadow(color: .black.opacity(lifted ? 0.35 : 0), radius: lifted ? 24 : 0, y: lifted ? 12 : 0)
        .animation(.snappy, value: lifted)
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
        .padding(.vertical, 8)
        .background(lifted ? AnyShapeStyle(Color.accentColor.opacity(0.25)) : AnyShapeStyle(.thinMaterial))
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 6, coordinateSpace: .named(BoardView.space))
                .onChanged { changed($0) }
                .onEnded { _ in ended() })
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
