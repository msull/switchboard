//! The content-version signals of docs/remote-client.md §4.4 against the
//! cases that defeat weaker ones. The design versions a pinned file and
//! a run log by inode, ctime and size; this shows why mtime and size,
//! or length alone, are not enough.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Weak {
    mtime: (i64, i64),
    size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Strong {
    ino: u64,
    ctime: (i64, i64),
    size: u64,
}

fn weak(p: &Path) -> Weak {
    let m = fs::metadata(p).unwrap();
    Weak {
        mtime: (m.mtime(), m.mtime_nsec()),
        size: m.len(),
    }
}

fn strong(p: &Path) -> Strong {
    let m = fs::metadata(p).unwrap();
    Strong {
        ino: m.ino(),
        ctime: (m.ctime(), m.ctime_nsec()),
        size: m.len(),
    }
}

fn tempdir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("spike22-{}-{}", std::process::id(), rand()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn rand() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    u64::from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos(),
    )
}

/// A same-size rewrite with the old mtime put back (`touch -r`): the
/// weak signal cannot see it, the strong one can, because ctime moves on
/// every inode change and no user tool sets it.
#[test]
fn a_same_size_rewrite_with_mtime_restored_moves_ctime_only() {
    let d = tempdir();
    let f = d.join("pinned.md");
    let r = d.join("reference");
    fs::write(&f, "hello world").unwrap();
    fs::write(&r, "").unwrap();
    // `touch -r` copies the reference's times, which are a tick older.
    sleep(Duration::from_millis(20));
    let status = Command::new("touch")
        .arg("-r")
        .arg(&f)
        .arg(&r)
        .status()
        .unwrap();
    assert!(status.success());
    let w0 = weak(&f);
    let s0 = strong(&f);
    sleep(Duration::from_millis(20));
    fs::write(&f, "HELLO WORLD").unwrap();
    let status = Command::new("touch")
        .arg("-r")
        .arg(&r)
        .arg(&f)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(weak(&f), w0, "mtime and size read as unchanged");
    assert_ne!(strong(&f), s0, "ctime moved");
    fs::remove_dir_all(&d).unwrap();
}

/// Replace by rename (what editors and `git checkout` do): a new inode.
#[test]
fn a_replace_by_rename_moves_the_inode() {
    let d = tempdir();
    let f = d.join("pinned.md");
    fs::write(&f, "hello world").unwrap();
    let s0 = strong(&f);
    let t = d.join("pinned.md.tmp");
    fs::write(&t, "HELLO WORLD").unwrap();
    fs::rename(&t, &f).unwrap();
    let s1 = strong(&f);
    assert_ne!(s1.ino, s0.ino);
    fs::remove_dir_all(&d).unwrap();
}

/// A run log truncated and regrown to its old length: length alone is
/// blind, ctime is not.
#[test]
fn a_log_truncated_and_regrown_to_the_same_length_moves_ctime() {
    let d = tempdir();
    let f = d.join("run.log");
    fs::write(&f, "line one\nline two\n").unwrap();
    let s0 = strong(&f);
    sleep(Duration::from_millis(20));
    fs::write(&f, "").unwrap();
    fs::write(&f, "LINE ONE\nLINE TWO\n").unwrap();
    let s1 = strong(&f);
    assert_eq!(s1.size, s0.size, "same length");
    assert_ne!(s1.ctime, s0.ctime);
    fs::remove_dir_all(&d).unwrap();
}

/// A transcript only grows: size is a version on its own, and two
/// appends inside one poll interval are two sizes.
#[test]
fn an_appended_transcript_is_versioned_by_size() {
    let dir = tempdir();
    let path = dir.join("t.jsonl");
    fs::write(&path, "{}\n").unwrap();
    let first = weak(&path);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"{\"a\":1}\n").unwrap();
    let second = weak(&path);
    file.write_all(b"{\"b\":2}\n").unwrap();
    let third = weak(&path);
    assert!(first.size < second.size && second.size < third.size);
    fs::remove_dir_all(&dir).unwrap();
}
