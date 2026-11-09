import Foundation

/// `GET sessions/{id}/files`: the files a session's tool calls wrote
/// under its project root, newest first, with their line counts against
/// the base of the checkout it works in. No base when git could not read
/// the checkout; the files are then listed without counts.
public struct EditedFilesBody: Decodable, Sendable, Equatable {
    public let base: DiffBase?
    public let files: [EditedFile]

    public init(base: DiffBase?, files: [EditedFile]) {
        self.base = base
        self.files = files
    }
}

/// What the counts are against: where the branch split from the default
/// branch (`onto` names it), or the commit the checkout was at when the
/// session started.
public struct DiffBase: Codable, Sendable, Equatable, Hashable {
    public let sha: String
    /// `branch` or `session`.
    public let kind: String
    public let onto: String?

    public init(sha: String, kind: String, onto: String? = nil) {
        self.sha = sha
        self.kind = kind
        self.onto = onto
    }

    /// A short line for the head of the list.
    public var label: String {
        if kind == "branch", let onto { return "Since branching from \(onto)" }
        return "Since the session started"
    }
}

/// One file the session wrote; `path` is under the project root.
public struct EditedFile: Decodable, Sendable, Equatable, Identifiable {
    public let path: String
    /// In the checkout the base is from, so it has counts and a diff.
    public let inRepo: Bool
    /// Differs from the base; a file written back to what it was does not.
    public let changed: Bool
    public let added: Int
    public let removed: Int
    public let binary: Bool

    public var id: String { path }

    public init(
        path: String, inRepo: Bool = true, changed: Bool = true, added: Int = 0, removed: Int = 0,
        binary: Bool = false
    ) {
        self.path = path
        self.inRepo = inRepo
        self.changed = changed
        self.added = added
        self.removed = removed
        self.binary = binary
    }

    enum CodingKeys: String, CodingKey {
        case path, changed, added, removed, binary
        case inRepo = "in_repo"
    }
}

/// `GET sessions/{id}/diff/{base}/{path}`: one file against the base.
public struct FileDiffBody: Decodable, Sendable, Equatable {
    /// `modified`, `added`, `deleted` or `renamed`.
    public let status: String
    public let body: DiffContent

    public init(status: String, body: DiffContent) {
        self.status = status
        self.body = body
    }
}

/// A diff's content: hunks, or why there are none to show.
public struct DiffContent: Decodable, Sendable, Equatable {
    /// `hunks`, `binary`, `too_large` or `empty`.
    public let kind: String
    public let hunks: [DiffHunk]?
    /// For `too_large`: how many lines the diff had.
    public let lines: Int?

    public init(kind: String, hunks: [DiffHunk]? = nil, lines: Int? = nil) {
        self.kind = kind
        self.hunks = hunks
        self.lines = lines
    }
}

public struct DiffHunk: Decodable, Sendable, Equatable {
    public let oldStart: Int
    public let newStart: Int
    public let lines: [DiffLine]

    public init(oldStart: Int, newStart: Int, lines: [DiffLine]) {
        self.oldStart = oldStart
        self.newStart = newStart
        self.lines = lines
    }

    enum CodingKeys: String, CodingKey {
        case lines
        case oldStart = "old_start"
        case newStart = "new_start"
    }
}

/// One line of a hunk, numbered on the sides it is on. `marks` are
/// UTF-8 byte ranges of `text` that changed from the line it replaced.
public struct DiffLine: Decodable, Sendable, Equatable {
    /// `context`, `added` or `removed`.
    public let kind: String
    public let old: Int?
    public let new: Int?
    public let text: String
    public let marks: [[Int]]

    public init(kind: String, old: Int? = nil, new: Int? = nil, text: String, marks: [[Int]] = []) {
        self.kind = kind
        self.old = old
        self.new = new
        self.text = text
        self.marks = marks
    }
}
