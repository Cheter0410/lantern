//! Lantern on the network (mDNS / Bonjour): announces its OSC port so Show
//! Master lists this computer in the Lantern cue, announces the RTP-MIDI
//! session while it runs, and lists the other RTP-MIDI sessions.

use sm_mdns::{Desired, Mdns, RtpAdvert};
use std::time::{Duration, Instant};

pub struct Net {
    mdns: Mdns,
    name: String,
    at: Instant,
    last: Option<Desired>,
}

impl Default for Net {
    fn default() -> Self {
        let name = sm_mdns::computer_name();
        Net {
            mdns: Mdns::with_options(sm_mdns::MDNS_PORT, "lantern", &name),
            name,
            at: Instant::now() - Duration::from_secs(1),
            last: None,
        }
    }
}

impl Net {
    /// The name Show Master lists this computer under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Keep the announcements in line (cheap; call every frame).
    /// `web_port`: the web remote, announced as `http://lantern.local:<port>`.
    pub fn update(&mut self, osc_port: Option<u16>, rtp: Option<(String, u16)>, web_port: Option<u16>) {
        if self.at.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.at = Instant::now();
        let want = Desired {
            http_port: web_port,
            lantern: osc_port.map(|port| RtpAdvert {
                name: self.name.clone(),
                port,
            }),
            browse_rtp: rtp.is_some(),
            rtp: rtp.map(|(name, port)| RtpAdvert { name, port }),
            ..Desired::default()
        };
        if self.last.as_ref() != Some(&want) {
            self.mdns.apply(&want);
            self.last = Some(want);
        }
        self.mdns.poll();
    }

    /// Other RTP-MIDI sessions on the network (name, `ip:port`).
    pub fn rtp_sessions(&self) -> Vec<(String, String)> {
        self.mdns
            .status()
            .discovered
            .into_iter()
            .map(|d| (d.name, d.addr))
            .collect()
    }

    /// The web remote's name address (`http://lantern.local:8090`) while it is announced.
    pub fn web_url(&self) -> Option<String> {
        self.mdns.status().url
    }

    /// A problem with the network announcement, if any.
    pub fn problem(&self) -> Option<String> {
        self.mdns.status().error
    }
}
