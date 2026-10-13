//! Reading the raw VT stream `pipe-pane` writes for a session, as plain
//! text: the last lines of what the pane showed, for records whose pane
//! is gone (after a reboot, say). Escape sequences are stripped rather
//! than interpreted, which is right for line-oriented output and rough
//! for full-screen programs.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Where the host pipes every session's and run's raw output.
#[must_use]
pub fn scrollback_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("scrollback")
}

/// How much of the end of the file is read.
const TAIL_BYTES: u64 = 128 * 1024;

/// The last `max_lines` non-blank lines of the stream at `path`.
///
/// # Errors
/// The file cannot be opened or read.
pub fn tail_text(path: &Path, max_lines: usize) -> std::io::Result<String> {
    tail_masked(path, max_lines, &|s| s.to_owned(), 0).map(|(_, text)| text)
}

/// What a log file is at a read: inode, change time and size. A log
/// truncated and regrown to its old length has a new change time; a
/// rotated one a new inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogVersion {
    pub inode: u64,
    pub ctime: i64,
    pub ctime_nsec: i64,
    pub size: u64,
}

impl LogVersion {
    fn of(meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self {
            inode: meta.ino(),
            ctime: meta.ctime(),
            ctime_nsec: meta.ctime_nsec(),
            size: meta.len(),
        }
    }
}

/// The last `max_lines` non-blank lines of the stream at `path` with
/// `mask` over the text, and the file's version at the read. The read
/// starts `overlap` bytes before the tail window so a masked value
/// that straddles the window's first byte is masked whole; the lines
/// kept are the last ones either way.
///
/// # Errors
/// The file cannot be opened or read.
pub fn tail_masked(
    path: &Path,
    max_lines: usize,
    mask: &dyn Fn(&str) -> String,
    overlap: usize,
) -> std::io::Result<(LogVersion, String)> {
    let mut file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    let version = LogVersion::of(&meta);
    let start = meta
        .len()
        .saturating_sub(TAIL_BYTES + u64::try_from(overlap).unwrap_or(u64::MAX));
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let plain = mask(&strip_escapes(&text));
    let lines: Vec<&str> = plain.lines().filter(|l| !l.trim().is_empty()).collect();
    let keep = lines.len().saturating_sub(max_lines);
    Ok((version, lines[keep..].join("\n")))
}

/// Drop ANSI escape sequences and control characters, honoring `\r` as
/// "overwrite this line from the start" the way a terminal would.
#[must_use]
fn strip_escapes(text: &str) -> String {
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
            // CR LF is a plain newline, and so is CR CR LF, which a
            // program under a pty writes for every line (the pty adds a
            // CR to the one it printed). Only a CR with text after it
            // overwrites the line, as a terminal would.
            '\r' => {
                while chars.peek() == Some(&'\r') {
                    chars.next();
                }
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
        // A pty doubles the CR before each LF; nothing is overwritten.
        assert_eq!(
            strip_escapes("Build Succeeded\r\r\nUploading\r\r\n"),
            "Build Succeeded\nUploading\n"
        );
        assert_eq!(strip_escapes("10%\r\r20%\r\n"), "20%\n");
    }

    #[test]
    fn a_masked_tail_finds_a_value_across_the_window_edge_and_versions_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.vt");
        // A file past the tail window, with the secret straddling the
        // window's first byte: the line holding it starts the file,
        // and the window starts four bytes into the value.
        let secret = "tok-5e3cr3t";
        let tail = usize::try_from(TAIL_BYTES).unwrap();
        let mut text = format!("key={secret}\n");
        let head = text.len();
        text.push_str(&"b\n".repeat((tail + 8 - head) / 2));
        assert_eq!(text.len() - tail, 8, "the window starts inside the value");
        std::fs::write(&p, &text).unwrap();
        let mask = |s: &str| s.replace(secret, "<TOKEN>");
        let (_, out) = tail_masked(&p, usize::MAX, &mask, 0).unwrap();
        assert!(
            out.starts_with("5e3cr3t\n"),
            "no overlap: a fragment leaks: {out:.20}"
        );
        let (v1, out) = tail_masked(&p, usize::MAX, &mask, secret.len()).unwrap();
        assert!(out.starts_with("key=<TOKEN>\nb"), "{out:.20}");
        let (v2, _) = tail_masked(&p, 1, &mask, 0).unwrap();
        assert_eq!(v1, v2, "a read does not move the version");
        std::fs::write(&p, "other\n").unwrap();
        let (v3, _) = tail_masked(&p, 1, &mask, 0).unwrap();
        assert_ne!(v1, v3, "a rewrite moves it");
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
