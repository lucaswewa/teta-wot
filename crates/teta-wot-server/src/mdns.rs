//! DNS-SD over mDNS, with the `mdns` feature.
//!
//! The server advertises itself as a Thing Description Directory, as W3C
//! WoT Discovery describes: a `_wot._tcp` service with the `_directory`
//! subtype, whose TXT record gives the path of its TD (`td`, the well-known
//! URL), its `type` (`Directory`) and its `scheme` (`http`). A client browses
//! for `_directory._sub._wot._tcp.local.`, reads `/.well-known/wot`, and
//! finds the Things in the directory.

use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceInfo};

/// The DNS-SD service type of WoT Things and directories.
pub const WOT_SERVICE_TYPE: &str = "_wot._tcp.local.";

/// The DNS-SD service type of a TD Directory: `_wot._tcp` with the
/// `_directory` subtype.
pub const DIRECTORY_SERVICE_TYPE: &str = "_directory._sub._wot._tcp.local.";

/// A registered advertisement, withdrawn by [`Advertisement::stop`].
pub(crate) struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertisement {
    /// Advertises the directory at `port`, as `instance` on the host
    /// `{instance}.local.`, with every address of the computer.
    pub(crate) fn start(instance: &str, port: u16) -> Result<Self, mdns_sd::Error> {
        let daemon = ServiceDaemon::new()?;
        let host: String = instance
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let properties = [
            ("td", "/.well-known/wot"),
            ("type", "Directory"),
            ("scheme", "http"),
        ];
        let info = ServiceInfo::new(
            DIRECTORY_SERVICE_TYPE,
            instance,
            &format!("{host}.local."),
            "",
            port,
            &properties[..],
        )?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_owned();
        daemon.register(info)?;
        Ok(Self { daemon, fullname })
    }

    /// Withdraws the advertisement (a DNS-SD goodbye) and stops the daemon.
    /// Blocks for up to a second.
    pub(crate) fn stop(self) {
        if let Ok(done) = self.daemon.unregister(&self.fullname) {
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
        let _ = self.daemon.shutdown();
    }
}
