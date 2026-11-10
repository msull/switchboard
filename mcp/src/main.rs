//! `switchboard-mcp`: with no argument, the MCP server on stdio;
//! `pair`, `hosts` and `forget` manage the paired hosts.

use std::io::{BufReader, Write as _};

use switchboard_mcp::{Server, config, pair};

const USAGE: &str = "usage:
  switchboard-mcp                     the MCP server on stdin and stdout
  switchboard-mcp pair <name> --address <host:port> --spki <key> --code <code> [--device <name>]
                                      pair with a host whose owner opened a control
                                      pairing window with Agent ticked; the key and
                                      the code are what the window shows
  switchboard-mcp hosts               the paired hosts
  switchboard-mcp forget <name>       drop a paired host
the config directory is $SWITCHBOARD_MCP_DIR, or ~/.config/switchboard-mcp";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Err(e) = command(&args) {
        eprintln!("switchboard-mcp: {e}");
        std::process::exit(1);
    }
}

fn command(args: &[&str]) -> Result<(), String> {
    let dir = config::default_dir()?;
    match args {
        [] => {
            let server = Server::new(dir, Box::new(std::io::stdout()));
            server.run(BufReader::new(std::io::stdin()));
            Ok(())
        }
        ["hosts"] => print(&pair::hosts(&dir)?),
        ["forget", name] => {
            if config::forget(&dir, name)? {
                print(&[format!("forgot {name}")])
            } else {
                Err(format!("no host {name} is paired"))
            }
        }
        ["pair", name, rest @ ..] => {
            let flag = |f: &str| {
                rest.iter()
                    .position(|a| *a == f)
                    .and_then(|i| rest.get(i + 1))
                    .copied()
            };
            let (Some(address), Some(spki), Some(code)) =
                (flag("--address"), flag("--spki"), flag("--code"))
            else {
                return Err(USAGE.into());
            };
            let lines = pair::pair(
                &dir,
                &pair::Pairing {
                    name,
                    address,
                    spki_sha256: spki,
                    code,
                    device: flag("--device").unwrap_or("switchboard-mcp"),
                },
            )?;
            print(&lines)
        }
        _ => Err(USAGE.into()),
    }
}

fn print(lines: &[String]) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    for line in lines {
        writeln!(out, "{line}").map_err(|e| e.to_string())?;
    }
    Ok(())
}
