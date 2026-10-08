//! Marks the session this runs in as asking the owner something: its
//! card, the supervisor chip and the Dock badge show the message until
//! the owner's next prompt, a dismiss, or `--clear`.
//!
//! A Claude Code session may ask for an answer of a shape: `--confirm`
//! (Yes or No), `--choice <option>` once per option, or `--text` (a line
//! of the owner's own). The owner answers on the card, and the answer
//! arrives as the session's next prompt once its turn has ended:
//! `Owner answered "<question>": <answer>`.
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
  switchboard-ask \"<question>\" --confirm
  switchboard-ask \"<question>\" --choice \"<option>\" --choice \"<option>\" ...
  switchboard-ask \"<question>\" --text
  switchboard-ask --clear";

/// What the arguments ask for.
#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Clear,
    Ask {
        message: String,
        kind: Option<String>,
        choices: Vec<String>,
    },
}

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
    let parsed = parse(args).map_err(Stop::Usage)?;
    let (session, token) = credentials().map_err(Stop::Usage)?;
    let body = match parsed {
        Parsed::Clear => Body::SessionAsk {
            session,
            token,
            message: None,
            ask_kind: None,
            choices: Vec::new(),
        },
        Parsed::Ask {
            message,
            kind,
            choices,
        } => Body::SessionAsk {
            session,
            token,
            message: Some(message),
            ask_kind: kind,
            choices,
        },
    };
    match call(body)? {
        Reply::Failed { reason } => Err(Stop::Failed(reason)),
        _ => Ok(()),
    }
}

/// The ask, or `Clear`. The kind flags may stand anywhere; the other
/// words are joined with spaces, so an unquoted question still reads as
/// one.
fn parse(args: &[String]) -> Result<Parsed, String> {
    match args {
        [flag] if flag == "--clear" => return Ok(Parsed::Clear),
        [first, ..] if first == "--clear" => {
            return Err("--clear takes nothing after it".into());
        }
        _ => {}
    }
    let mut kind: Option<&str> = None;
    let mut choices = Vec::new();
    let mut words = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let flag = match arg.as_str() {
            "--clear" => return Err("--clear takes nothing else".into()),
            "--confirm" => "confirm",
            "--text" => "text",
            "--choice" => {
                let Some(option) = rest.next() else {
                    return Err("--choice needs an option after it".into());
                };
                choices.push(option.clone());
                "choice"
            }
            _ => {
                words.push(arg.as_str());
                continue;
            }
        };
        if kind.is_some_and(|k| k != flag) {
            return Err("use one of --confirm, --choice and --text".into());
        }
        if kind == Some(flag) && flag != "choice" {
            return Err(format!("--{flag} given twice"));
        }
        kind = Some(flag);
    }
    let message = words.join(" ");
    if message.trim().is_empty() {
        return Err("no message".into());
    }
    Ok(Parsed::Ask {
        message,
        kind: kind.map(str::to_owned),
        choices,
    })
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
    use super::{Parsed, parse};

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn plain(message: &str) -> Parsed {
        Parsed::Ask {
            message: message.to_owned(),
            kind: None,
            choices: vec![],
        }
    }

    fn shaped(message: &str, kind: &str, choices: &[&str]) -> Parsed {
        Parsed::Ask {
            message: message.to_owned(),
            kind: Some(kind.to_owned()),
            choices: args(choices),
        }
    }

    #[test]
    fn clear_takes_no_message() {
        assert_eq!(parse(&args(&["--clear"])), Ok(Parsed::Clear));
        assert!(parse(&args(&["--clear", "now"])).is_err());
    }

    #[test]
    fn a_message_is_one_argument_or_several_joined() {
        assert_eq!(parse(&args(&["merge now?"])), Ok(plain("merge now?")));
        assert_eq!(parse(&args(&["merge", "now?"])), Ok(plain("merge now?")));
    }

    #[test]
    fn each_kind_flag_shapes_the_ask() {
        assert_eq!(
            parse(&args(&["merge?", "--confirm"])),
            Ok(shaped("merge?", "confirm", &[]))
        );
        assert_eq!(
            parse(&args(&["--text", "which branch?"])),
            Ok(shaped("which branch?", "text", &[]))
        );
        assert_eq!(
            parse(&args(&["pick", "--choice", "a", "--choice", "b c"])),
            Ok(shaped("pick", "choice", &["a", "b c"]))
        );
    }

    #[test]
    fn flags_may_stand_among_the_words() {
        assert_eq!(
            parse(&args(&["pick", "--choice", "a", "one", "--choice", "b"])),
            Ok(shaped("pick one", "choice", &["a", "b"]))
        );
    }

    #[test]
    fn mixed_or_incomplete_flags_are_usage_errors() {
        assert!(parse(&args(&["q", "--confirm", "--text"])).is_err());
        assert!(parse(&args(&["q", "--choice", "a", "--confirm"])).is_err());
        assert!(parse(&args(&["q", "--confirm", "--confirm"])).is_err());
        assert!(parse(&args(&["q", "--choice"])).is_err());
        assert!(parse(&args(&["--clear", "--confirm"])).is_err());
        assert!(parse(&args(&["q", "--confirm", "--clear"])).is_err());
        assert!(parse(&args(&["--choice", "a", "--choice", "b"])).is_err());
    }

    #[test]
    fn nothing_to_ask_is_a_usage_error() {
        assert!(parse(&[]).is_err());
        assert!(parse(&args(&["  "])).is_err());
    }
}
