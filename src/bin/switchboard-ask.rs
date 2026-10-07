//! Marks the session this runs in as asking the owner something: its
//! card, the supervisor chip and the Dock badge show the message until
//! the owner's next prompt, a dismiss, or `--clear`.
//!
//! Usage is `USAGE`. Silent on success, since what it prints reaches the
//! agent's tool result; on a refusal it prints the app's reason.
//!
//! Environment: `SWITCHBOARD_RECORD_ID` and `SWITCHBOARD_RECORD_TOKEN`
//! (put in every pane by Switchboard) and `SWITCHBOARD_DATA_DIR`
//! (defaults to the app's Application Support dir). The token is what
//! limits a session to marking its own record.
//!
//! std and `switchboard_control` only, like `switchboard-env`.

use std::process::ExitCode;

use switchboard_control::{
    Body, Client, Reply, Request, SOCKET_FILE, credentials, data_dir, op_id,
};

/// `sysexits.h`'s `EX_USAGE`.
const USAGE_EXIT: u8 = 64;

const USAGE: &str = "usage:
  switchboard-ask \"<one line for the owner>\"
  switchboard-ask --clear";

/// Why a run stopped: bad usage, or anything else, with the line to print.
enum Stop {
    Usage(String),
    Failed(String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Stop::Usage(why)) => {
            eprintln!("switchboard-ask: {why}\n{USAGE}");
            ExitCode::from(USAGE_EXIT)
        }
        Err(Stop::Failed(why)) => {
            eprintln!("switchboard-ask: {why}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), Stop> {
    let message = parse(args).map_err(Stop::Usage)?;
    let (session, token) = credentials().map_err(Stop::Usage)?;
    match call(Body::SessionAsk {
        session,
        token,
        message,
    })? {
        Reply::Failed { reason } => Err(Stop::Failed(reason)),
        _ => Ok(()),
    }
}

/// The message to ask, or `None` for `--clear`. Several words are
/// joined with spaces, so an unquoted question still reads as one.
fn parse(args: &[String]) -> Result<Option<String>, String> {
    match args {
        [] => Err("no message".into()),
        [flag] if flag == "--clear" => Ok(None),
        [first, ..] if first == "--clear" => Err("--clear takes nothing after it".into()),
        words => {
            let message = words.join(" ");
            if message.trim().is_empty() {
                Err("no message".into())
            } else {
                Ok(Some(message))
            }
        }
    }
}

/// One request, one reply. A missing or refusing socket means the app is
/// not running, which is the one thing worth saying about it.
fn call(body: Body) -> Result<Reply, Stop> {
    let path = data_dir().join(SOCKET_FILE);
    let mut client =
        Client::connect(&path).map_err(|_| Stop::Failed("Switchboard is not running".into()))?;
    client
        .call(&Request::new(op_id("switchboard-ask"), body))
        .map_err(|e| Stop::Failed(format!("control port: {e}")))
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn clear_takes_no_message() {
        assert_eq!(parse(&args(&["--clear"])), Ok(None));
        assert!(parse(&args(&["--clear", "now"])).is_err());
    }

    #[test]
    fn a_message_is_one_argument_or_several_joined() {
        assert_eq!(
            parse(&args(&["merge now?"])),
            Ok(Some("merge now?".to_owned()))
        );
        assert_eq!(
            parse(&args(&["merge", "now?"])),
            Ok(Some("merge now?".to_owned()))
        );
    }

    #[test]
    fn nothing_to_ask_is_a_usage_error() {
        assert!(parse(&[]).is_err());
        assert!(parse(&args(&["  "])).is_err());
    }
}
