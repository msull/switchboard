//! The command line's side: pair with a host the owner opened a
//! pairing window on, list the paired hosts, forget one.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::config::{self, Host};
use crate::http;

/// What `pair` needs: the name to keep the host under, where it
/// answers, the key the dialog shows, the code, and what this device
/// calls itself there.
#[derive(Debug, Clone)]
pub struct Pairing<'a> {
    pub name: &'a str,
    pub address: &'a str,
    pub spki_sha256: &'a str,
    pub code: &'a str,
    pub device: &'a str,
}

/// Pair, keep the host, and say how it went in lines for a person;
/// the token is never in them.
pub fn pair(dir: &Path, p: &Pairing) -> Result<Vec<String>, String> {
    let addresses = vec![p.address.to_owned()];
    let spki = p.spki_sha256.trim().to_ascii_lowercase();
    let body = json!({
        "code": p.code.trim(),
        "device_name": p.device,
        "spki_sha256": spki,
    });
    let reply = http::request(&addresses, &spki, "POST", "/api/v1/pair", None, Some(&body))
        .map_err(|e| format!("{}: {e}", p.address))?;
    if !reply.ok() {
        let (code, message) = reply.error();
        return Err(format!("pairing refused: {code}: {message}"));
    }
    let b = reply.body();
    let token = b["token"]
        .as_str()
        .ok_or("the host sent no token")?
        .to_owned();
    let scope = b["scope"].as_str().unwrap_or("read").to_owned();
    let paired_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    config::upsert(
        dir,
        Host {
            name: p.name.to_owned(),
            addresses: addresses.clone(),
            server_id: b["server_id"].as_str().unwrap_or_default().to_owned(),
            spki_sha256: spki.clone(),
            token: token.clone(),
            scope: scope.clone(),
            paired_ms,
            needs_pairing: false,
        },
    )?;
    let mut lines = vec![format!(
        "paired with {} as {:?}, {scope} scope",
        p.name, p.device
    )];
    let me = http::request(&addresses, &spki, "GET", "/api/v1/me", Some(&token), None)
        .ok()
        .filter(http::Reply::ok);
    let agent = me.as_ref().and_then(|m| m.body()["agent"].as_bool());
    match agent {
        Some(true) => {}
        Some(false) => lines.push(
            "the window was not opened with Agent ticked: the host refuses ask_supervisor, and \
             decisions are the owner's; open a window with Agent ticked and pair again"
                .into(),
        ),
        None => lines.push(
            "the host does not say whether this device is an agent: it may be too old for the \
             owner channel"
                .into(),
        ),
    }
    if scope != "control" {
        lines.push("a read scope only reads; the host's dialog can raise it".into());
    }
    Ok(lines)
}

/// The paired hosts, one line each, never a token.
pub fn hosts(dir: &Path) -> Result<Vec<String>, String> {
    let hosts = config::load(dir)?;
    if hosts.hosts.is_empty() {
        return Ok(vec!["no host is paired".into()]);
    }
    Ok(hosts
        .hosts
        .iter()
        .map(|h| {
            format!(
                "{} {} {} server {}{}",
                h.name,
                h.addresses.join(","),
                h.scope,
                h.server_id,
                if h.needs_pairing {
                    " (needs pairing again)"
                } else {
                    ""
                }
            )
        })
        .collect())
}
