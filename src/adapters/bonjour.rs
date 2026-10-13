//! Bonjour advertisement of the remote port, through `mdns-sd` (pure
//! Rust, no daemon and no runtime) on macOS. The multicast responder
//! lives for as long as the adapter holds its daemon; dropping the
//! adapter withdraws the record. Elsewhere the adapter is a stub that
//! advertises nothing, so Linux CI builds without the crate.

use crate::ports::remote::{Advertisement, Advertiser};

/// The host's one Bonjour record.
#[derive(Default)]
pub struct Bonjour {
    #[cfg(target_os = "macos")]
    daemon: Option<mdns_sd::ServiceDaemon>,
    #[cfg(target_os = "macos")]
    registered: Option<String>,
}

#[cfg(target_os = "macos")]
impl Bonjour {
    fn service_type() -> String {
        format!("{}.local.", Advertisement::SERVICE)
    }

    fn unregister(&mut self) {
        if let (Some(daemon), Some(name)) = (&self.daemon, self.registered.take()) {
            let full = format!("{name}.{}", Self::service_type());
            if let Err(e) = daemon.unregister(&full) {
                log::warn!("bonjour: withdraw {name}: {e}");
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl Advertiser for Bonjour {
    fn advertise(&mut self, ad: &Advertisement) -> Result<(), String> {
        self.unregister();
        if self.daemon.is_none() {
            self.daemon = Some(mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?);
        }
        let host = format!("{}.local.", super::remote::default_server_name());
        let txt: Vec<(String, String)> = ad.txt();
        let info = mdns_sd::ServiceInfo::new(
            &Self::service_type(),
            &ad.instance,
            &host,
            "",
            ad.port,
            &txt[..],
        )
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
        self.daemon
            .as_ref()
            .ok_or("no daemon")?
            .register(info)
            .map_err(|e| e.to_string())?;
        self.registered = Some(ad.instance.clone());
        log::info!("bonjour: advertising {:?} on port {}", ad.instance, ad.port);
        Ok(())
    }

    fn withdraw(&mut self) {
        self.unregister();
    }
}

#[cfg(target_os = "macos")]
impl Drop for Bonjour {
    fn drop(&mut self) {
        self.unregister();
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.shutdown();
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
    use std::time::Duration;

    /// Registers a record and browses for it on this machine. Needs
    /// multicast on an interface, which some networks refuse, so it is
    /// run by hand:
    /// `cargo test --locked -p switchboard --lib bonjour -- --ignored`
    #[test]
    #[ignore = "needs multicast on an interface; run by hand"]
    fn the_record_is_browsable_until_withdrawn() {
        let id = uuid::Uuid::new_v4();
        let ad = Advertisement::new(id, "Test host", 43999);
        let mut bonjour = Bonjour::default();
        bonjour.advertise(&ad).unwrap();
        let browser = mdns_sd::ServiceDaemon::new().unwrap();
        let rx = browser.browse(&Bonjour::service_type()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut seen = None;
        while seen.is_none() && std::time::Instant::now() < deadline {
            if let Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) =
                rx.recv_timeout(Duration::from_millis(500))
                && info.get_fullname().starts_with(&ad.instance)
            {
                seen = Some(info);
            }
        }
        let info = seen.expect("the record was browsable");
        assert_eq!(info.get_port(), 43999);
        assert_eq!(
            info.get_property_val_str("id"),
            Some(id.to_string().as_str())
        );
        assert_eq!(info.get_property_val_str("v"), Some("1"));
        bonjour.withdraw();
        let _ = browser.shutdown();
    }
}
