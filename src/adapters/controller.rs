//! The nunchuk over USB serial: the Feather in `firmware/nunchuk` prints
//! one line per button change or stick flick. A thread owns the port,
//! reconnects when the cable is pulled, and hands events over a channel;
//! the app drains them on its poll. The waiting count goes back down the
//! same port for the LED strip.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use crate::ports::controller::{Controller, ControllerEvent};

const BAUD: u32 = 115_200;
/// How long a read waits before the thread checks for something to
/// send and whether the port is still there.
const READ_TIMEOUT: Duration = Duration::from_millis(200);
const RETRY: Duration = Duration::from_secs(1);
/// How often the device is pinged. It answers each ping, and treats a
/// host that has pinged as gone once the pings stop, so its display
/// can say whether Switchboard is still there.
const KEEPALIVE: Duration = Duration::from_secs(1);

pub struct SerialController {
    events: Receiver<ControllerEvent>,
    outbound: Sender<String>,
}

impl SerialController {
    /// Starts the port thread. `port` names the device
    /// (`SWITCHBOARD_CONTROLLER`), else the Feather's `usbmodem` port is
    /// used, so the thread also waits for one to be plugged in. `wake`
    /// runs on the thread after each event, for a repaint request.
    #[must_use]
    pub fn spawn(port: Option<String>, wake: impl Fn() + Send + 'static) -> Self {
        let (event_tx, events) = channel();
        let (outbound, outbound_rx) = channel();
        std::thread::Builder::new()
            .name("switchboard-controller".into())
            .spawn(move || run(port.as_deref(), &event_tx, &outbound_rx, &wake))
            .expect("spawn controller thread");
        Self { events, outbound }
    }
}

impl Controller for SerialController {
    fn poll(&mut self) -> Vec<ControllerEvent> {
        self.events.try_iter().collect()
    }

    fn set_waiting(&mut self, count: usize) {
        // The thread is gone only if the app is; nothing to do then.
        let _ = self.outbound.send(format!("W{count}\n"));
    }
}

/// The port to open: the Feather, found by its USB product name, and
/// nothing else. An unrelated board on a `usbmodem` port (a Trinkey,
/// say) must never be opened as a fallback: opening a CDC port whose
/// firmware never answers blocks inside the kernel, uninterruptibly,
/// and a thread stuck there pins the whole process at exit, immune to
/// force quit. Any other device is opted into with
/// `SWITCHBOARD_CONTROLLER`.
fn find_port() -> Option<String> {
    let ports = serialport::available_ports().unwrap_or_default();
    let mut names: Vec<String> = ports
        .into_iter()
        .filter(|p| p.port_name.contains("usbmodem"))
        .filter(|p| match &p.port_type {
            serialport::SerialPortType::UsbPort(usb) => usb
                .product
                .as_deref()
                .unwrap_or_default()
                .contains("Feather"),
            _ => false,
        })
        // macOS lists each device twice; the callout one is for us.
        .map(|p| p.port_name.replace("/dev/tty.", "/dev/cu."))
        .collect();
    names.sort();
    names.dedup();
    names.into_iter().next()
}

fn run(
    port: Option<&str>,
    events: &Sender<ControllerEvent>,
    outbound: &Receiver<String>,
    wake: &dyn Fn(),
) {
    loop {
        let Some(path) = port.map(str::to_owned).or_else(find_port) else {
            std::thread::sleep(RETRY);
            continue;
        };
        match serialport::new(&path, BAUD).timeout(READ_TIMEOUT).open() {
            Ok(opened) => {
                log::info!("controller on {path}");
                if events.send(ControllerEvent::Connected(true)).is_err() {
                    return;
                }
                wake();
                let why = session(opened, events, outbound, wake);
                log::info!("controller {path} lost: {why}");
                if events.send(ControllerEvent::Connected(false)).is_err() {
                    return;
                }
                wake();
            }
            Err(e) => log::debug!("controller {path}: {e}"),
        }
        std::thread::sleep(RETRY);
    }
}

/// Reads lines until the port fails; returns why.
fn session(
    port: Box<dyn serialport::SerialPort>,
    events: &Sender<ControllerEvent>,
    outbound: &Receiver<String>,
    wake: &dyn Fn(),
) -> String {
    let mut writer = match port.try_clone() {
        Ok(w) => w,
        Err(e) => return format!("clone: {e}"),
    };
    let mut reader = BufReader::new(port);
    let mut line = String::new();
    let mut last_ping: Option<Instant> = None;
    loop {
        if last_ping.is_none_or(|t| t.elapsed() >= KEEPALIVE) {
            if let Err(e) = writer.write_all(b"P\n") {
                return format!("write: {e}");
            }
            last_ping = Some(Instant::now());
        }
        loop {
            match outbound.try_recv() {
                Ok(text) => {
                    if let Err(e) = writer.write_all(text.as_bytes()) {
                        return format!("write: {e}");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return "app gone".into(),
            }
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return "eof".into(),
            Ok(_) => {
                if let Some(event) = ControllerEvent::parse(&line) {
                    if events.send(event).is_err() {
                        return "app gone".into();
                    }
                    wake();
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return format!("read: {e}"),
        }
    }
}
