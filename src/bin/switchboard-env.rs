//! Runs one child with the credentials of a session's environment sets.
//! The values are resolved by the running app over its control port,
//! answered only to the holder of the pane's launch token, and put only
//! in the child's environment: never on a command line, in a file, or in
//! anything this prints.
//!
//! Usage is `USAGE`, printed on any bad command line.
//!
//! Environment: `SWITCHBOARD_RECORD_ID` and `SWITCHBOARD_RECORD_TOKEN`
//! (put in every pane by Switchboard; `exec` and `check` need both) and
//! `SWITCHBOARD_DATA_DIR` (defaults to the app's Application Support
//! dir). The setup commands are accepted only while the owner has
//! unlocked environment setup in the app.
//!
//! std and `switchboard_control` only: it never links the app, and takes
//! the names it shares with it from the wire crate.

use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};

use switchboard_control::{
    AwsMethod, Body, Client, EnvVarView, RECORD_TOKEN_ENV, Reply, Request, SOCKET_FILE,
    credentials, data_dir, op_id,
};

/// What a child must not inherit when a set gives AWS credentials: the
/// CLI and SDKs prefer ambient keys over `AWS_PROFILE`, so any of these
/// left in place could run the child against another account.
const AWS_AMBIENT: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_SECURITY_TOKEN",
    "AWS_CREDENTIAL_EXPIRATION",
    "AWS_PROFILE",
    "AWS_DEFAULT_PROFILE",
    "AWS_ROLE_ARN",
    "AWS_ROLE_SESSION_NAME",
    "AWS_WEB_IDENTITY_TOKEN_FILE",
    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
    "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_CONTAINER_AUTHORIZATION_TOKEN",
];

/// The variables a `static` set must supply itself.
const AWS_STATIC_KEYS: [&str; 2] = ["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"];

/// `sysexits.h`'s `EX_USAGE`.
const USAGE_EXIT: u8 = 64;

const USAGE: &str = "usage:
  switchboard-env exec -- <command> [args...]
  switchboard-env check
  switchboard-env sets
  switchboard-env set create <set>
  switchboard-env set var <set> NAME=VALUE
  switchboard-env set secret <set> NAME        (value from the terminal or stdin)
  switchboard-env set aws <set> vault <profile> | sso <profile> | static
  switchboard-env grant  (--project <id> | --session <id> | --runner) <set>
  switchboard-env revoke (--project <id> | --session <id> | --runner) <set>";

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
            eprintln!("switchboard-env: {why}\n{USAGE}");
            ExitCode::from(USAGE_EXIT)
        }
        Err(Stop::Failed(why)) => {
            eprintln!("switchboard-env: {why}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), Stop> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["exec", "--", command @ ..] if !command.is_empty() => exec(command),
        ["exec", ..] => Err(Stop::Usage("exec needs -- and a command".into())),
        ["check"] => check(),
        ["sets"] => sets(),
        ["set", "create", set] => setup(Body::EnvSetUpsert {
            name: (*set).to_owned(),
            vars: Vec::new(),
            aws: None,
        }),
        ["set", "var", set, pair] => {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| Stop::Usage("set var wants NAME=VALUE".into()))?;
            setup(Body::EnvSetUpsert {
                name: (*set).to_owned(),
                vars: vec![EnvVarView {
                    name: name.to_owned(),
                    value: value.to_owned(),
                    secret: false,
                }],
                aws: None,
            })
        }
        ["set", "secret", set, name] => {
            let value = read_secret(name)?;
            setup(Body::EnvSecretStore {
                set: (*set).to_owned(),
                name: (*name).to_owned(),
                value,
            })
        }
        ["set", "aws", set, method @ ..] => {
            let aws = match method {
                ["vault", profile] => AwsMethod::Vault {
                    profile: (*profile).to_owned(),
                },
                ["sso", profile] => AwsMethod::Sso {
                    profile: (*profile).to_owned(),
                },
                ["static"] => AwsMethod::Static,
                _ => {
                    return Err(Stop::Usage(
                        "set aws wants vault <profile>, sso <profile> or static".into(),
                    ));
                }
            };
            setup(Body::EnvSetUpsert {
                name: (*set).to_owned(),
                vars: Vec::new(),
                aws: Some(aws),
            })
        }
        [verb @ ("grant" | "revoke"), target @ .., set] => {
            let (project, session, runner) = match target {
                ["--project", id] => (Some((*id).to_owned()), None, false),
                ["--session", id] => (None, Some((*id).to_owned()), false),
                ["--runner"] => (None, None, true),
                _ => {
                    return Err(Stop::Usage(format!(
                        "{verb} wants one of --project <id>, --session <id> or --runner"
                    )));
                }
            };
            setup(Body::EnvGrant {
                project,
                session,
                runner,
                set: (*set).to_owned(),
                remove: *verb == "revoke",
            })
        }
        [] => Err(Stop::Usage("no command".into())),
        [other, ..] => Err(Stop::Usage(format!("unknown command {other}"))),
    }
}

/// One request, one reply. A missing or refusing socket means the app is
/// not running, which is the one thing worth saying about it.
fn call(body: Body) -> Result<Reply, Stop> {
    let path = data_dir().join(SOCKET_FILE);
    let mut client =
        Client::connect(&path).map_err(|_| Stop::Failed("Switchboard is not running".into()))?;
    client
        .call(&Request::new(op_id("switchboard-env"), body))
        .map_err(|e| Stop::Failed(format!("control port: {e}")))
}

/// The fields of an `env` reply.
struct Resolved {
    pairs: Vec<(String, String)>,
    aws: Option<AwsMethod>,
    missing: Vec<String>,
}

/// What the session's sets resolve to now, or the reason they do not.
fn resolve() -> Result<Resolved, Stop> {
    let (session, token) = credentials().map_err(Stop::Usage)?;
    match call(Body::EnvResolve { session, token })? {
        Reply::Env {
            pairs,
            aws,
            missing,
        } => Ok(Resolved {
            pairs,
            aws,
            missing,
        }),
        Reply::Failed { reason } => Err(Stop::Failed(reason)),
        _ => Err(Stop::Failed("unexpected reply to env.resolve".into())),
    }
}

/// A command with the resolved pairs and none of what it must not
/// inherit: the token (a child is not the session), a stale `AWS_VAULT`,
/// which makes aws-vault refuse to nest, and, when the sets give an AWS
/// method (`aws`), every ambient AWS credential and profile.
fn command(program: &str, args: &[&str], pairs: &[(String, String)], aws: bool) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env_remove(RECORD_TOKEN_ENV)
        .env_remove("AWS_VAULT");
    if aws {
        for name in AWS_AMBIENT {
            cmd.env_remove(name);
        }
    }
    cmd.envs(pairs.iter().map(|(k, v)| (k, v)));
    cmd
}

/// Whether the SSO session behind `profile` still answers.
fn sso_valid(profile: &str, pairs: &[(String, String)]) -> bool {
    command(
        "aws",
        &["sts", "get-caller-identity", "--profile", profile],
        pairs,
        true,
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|s| s.success())
}

fn sso_login_line(profile: &str) -> String {
    format!(
        "the AWS SSO session for {profile} has expired or is missing: run aws sso login --profile {profile}"
    )
}

/// The keys a `static` set lacks, which the child would otherwise not
/// have at all: ambient ones are removed.
fn static_keys_absent(pairs: &[(String, String)]) -> Vec<&'static str> {
    AWS_STATIC_KEYS
        .into_iter()
        .filter(|key| !pairs.iter().any(|(k, _)| k == key))
        .collect()
}

fn exec(argv: &[&str]) -> Result<(), Stop> {
    let Resolved {
        mut pairs,
        aws,
        missing,
    } = resolve()?;
    if !missing.is_empty() {
        return Err(Stop::Failed(format!(
            "secrets with no stored value: {}",
            missing.join(", ")
        )));
    }
    let mut cmd = match &aws {
        Some(AwsMethod::Vault { profile }) => {
            let mut wrapped = vec!["exec", profile.as_str(), "--"];
            wrapped.extend_from_slice(argv);
            command("aws-vault", &wrapped, &pairs, true)
        }
        Some(AwsMethod::Sso { profile }) => {
            if !sso_valid(profile, &pairs) {
                return Err(Stop::Failed(sso_login_line(profile)));
            }
            pairs.push(("AWS_PROFILE".into(), profile.clone()));
            command(argv[0], &argv[1..], &pairs, true)
        }
        Some(AwsMethod::Static) => {
            let absent = static_keys_absent(&pairs);
            if !absent.is_empty() {
                return Err(Stop::Failed(format!(
                    "static AWS credentials need {}",
                    absent.join(" and ")
                )));
            }
            command(argv[0], &argv[1..], &pairs, true)
        }
        None => command(argv[0], &argv[1..], &pairs, false),
    };
    // `exec` returns only when it could not replace this process.
    let e = cmd.exec();
    let program = cmd.get_program().to_string_lossy().into_owned();
    Err(Stop::Failed(format!("cannot run {program}: {e}")))
}

fn check() -> Result<(), Stop> {
    let Resolved {
        pairs,
        aws,
        missing,
    } = resolve()?;
    let names: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    println!(
        "variables: {}",
        if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        }
    );
    let mut ok = missing.is_empty();
    if !missing.is_empty() {
        println!("missing secrets: {}", missing.join(", "));
    }
    match &aws {
        None => println!("aws: none"),
        Some(AwsMethod::Static) => {
            let absent = static_keys_absent(&pairs);
            if absent.is_empty() {
                println!("aws: static keys");
            } else {
                println!("aws: static keys: missing {}", absent.join(", "));
            }
            ok &= absent.is_empty();
        }
        Some(AwsMethod::Sso { profile }) => {
            let valid = sso_valid(profile, &pairs);
            println!(
                "aws: sso {profile}: {}",
                if valid { "valid" } else { "not logged in" }
            );
            if !valid {
                println!("{}", sso_login_line(profile));
            }
            ok &= valid;
        }
        Some(AwsMethod::Vault { profile }) => {
            let valid = command(
                "aws-vault",
                &["exec", profile, "--", "aws", "sts", "get-caller-identity"],
                &pairs,
                true,
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
            println!(
                "aws: vault {profile}: {}",
                if valid { "valid" } else { "failed" }
            );
            ok &= valid;
        }
    }
    if ok {
        Ok(())
    } else {
        Err(Stop::Failed("the environment is not ready".into()))
    }
}

fn sets() -> Result<(), Stop> {
    let sets = match call(Body::EnvSets)? {
        Reply::EnvSets { sets } => sets,
        Reply::Failed { reason } => return Err(Stop::Failed(reason)),
        _ => return Err(Stop::Failed("unexpected reply to env.sets".into())),
    };
    if sets.is_empty() {
        println!("no environment sets");
    }
    for set in sets {
        match &set.aws {
            Some(AwsMethod::Vault { profile }) => println!("{} (aws: vault {profile})", set.name),
            Some(AwsMethod::Sso { profile }) => println!("{} (aws: sso {profile})", set.name),
            Some(AwsMethod::Static) => println!("{} (aws: static)", set.name),
            None => println!("{}", set.name),
        }
        for var in set.vars {
            if var.secret {
                println!("  {} (secret)", var.name);
            } else {
                println!("  {}={}", var.name, var.value);
            }
        }
    }
    Ok(())
}

/// A setup command; the app refuses it unless the owner unlocked setup.
fn setup(body: Body) -> Result<(), Stop> {
    if let Reply::Failed { reason } = call(body)? {
        return Err(Stop::Failed(reason));
    }
    println!("ok");
    Ok(())
}

/// A secret's value, never from argv: typed at the terminal with echo
/// off, or one line from a pipe.
fn read_secret(name: &str) -> Result<String, Stop> {
    let stdin = std::io::stdin();
    let tty = stdin.is_terminal();
    let stty = |flag: &str| {
        let _ = Command::new("stty")
            .arg(flag)
            .stdin(Stdio::inherit())
            .status();
    };
    if tty {
        eprint!("value for {name}: ");
        let _ = std::io::stderr().flush();
        stty("-echo");
    }
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if tty {
        stty("echo");
        eprintln!();
    }
    read.map_err(|e| Stop::Failed(format!("cannot read the value: {e}")))?;
    let value = line.trim_end_matches(['\r', '\n']).to_owned();
    if value.is_empty() {
        return Err(Stop::Failed("no value given".into()));
    }
    Ok(value)
}
