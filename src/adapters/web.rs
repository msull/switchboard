//! The web page the remote port serves before pairing: three files
//! built into the binary, with no build step and nothing fetched from
//! elsewhere. The page speaks the same `/api/v1/` a native client
//! does; this file only says which paths are the page and what they
//! hold.

/// One of the page's files.
pub struct Page {
    /// The exact request path that answers with this file.
    pub path: &'static str,
    /// The `Content-Type` it is served with.
    pub content_type: &'static str,
    /// The file, built into the binary.
    pub body: &'static str,
}

/// Every file the page is made of, by exact path.
pub const PAGES: &[Page] = &[
    Page {
        path: "/",
        content_type: "text/html; charset=utf-8",
        body: include_str!("../../web/index.html"),
    },
    Page {
        path: "/app.js",
        content_type: "text/javascript; charset=utf-8",
        body: include_str!("../../web/app.js"),
    },
    Page {
        path: "/app.css",
        content_type: "text/css; charset=utf-8",
        body: include_str!("../../web/app.css"),
    },
];

/// What the browser may do with the page: run only its own script and
/// style, talk only to this port, and be framed by nothing. No inline
/// script or style exists to allow, so agent output that reached the
/// DOM as markup still could not run.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The headers on every page file beside `Content-Type`,
/// `Content-Length` and `Connection`.
pub const PAGE_HEADERS: &[(&str, &str)] = &[
    ("Content-Security-Policy", CONTENT_SECURITY_POLICY),
    ("Cache-Control", "no-store"),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
];

/// The page file a `GET` for `path` asks for, matched exactly.
#[must_use]
pub fn page(method: &str, path: &str) -> Option<&'static Page> {
    if method != "GET" {
        return None;
    }
    PAGES.iter().find(|p| p.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> &'static str {
        page("GET", "/").unwrap().body
    }

    #[test]
    fn only_a_get_for_an_exact_path_is_a_page() {
        assert_eq!(page("GET", "/").map(|p| p.path), Some("/"));
        assert_eq!(
            page("GET", "/app.js").map(|p| p.content_type),
            Some("text/javascript; charset=utf-8")
        );
        assert!(page("POST", "/").is_none());
        assert!(page("GET", "/favicon.ico").is_none());
        assert!(page("GET", "/app.js/").is_none());
        assert!(page("GET", "/api/v1/info").is_none());
    }

    /// Every `src` and `href` in the page is one of its own files or a
    /// `data:` URL, so the policy's `'self'` is all it needs.
    #[test]
    fn the_page_references_only_its_own_files() {
        let html = index();
        let mut seen = 0;
        for attribute in ["src=\"", "href=\""] {
            for (at, _) in html.match_indices(attribute) {
                let value = &html[at + attribute.len()..];
                let value = &value[..value.find('"').unwrap()];
                assert!(
                    value.starts_with("data:") || PAGES.iter().any(|p| p.path == value),
                    "{value}"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, 3, "the icon, the style and the script");
    }

    /// Nothing the policy would have to allow: no inline script, no
    /// handler attribute, no inline style, and a script that never
    /// turns text into markup or code.
    #[test]
    fn the_page_has_no_inline_code_and_the_script_writes_no_markup() {
        let html = index();
        for (at, _) in html.match_indices("<script") {
            let tag = &html[at..at + html[at..].find('>').unwrap()];
            assert!(tag.contains(" src=\""), "inline script: {tag}");
        }
        assert!(!html.contains("style="), "inline style");
        assert!(!html.contains("<style"), "inline style element");
        for (at, _) in html.match_indices(" on") {
            let word: String = html[at + 1..]
                .chars()
                .take_while(char::is_ascii_alphabetic)
                .collect();
            assert!(
                !html[at + 1 + word.len()..].starts_with('='),
                "handler attribute {word}"
            );
        }
        let script = page("GET", "/app.js").unwrap().body;
        for banned in [
            "innerHTML",
            "outerHTML",
            "eval(",
            "document.write",
            "insertAdjacentHTML",
        ] {
            assert!(!script.contains(banned), "{banned}");
        }
    }

    /// The page's pure parts under `node --test`. Without node the test
    /// fails on CI, which installs it, and elsewhere says it skipped.
    #[test]
    fn the_pages_logic_passes_its_node_tests() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
        let output = match std::process::Command::new("node")
            .arg("--test")
            .arg("app.test.js")
            .current_dir(&dir)
            .output()
        {
            Ok(output) => output,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let on_ci = std::env::var("CI").is_ok_and(|v| !v.is_empty());
                assert!(!on_ci, "node is required on CI to run `web/app.test.js`");
                // libtest captures the print macros of a passing test,
                // not a direct write, so the skip reaches the terminal.
                let _ = std::io::Write::write_all(
                    &mut std::io::stderr(),
                    b"SKIPPED: the page's node tests did not run: node is not installed\n",
                );
                return;
            }
            Err(e) => panic!("node: {e}"),
        };
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
