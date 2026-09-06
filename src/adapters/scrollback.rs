//! Reading the raw VT stream `pipe-pane` writes for a session, as plain
//! text: the last lines of what the pane showed, for records whose pane
//! is gone (after a reboot, say). Escape sequences are stripped rather
//! than interpreted, which is right for line-oriented output and rough
//! for full-screen programs; the design's parser-fed history is later.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How much of the end of the file is read.
const TAIL_BYTES: u64 = 128 * 1024;

/// The last `max_lines` non-blank lines of the stream at `path`.
///
/// # Errors
/// The file cannot be opened or read.
pub fn tail_text(path: &Path, max_lines: usize) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let plain = strip_escapes(&text);
    let lines: Vec<&str> = plain.lines().filter(|l| !l.trim().is_empty()).collect();
    let keep = lines.len().saturating_sub(max_lines);
    Ok(lines[keep..].join("\n"))
}

/// Drop ANSI escape sequences and control characters, honoring `\r` as
/// "overwrite this line from the start" the way a terminal would.
#[must_use]
pub fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut line = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parameters and intermediates, then a final byte.
                Some('[') => {
                    for d in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&d) {
                            break;
                        }
                    }
                }
                // OSC: until BEL or ESC \.
                Some(']') => {
                    let mut prev = ' ';
                    for d in chars.by_ref() {
                        if d == '\u{7}' || (prev == '\u{1b}' && d == '\\') {
                            break;
                        }
                        prev = d;
                    }
                }
                // Charset designations take one more byte (ESC ( B).
                Some('(' | ')' | '*' | '+' | '#') => {
                    chars.next();
                }
                // Other two-character escapes (keypad modes, ...).
                Some(_) | None => {}
            },
            '\n' => {
                out.push_str(&line);
                out.push('\n');
                line.clear();
            }
            // CR LF is a plain newline; a lone CR overwrites the line.
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    line.clear();
                }
            }
            '\t' => line.push_str("    "),
            c if c.is_control() => {}
            c => line.push(c),
        }
    }
    out.push_str(&line);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_osc_and_honors_carriage_return() {
        let raw = "\u{1b}[32mgreen\u{1b}[0m text\n\u{1b}]0;title\u{7}progress 10%\rprogress 100%\n\u{1b}(Bdone";
        assert_eq!(strip_escapes(raw), "green text\nprogress 100%\ndone");
    }

    #[test]
    fn tail_keeps_the_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.vt");
        std::fs::write(&p, "one\n\ntwo\r\nthree\n\u{1b}[1mfour\u{1b}[0m\n").unwrap();
        assert_eq!(tail_text(&p, 2).unwrap(), "three\nfour");
        assert_eq!(tail_text(&p, 10).unwrap(), "one\ntwo\nthree\nfour");
    }
}
