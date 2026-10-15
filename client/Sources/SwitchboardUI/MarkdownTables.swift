import Foundation
import SwiftUI

/// A piece of what an agent wrote: prose, or one pipe table. The prose
/// view reads inline marks only, so a table would show as its pipes;
/// the tables are cut out and laid out as grids, as the Mac app does.
public enum MarkdownSegment: Equatable, Sendable {
    case prose(String)
    case table(MarkdownTable)
}

/// The cells of one table as Markdown source, inline marks included.
public struct MarkdownTable: Equatable, Sendable {
    public enum Alignment: Equatable, Sendable {
        case leading, center, trailing
    }

    public var header: [String]
    /// One per header cell, from the delimiter row's colons.
    public var alignments: [Alignment]
    public var rows: [[String]]

    public init(header: [String], alignments: [Alignment], rows: [[String]]) {
        self.header = header
        self.alignments = alignments
        self.rows = rows
    }

    public var columns: Int {
        max(header.count, rows.map(\.count).max() ?? 0)
    }

    /// The cell at `row` and `column`, empty past a short row.
    public func cell(_ row: Int, _ column: Int) -> String {
        column < rows[row].count ? rows[row][column] : ""
    }

    public func alignment(_ column: Int) -> Alignment {
        column < alignments.count ? alignments[column] : .leading
    }

    /// Cut `text` into prose and its tables, in order. A table is a
    /// line with pipes over a delimiter line with as many cells, then
    /// every line with a pipe up to a blank line; one inside a code
    /// fence stays prose. Pure, so the Mac tests cover it.
    public static func split(_ text: String) -> [MarkdownSegment] {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        var out: [MarkdownSegment] = []
        var prose: [String] = []
        var fenced = false
        var i = 0
        func flush() {
            let joined = prose.joined(separator: "\n").trimmingCharacters(in: .newlines)
            if !joined.isEmpty { out.append(.prose(joined)) }
            prose = []
        }
        while i < lines.count {
            let line = lines[i]
            if isFence(line) { fenced.toggle() }
            if !fenced, i + 1 < lines.count, line.contains("|"),
                let alignments = delimiter(lines[i + 1])
            {
                let header = cells(line)
                if header.count == alignments.count {
                    flush()
                    var rows: [[String]] = []
                    var j = i + 2
                    while j < lines.count, lines[j].contains("|"),
                        !lines[j].trimmingCharacters(in: .whitespaces).isEmpty
                    {
                        rows.append(cells(lines[j]))
                        j += 1
                    }
                    out.append(.table(MarkdownTable(header: header, alignments: alignments, rows: rows)))
                    i = j
                    continue
                }
            }
            prose.append(line)
            i += 1
        }
        flush()
        if out.isEmpty { out.append(.prose(text.trimmingCharacters(in: .newlines))) }
        return out
    }

    static func isFence(_ line: String) -> Bool {
        let t = line.trimmingCharacters(in: .whitespaces)
        return t.hasPrefix("```") || t.hasPrefix("~~~")
    }

    /// The cells of a row: split on pipes not escaped, the outer ones
    /// dropped, each trimmed, `\|` read as a pipe.
    static func cells(_ line: String) -> [String] {
        var cells: [String] = []
        var cell = ""
        var escaped = false
        for ch in line {
            if escaped {
                if ch != "|" { cell.append("\\") }
                cell.append(ch)
                escaped = false
            } else if ch == "\\" {
                escaped = true
            } else if ch == "|" {
                cells.append(cell)
                cell = ""
            } else {
                cell.append(ch)
            }
        }
        if escaped { cell.append("\\") }
        cells.append(cell)
        if cells.first?.trimmingCharacters(in: .whitespaces).isEmpty == true { cells.removeFirst() }
        if cells.count > 1, cells.last?.trimmingCharacters(in: .whitespaces).isEmpty == true { cells.removeLast() }
        return cells.map { $0.trimmingCharacters(in: .whitespaces) }
    }

    /// The alignments of a delimiter row (`| :-- | :-: | --: |`), or
    /// nil when the line is not one.
    static func delimiter(_ line: String) -> [Alignment]? {
        guard line.contains("|") || line.contains("-") else { return nil }
        let parts = cells(line)
        guard !parts.isEmpty else { return nil }
        var out: [Alignment] = []
        for part in parts {
            var body = Substring(part)
            let left = body.hasPrefix(":")
            if left { body = body.dropFirst() }
            let right = body.hasSuffix(":")
            if right { body = body.dropLast() }
            guard !body.isEmpty, body.allSatisfy({ $0 == "-" }) else { return nil }
            out.append(left && right ? .center : right ? .trailing : .leading)
        }
        return out
    }
}

/// One table as a grid: each column as wide as its content asks, cells
/// wrapping inside it, the header bold over a rule.
struct MarkdownTableView: View {
    let table: MarkdownTable
    @Environment(\.textScale) private var scale

    var body: some View {
        Grid(alignment: .topLeading, horizontalSpacing: 14, verticalSpacing: 6) {
            GridRow {
                ForEach(0..<table.columns, id: \.self) { column in
                    cell(column < table.header.count ? table.header[column] : "", column: column)
                        .bold()
                        .gridColumnAlignment(Self.horizontal(table.alignment(column)))
                }
            }
            Divider()
            ForEach(0..<table.rows.count, id: \.self) { row in
                GridRow {
                    ForEach(0..<table.columns, id: \.self) { column in
                        cell(table.cell(row, column), column: column)
                    }
                }
                if row + 1 < table.rows.count {
                    Divider().opacity(0.4)
                }
            }
        }
        .font(Scaled.prose(scale))
        .textSelection(.enabled)
    }

    private func cell(_ text: String, column: Int) -> some View {
        Text(AgentText.inline(text))
            .multilineTextAlignment(Self.text(table.alignment(column)))
            .fixedSize(horizontal: false, vertical: true)
    }

    private static func horizontal(_ a: MarkdownTable.Alignment) -> HorizontalAlignment {
        switch a {
        case .leading: .leading
        case .center: .center
        case .trailing: .trailing
        }
    }

    private static func text(_ a: MarkdownTable.Alignment) -> TextAlignment {
        switch a {
        case .leading: .leading
        case .center: .center
        case .trailing: .trailing
        }
    }
}
