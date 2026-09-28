//! The nunchuk over USB serial: the Feather in `firmware/nunchuk` prints
//! one line per button change or stick flick. A thread owns the port,
//! reconnects when the cable is pulled, and hands events over a channel;
//! the app drains them on its poll. The waiting count goes back down the
//! same port for the LED strip.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use crate::ports::controller::{Controller, ControllerEvent};

const BAUD: u32 = 115_200;
/// How long a read waits before the thread checks for something to
/// send and whether the port is still there.
const READ_TIMEOUT: Duration = Duration::from_millis(200);
const RETRY: Duration = Duration::from_secs(1);

pub struct SerialController {
    events: Receiver<ControllerEvent>,
    outbound: Sender<String>,
}

impl SerialController {
    /// Starts the port thread. `port` names the device
    /// (`SWITCHBOARD_CONTROLLER`), else the first `usbmodem` port is
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

/// Adafruit's USB vendor id: the Feather, and other `CircuitPython`
/// boards that may be plugged in beside it.
const ADAFRUIT: u16 = 0x239a;

/// The port to open: a Feather by its USB product name, else any
/// Adafruit board, else the first `usbmodem` port. Another board on
/// the same machine (a Trinkey, say) must not win by sorting first.
fn find_port() -> Option<String> {
    let ports = serialport::available_ports().unwrap_or_default();
    let mut ranked: Vec<(u8, String)> = ports
        .into_iter()
        .filter(|p| p.port_name.contains("usbmodem"))
        .map(|p| {
            let rank = match &p.port_type {
                serialport::SerialPortType::UsbPort(usb) => {
                    let product = usb.product.as_deref().unwrap_or_default();
                    if product.contains("Feather") {
                        0
                    } else if usb.vid == ADAFRUIT {
                        1
                    } else {
                        2
                    }
                }
                _ => 3,
            };
            // macOS lists each device twice; the callout one is for us.
            (rank, p.port_name.replace("/dev/tty.", "/dev/cu."))
        })
        .collect();
    ranked.sort();
    ranked.dedup();
    ranked.into_iter().next().map(|(_, name)| name)
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
    loop {
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
