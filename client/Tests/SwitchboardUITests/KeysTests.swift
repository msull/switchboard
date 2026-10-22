import Foundation
import XCTest

@testable import SwitchboardUI

/// The keys panel's edits: each key as a function of the field before
/// it, and the field's caret when text arrives from outside.
final class KeysTests: XCTestCase {
    private func after(_ key: PanelKey, _ text: String, caret: Int) -> EditState {
        KeyEdit.apply(key, to: EditState(text, caret: caret))
    }

    private func after(_ key: PanelKey, _ text: String, selecting: Range<Int>) -> EditState {
        KeyEdit.apply(key, to: EditState(text, selection: selecting))
    }

    func testAMarkGoesAgainstTheWordBeforeItAndSpacesTheWordAfter() {
        XCTAssertEqual(after(.period, "hello world ", caret: 12), EditState("hello world.", caret: 12))
        XCTAssertEqual(after(.comma, "hello world", caret: 5), EditState("hello, world", caret: 6))
        XCTAssertEqual(after(.question, "hello world", caret: 5), EditState("hello? world", caret: 6))
        XCTAssertEqual(after(.exclamation, "hello  world", caret: 7), EditState("hello! world", caret: 7))
        XCTAssertEqual(after(.colon, "hello", caret: 5), EditState("hello:", caret: 6))
        XCTAssertEqual(after(.semicolon, "a.", caret: 1), EditState("a;.", caret: 2))
    }

    func testABreakTakesTheSpacesOnBothSidesWithIt() {
        XCTAssertEqual(after(.newline, "one  two", caret: 4), EditState("one\ntwo", caret: 4))
        XCTAssertEqual(after(.paragraph, "one two ", caret: 8), EditState("one two\n\n", caret: 9))
        XCTAssertEqual(after(.newline, "one\n", caret: 4), EditState("one\n\n", caret: 5))
    }

    func testDeleteTakesACharacterOrAWordAndASelectionFirst() {
        XCTAssertEqual(after(.deleteBackward, "héllo", caret: 2), EditState("hllo", caret: 1))
        XCTAssertEqual(after(.deleteBackward, "hello", caret: 0), EditState("hello", caret: 0))
        XCTAssertEqual(after(.deleteWord, "one two  ", caret: 9), EditState("one ", caret: 4))
        XCTAssertEqual(after(.deleteWord, "one two", caret: 5), EditState("one wo", caret: 4))
        XCTAssertEqual(after(.deleteWord, "one\ntwo", caret: 7), EditState("one\n", caret: 4))
        XCTAssertEqual(after(.deleteBackward, "hello world", selecting: 2..<7), EditState("heorld", caret: 2))
        XCTAssertEqual(after(.deleteWord, "hello world", selecting: 2..<7), EditState("heorld", caret: 2))
        XCTAssertEqual(after(.period, "hello world", selecting: 5..<11), EditState("hello.", caret: 6))
    }

    func testTheCaretMovesByCharacterAndToTheEndsOfLineAndText() {
        XCTAssertEqual(after(.left, "ab", caret: 1).caret, 0)
        XCTAssertEqual(after(.left, "ab", caret: 0).caret, 0)
        XCTAssertEqual(after(.right, "ab", caret: 1).caret, 2)
        XCTAssertEqual(after(.right, "ab", caret: 2).caret, 2)
        XCTAssertEqual(after(.left, "abcd", selecting: 1..<3).selection, 1..<1)
        XCTAssertEqual(after(.right, "abcd", selecting: 1..<3).selection, 3..<3)
        let two = "first line\nsecond line"
        XCTAssertEqual(after(.lineStart, two, caret: 14).caret, 11)
        XCTAssertEqual(after(.lineEnd, two, caret: 2).caret, 10)
        XCTAssertEqual(after(.lineStart, two, caret: 11).caret, 11)
        XCTAssertEqual(after(.start, two, caret: 14).caret, 0)
        XCTAssertEqual(after(.end, two, caret: 2).caret, two.utf16.count)
        // A flag is one character of four UTF-16 units.
        XCTAssertEqual(after(.left, "🇫🇷!", caret: 5).caret, 4)
        XCTAssertEqual(after(.left, "🇫🇷!", caret: 4).caret, 0)
        XCTAssertEqual(after(.deleteBackward, "a🇫🇷", caret: 5), EditState("a", caret: 1))
    }

    func testAStateClampsItsSelectionToTheText() {
        XCTAssertEqual(EditState("abc", caret: 9).caret, 3)
        XCTAssertEqual(EditState("abc", selection: 2..<9).selection, 2..<3)
        XCTAssertEqual(EditState("abc", caret: -1).caret, 0)
    }

    @MainActor func testAKeyPressedOnTheFieldPlacesItsCaretAndOutsideTextKeepsIt() {
        let field = EditField()
        field.text = "hello world"
        XCTAssertEqual(field.caret, 0..<0)
        field.press(.end)
        XCTAssertEqual(field.caret, 11..<11)
        field.press(.period)
        XCTAssertEqual(field.text, "hello world.")
        XCTAssertEqual(field.caret, 12..<12)
        XCTAssertNotNil(field.selection)
        // Dictation appends while the caret is at the end: it follows.
        field.set("hello world. and more")
        XCTAssertEqual(field.caret, 21..<21)
        // A caret inside the text keeps its place when text arrives.
        field.press(.lineStart)
        field.press(.right)
        field.set("hello world. and more still")
        XCTAssertEqual(field.caret, 1..<1)
        // Text that got shorter clamps it.
        field.press(.end)
        field.press(.left)
        field.set("hi")
        XCTAssertEqual(field.caret, 2..<2)
        // The same text again changes nothing.
        field.set("hi")
        XCTAssertEqual(field.text, "hi")
    }

    @MainActor func testTheHostsTextIsTakenAsTheBaselineForTheNextEdit() {
        let field = EditField()
        field.takeHost("from the host")
        XCTAssertEqual(field.text, "from the host")
        XCTAssertEqual(field.hostText, "from the host")
        XCTAssertFalse(field.editing)
        field.press(.exclamation)
        XCTAssertEqual(field.text, "from the host!")
        XCTAssertNotEqual(field.text, field.hostText)
    }

    func testTheRowsHoldEveryKeyOnce() {
        let laid = PanelKey.rows.flatMap { $0 }
        XCTAssertEqual(Set(laid).count, laid.count)
        XCTAssertEqual(Set(laid), Set(PanelKey.allCases))
    }
}
