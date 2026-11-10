//! The host's Bonjour record registered with the system's mDNSResponder
//! through `DNSServiceRegister` (`dns_sd.h`, in libSystem), so the Mac
//! runs one multicast responder, not two. This is the one module in the
//! tree that may use `unsafe`: the workspace lint is `deny`, and
//! `adapters/mod.rs` allows it for this module alone.
//!
//! Two promises hold the module together: a `DNSServiceRef` stays valid
//! until `DNSServiceRefDeallocate`, and only one thread ever touches it.
//! A worker thread creates, services and deallocates the ref, and the
//! rest of the app only sends it commands.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::mpsc::{self, Receiver, RecvError, Sender, TryRecvError};
use std::thread::JoinHandle;

use crate::ports::remote::Advertisement;

/// mDNSResponder's handle for one registration; opaque on this side.
#[repr(C)]
struct DnsServiceOpaque {
    _private: [u8; 0],
}

type DnsServiceRef = *mut DnsServiceOpaque;

type RegisterReply = extern "C" fn(
    sd_ref: DnsServiceRef,
    flags: u32,
    error: i32,
    name: *const c_char,
    regtype: *const c_char,
    domain: *const c_char,
    context: *mut c_void,
);

/// `struct pollfd` from `<poll.h>`.
#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 0x0001;
const POLLERR: i16 = 0x0008;
const POLLHUP: i16 = 0x0010;
const POLLNVAL: i16 = 0x0020;
/// `kDNSServiceFlagsAdd`: set on a callback that reports the record in
/// force, unset when a later conflict took it away.
const FLAGS_ADD: u32 = 0x2;
/// How long the worker waits on mDNSResponder before it looks for a
/// command; bounds how late `withdraw` and `Drop` are answered.
const POLL_MS: c_int = 100;

// Signatures copied from the macOS SDK's `dns_sd.h` and `<poll.h>`.
// Declaring them is `unsafe extern` because the compiler takes these
// types on trust: nothing checks them against the C headers.
unsafe extern "C" {
    fn DNSServiceRegister(
        sd_ref: *mut DnsServiceRef,
        flags: u32,
        interface_index: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        host: *const c_char,
        port: u16,
        txt_len: u16,
        txt_record: *const c_void,
        callback: Option<RegisterReply>,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRefSockFD(sd_ref: DnsServiceRef) -> c_int;
    fn DNSServiceProcessResult(sd_ref: DnsServiceRef) -> i32;
    fn DNSServiceRefDeallocate(sd_ref: DnsServiceRef);
    fn poll(fds: *mut PollFd, nfds: u32, timeout: c_int) -> c_int;
}

/// The TXT record in DNS wire form: each `key=value` as one string
/// prefixed by its length byte. An empty record is one empty string, as
/// `dns_sd.h` asks.
fn txt_wire(entries: &[(String, String)]) -> Result<Vec<u8>, String> {
    if entries.is_empty() {
        return Ok(vec![0]);
    }
    let mut out = Vec::new();
    for (key, value) in entries {
        let entry = format!("{key}={value}");
        let len = u8::try_from(entry.len())
            .map_err(|_| format!("TXT entry {key:?} is over 255 bytes"))?;
        out.push(len);
        out.extend_from_slice(entry.as_bytes());
    }
    if u16::try_from(out.len()).is_err() {
        return Err("TXT record is over 65535 bytes".to_owned());
    }
    Ok(out)
}

/// What the owner of the worker asks of it.
pub(super) enum Cmd {
    /// Replace whatever is registered with this advertisement, unless it
    /// is already registered. The reply says whether a registration was
    /// made.
    Register(Advertisement, Sender<Result<bool, String>>),
    /// Deregister, if anything is registered.
    Withdraw(Sender<()>),
    /// Deregister and end the thread.
    Stop,
}

/// One live registration.
struct Registration {
    sd_ref: DnsServiceRef,
    /// What was registered, to tell an equal request from a new one.
    ad: Advertisement,
    /// The callback reads the instance name through a pointer into this
    /// buffer to tell a rename. Nothing on this side reads it; it is
    /// held so that pointer stays valid as long as the ref. A `CString`
    /// owns a heap buffer, so moving it into `Registration` does not
    /// move the bytes.
    _instance: CString,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: `sd_ref` came from a successful `DNSServiceRegister`
        // and is deallocated exactly once, here, on the worker thread
        // that services it; nothing polls its socket after this.
        unsafe { DNSServiceRefDeallocate(self.sd_ref) };
    }
}

/// Starts the worker that owns the host's registration.
pub(super) fn spawn() -> Result<(Sender<Cmd>, JoinHandle<()>), String> {
    let (tx, rx) = mpsc::channel();
    let join = std::thread::Builder::new()
        .name("bonjour".into())
        .spawn(move || run(&rx))
        .map_err(|e| format!("bonjour: worker: {e}"))?;
    Ok((tx, join))
}

fn run(rx: &Receiver<Cmd>) {
    let mut current: Option<Registration> = None;
    loop {
        let cmd = match &current {
            None => match rx.recv() {
                Ok(cmd) => cmd,
                Err(RecvError) => break,
            },
            Some(reg) => {
                if let Err(why) = service(reg) {
                    log::warn!("bonjour: mDNSResponder ended the registration: {why}");
                    current = None;
                    continue;
                }
                match rx.try_recv() {
                    Ok(cmd) => cmd,
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
        };
        match cmd {
            Cmd::Register(ad, reply) => {
                // An equal request is answered without a new
                // registration (that is a goodbye and a fresh probe).
                // `current` is empty after mDNSResponder dropped us, so
                // a request then registers again.
                let result = if current.as_ref().is_some_and(|reg| reg.ad == ad) {
                    Ok(false)
                } else {
                    // Cleared before registering: if the new one fails,
                    // nothing is registered, and the old record must not
                    // linger under a stale ad.
                    current = None;
                    match register(&ad) {
                        Ok(reg) => {
                            current = Some(reg);
                            Ok(true)
                        }
                        Err(e) => Err(e),
                    }
                };
                let _ = reply.send(result);
            }
            Cmd::Withdraw(reply) => {
                current = None;
                let _ = reply.send(());
            }
            Cmd::Stop => break,
        }
    }
}

/// Waits up to `POLL_MS` for mDNSResponder and runs the callback if it
/// answered. An error means the daemon dropped the connection (it
/// restarted, say), and the ref is no use any more.
fn service(reg: &Registration) -> Result<(), String> {
    // SAFETY: `unsafe` marks a promise the compiler cannot check across
    // the C boundary, not a switched-off borrow checker. Here: the ref is
    // live until `Registration` is dropped, which only this thread does,
    // after this call returns.
    let fd = unsafe { DNSServiceRefSockFD(reg.sd_ref) };
    if fd < 0 {
        return Err("no socket for the registration".to_owned());
    }
    let mut pfd = PollFd {
        fd,
        events: POLLIN,
        revents: 0,
    };
    // SAFETY: one valid `pollfd` that outlives the call.
    let ready = unsafe { poll(&raw mut pfd, 1, POLL_MS) };
    if ready > 0 && pfd.revents & (POLLERR | POLLHUP | POLLNVAL) != 0 {
        return Err(format!("socket closed (poll events {:#x})", pfd.revents));
    }
    if ready <= 0 || pfd.revents & POLLIN == 0 {
        return Ok(());
    }
    // SAFETY: as above; the callback runs inside this call, on this
    // thread, while the context (see `Registration::_instance`) is alive.
    match unsafe { DNSServiceProcessResult(reg.sd_ref) } {
        0 => Ok(()),
        code => Err(format!("error {code}")),
    }
}

fn register(ad: &Advertisement) -> Result<Registration, String> {
    let instance = CString::new(ad.instance.as_str())
        .map_err(|_| format!("instance name {:?} has a NUL", ad.instance))?;
    let regtype = CString::new(Advertisement::SERVICE).map_err(|e| e.to_string())?;
    let txt = txt_wire(&ad.txt())?;
    let txt_len = u16::try_from(txt.len()).map_err(|e| e.to_string())?;
    let mut sd_ref: DnsServiceRef = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call; mDNSResponder copies
    // the strings and the TXT bytes. The context is `instance`'s buffer;
    // see `Registration::_instance` for why it outlives the ref.
    let code = unsafe {
        DNSServiceRegister(
            &raw mut sd_ref,
            0,
            0,
            instance.as_ptr(),
            regtype.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            ad.port.to_be(),
            txt_len,
            txt.as_ptr().cast(),
            Some(on_register),
            instance.as_ptr().cast_mut().cast(),
        )
    };
    if code != 0 || sd_ref.is_null() {
        return Err(format!("DNSServiceRegister failed: {code}"));
    }
    Ok(Registration {
        sd_ref,
        ad: ad.clone(),
        _instance: instance,
    })
}

/// Called by mDNSResponder, inside `DNSServiceProcessResult`, once the
/// record is in force (under the name it settled on) or when it fails.
/// It only logs: the caller already has its answer from `register`.
extern "C" fn on_register(
    _sd_ref: DnsServiceRef,
    flags: u32,
    error: i32,
    name: *const c_char,
    _regtype: *const c_char,
    _domain: *const c_char,
    context: *mut c_void,
) {
    if error != 0 {
        log::warn!("bonjour: registration failed: {error}");
        return;
    }
    let read = |p: *const c_char| {
        if p.is_null() {
            String::new()
        } else {
            // SAFETY: mDNSResponder passes a NUL-terminated name valid
            // for the callback; the context is `Registration::_instance`.
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    };
    let used = read(name);
    let asked = read(context.cast_const().cast());
    if flags & FLAGS_ADD == 0 {
        log::warn!("bonjour: {used:?} was withdrawn by mDNSResponder");
    } else if used == asked {
        log::info!("bonjour: registered {used:?}");
    } else {
        log::warn!("bonjour: {asked:?} was taken; registered as {used:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_entries_are_length_prefixed_in_order() {
        let ad = Advertisement::new(uuid::Uuid::nil(), "Mac", 1);
        let wire = txt_wire(&ad.txt()).unwrap();
        let id = format!("id={}", uuid::Uuid::nil());
        let mut want = vec![3];
        want.extend_from_slice(b"v=1");
        want.push(8);
        want.extend_from_slice(b"name=Mac");
        want.push(u8::try_from(id.len()).unwrap());
        want.extend_from_slice(id.as_bytes());
        assert_eq!(wire, want);
    }

    #[test]
    fn an_entry_over_255_bytes_is_refused() {
        let long = vec![("name".to_owned(), "x".repeat(251))];
        assert!(txt_wire(&long).is_err());
        let fits = vec![("name".to_owned(), "x".repeat(250))];
        assert_eq!(txt_wire(&fits).unwrap().len(), 256);
    }

    #[test]
    fn an_empty_record_is_one_zero_byte() {
        assert_eq!(txt_wire(&[]).unwrap(), vec![0]);
    }
}
