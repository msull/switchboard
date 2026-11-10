//! Bonjour advertisement of the remote port. On macOS the record is
//! registered through mDNSResponder (`DNSServiceRegister`, see `dnssd`),
//! so the system's own responder answers for it, hands it to a Sleep
//! Proxy, and drops it when the app quits or is killed. Elsewhere the
//! adapter is a stub that advertises nothing.

#[cfg(target_os = "macos")]
use std::sync::mpsc::RecvTimeoutError;

use crate::ports::remote::{Advertisement, Advertiser};

/// The host's one Bonjour record.
#[derive(Default)]
pub struct Bonjour {
    /// The worker that owns the registration, started on the first
    /// `advertise`.
    #[cfg(target_os = "macos")]
    worker: Option<Worker>,
}

/// How long the frame waits for the worker's answer. A healthy worker
/// answers within one poll interval; a wedged mDNSResponder must not hold
/// the frame forever.
#[cfg(target_os = "macos")]
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(target_os = "macos")]
struct Worker {
    tx: std::sync::mpsc::Sender<super::dnssd::Cmd>,
    join: std::thread::JoinHandle<()>,
}

#[cfg(target_os = "macos")]
impl Advertiser for Bonjour {
    fn advertise(&mut self, ad: &Advertisement) -> Result<(), String> {
        use super::dnssd::{Cmd, spawn};
        let worker = if let Some(worker) = &mut self.worker {
            worker
        } else {
            let (tx, join) = spawn()?;
            self.worker.insert(Worker { tx, join })
        };
        let (reply, answer) = std::sync::mpsc::channel();
        if worker.tx.send(Cmd::Register(ad.clone(), reply)).is_err() {
            // The worker is gone; the next `advertise` starts another.
            self.worker = None;
            return Err("bonjour: the worker stopped".to_owned());
        }
        match answer.recv_timeout(REPLY_TIMEOUT) {
            Ok(result) => {
                // The worker skips a registration that is already in
                // force, so only a fresh one is worth a log line.
                if result? {
                    log::info!("bonjour: advertising {:?} on port {}", ad.instance, ad.port);
                }
                Ok(())
            }
            Err(RecvTimeoutError::Timeout) => {
                Err("bonjour: the worker did not answer in time".to_owned())
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.worker = None;
                Err("bonjour: the worker stopped".to_owned())
            }
        }
    }

    fn withdraw(&mut self) {
        if let Some(worker) = &self.worker {
            let (reply, answer) = std::sync::mpsc::channel();
            if worker.tx.send(super::dnssd::Cmd::Withdraw(reply)).is_ok() {
                let _ = answer.recv_timeout(REPLY_TIMEOUT);
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for Bonjour {
    fn drop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let _ = worker.tx.send(super::dnssd::Cmd::Stop);
        // Joining makes sure the record is withdrawn before the process
        // goes on, but a wedged worker must not hang the exit.
        let deadline = std::time::Instant::now() + REPLY_TIMEOUT;
        while !worker.join.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if worker.join.is_finished() {
            let _ = worker.join.join();
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl Advertiser for Bonjour {
    fn advertise(&mut self, ad: &Advertisement) -> Result<(), String> {
        log::info!(
            "bonjour: not available here; {:?} is not advertised",
            ad.instance
        );
        Ok(())
    }

    fn withdraw(&mut self) {}
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// Asks the worker directly, as `advertise` does, and says whether it
    /// made a new registration.
    fn registered(bonjour: &Bonjour, ad: &Advertisement) -> bool {
        let (reply, answer) = std::sync::mpsc::channel();
        let worker = bonjour.worker.as_ref().unwrap();
        worker
            .tx
            .send(super::super::dnssd::Cmd::Register(ad.clone(), reply))
            .unwrap();
        answer.recv().unwrap().unwrap()
    }

    #[test]
    fn an_equal_advertisement_is_not_registered_again() {
        let ad = Advertisement::new(uuid::Uuid::new_v4(), "Lifecycle test", 43998);
        let mut bonjour = Bonjour::default();
        if let Err(e) = bonjour.advertise(&ad) {
            eprintln!("skipping: mDNSResponder unusable here: {e}");
            return;
        }
        assert!(!registered(&bonjour, &ad));
        bonjour.withdraw();
        assert!(registered(&bonjour, &ad));
        let started = Instant::now();
        drop(bonjour);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// A `dns-sd` child, killed when the test ends however it ends.
    struct DnsSd(Child);

    impl DnsSd {
        fn start(args: &[&str]) -> Self {
            Self(
                Command::new("dns-sd")
                    .args(args)
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap(),
            )
        }

        /// Stops the child and returns what it printed.
        fn output(mut self) -> String {
            let _ = self.0.kill();
            let mut out = String::new();
            if let Some(mut stdout) = self.0.stdout.take() {
                let _ = stdout.read_to_string(&mut out);
            }
            out
        }
    }

    impl Drop for DnsSd {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Registers a record and browses for it with `dns-sd`, a second
    /// client of the same mDNSResponder. Needs a network interface with
    /// multicast, so it is run by hand:
    /// `cargo test --locked -p switchboard --lib bonjour -- --ignored`
    #[test]
    #[ignore = "needs multicast on an interface; run by hand"]
    fn the_record_is_browsable_until_withdrawn() {
        let id = uuid::Uuid::new_v4();
        let ad = Advertisement::new(id, "Test host", 43999);
        let mut bonjour = Bonjour::default();
        let browse = DnsSd::start(&["-B", Advertisement::SERVICE, "local."]);
        bonjour.advertise(&ad).unwrap();
        std::thread::sleep(Duration::from_secs(3));
        let lookup = DnsSd::start(&["-L", &ad.instance, Advertisement::SERVICE, "local"]);
        std::thread::sleep(Duration::from_secs(3));
        let lookup = lookup.output();
        assert!(lookup.contains(":43999"), "{lookup}");
        assert!(lookup.contains(&format!("id={id}")), "{lookup}");
        assert!(lookup.contains("v=1"), "{lookup}");
        bonjour.withdraw();
        std::thread::sleep(Duration::from_secs(3));
        let browse = browse.output();
        let lines: Vec<&str> = browse
            .lines()
            .filter(|l| l.contains(&ad.instance))
            .collect();
        assert!(lines.iter().any(|l| l.contains(" Add ")), "{browse}");
        assert!(lines.iter().any(|l| l.contains(" Rmv ")), "{browse}");
    }
}
