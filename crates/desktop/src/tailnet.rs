//! The machines around this Mac, as the chooser shows them: the Tailscale
//! devices it can reach (`tailscale status --json`) — offered by name, naming
//! an address typed by hand, catching one typed a digit wrong — and whether a
//! host answers on its SSH port at all.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::target::SshHost;

/// A computer on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// What its owner calls it ("Abel의 Mac mini").
    pub name: String,
    /// Its MagicDNS name, without the last dot (`tinums-mac-mini.tail3383cf.ts.net`).
    pub dns: String,
    /// Its IPv4 address on the tailnet.
    pub ip: String,
    /// When Tailscale last saw it (unix seconds), if not online now.
    pub last_seen: Option<u64>,
    pub online: bool,
}

impl Peer {
    /// The name to connect by: the MagicDNS one, else the address.
    pub fn host(&self) -> &str {
        if self.dns.is_empty() { &self.ip } else { &self.dns }
    }

    /// Its MagicDNS name's first label (`tinums-mac-mini`).
    pub fn short(&self) -> &str {
        self.dns.split('.').next().filter(|s| !s.is_empty()).unwrap_or(&self.ip)
    }
}

#[derive(Deserialize)]
struct Status {
    #[serde(rename = "Peer", default)]
    peer: std::collections::HashMap<String, Node>,
}

#[derive(Deserialize)]
struct Node {
    #[serde(rename = "HostName", default)]
    host_name: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(rename = "TailscaleIPs", default)]
    ips: Vec<String>,
    #[serde(rename = "Online", default)]
    online: bool,
    #[serde(rename = "LastSeen", default)]
    last_seen: Option<String>,
}

/// The computers in `tailscale status --json` (not phones, not Tailscale's
/// own nodes), online first, then by name.
pub fn parse(json: &str) -> Vec<Peer> {
    let Ok(status) = serde_json::from_str::<Status>(json) else { return Vec::new() };
    let mut peers: Vec<Peer> = status
        .peer
        .into_values()
        .filter(|n| matches!(n.os.to_ascii_lowercase().as_str(), "macos" | "linux" | "windows" | "freebsd" | "openbsd"))
        .filter_map(|n| {
            let ip = n.ips.into_iter().find(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())?;
            let dns = n.dns_name.trim_end_matches('.').to_owned();
            let name = n.host_name.replace('\u{a0}', " ");
            let last_seen = if n.online { None } else { n.last_seen.as_deref().and_then(unix_of) };
            Some(Peer { name, dns, ip, last_seen, online: n.online })
        })
        .collect();
    // Seen most lately first: Tailscale's "online" lags behind for a device
    // shared from another tailnet, so when it was last seen says more.
    peers.sort_by(|a, b| a.last_seen.map(std::cmp::Reverse).cmp(&b.last_seen.map(std::cmp::Reverse)).then_with(|| a.short().cmp(b.short())));
    peers
}

/// Devices not seen for this long are left out of the list, until searched for.
pub const STALE: u64 = 14 * 86_400;

impl Peer {
    /// Seen within [`STALE`] of `now`.
    pub fn fresh(&self, now: u64) -> bool {
        self.last_seen.is_none_or(|at| now.saturating_sub(at) < STALE)
    }
}

/// The Tailscale CLI, wherever it was installed.
fn binary() -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).map(|d| d.join("tailscale"));
    on_path
        .chain(["/opt/homebrew/bin/tailscale", "/usr/local/bin/tailscale", "/Applications/Tailscale.app/Contents/MacOS/Tailscale"].map(PathBuf::from))
        .find(|p| p.is_file())
}

/// The tailnet as Tailscale on this Mac sees it; none without Tailscale.
/// Blocks briefly: call it off the UI thread.
pub fn load() -> Vec<Peer> {
    let Some(bin) = binary() else { return Vec::new() };
    match Command::new(bin).args(["status", "--json"]).output() {
        Ok(out) => parse(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

/// The device `host` names: its address, its MagicDNS name or that name's
/// first label.
pub fn peer_for<'a>(peers: &'a [Peer], host: &str) -> Option<&'a Peer> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    peers.iter().find(|p| p.ip == host || p.dns.eq_ignore_ascii_case(&host) || (!p.dns.is_empty() && p.short().eq_ignore_ascii_case(&host)))
}

/// The device `host` was most likely meant to be, when it names none: one
/// digit off its address (`110.66.18.3` for `100.66.18.3`), or a letter or
/// two off its name. Only an unambiguous match.
pub fn near<'a>(peers: &'a [Peer], host: &str) -> Option<&'a Peer> {
    if peer_for(peers, host).is_some() {
        return None;
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let is_ip = host.parse::<std::net::Ipv4Addr>().is_ok();
    let close: Vec<&Peer> = peers
        .iter()
        .filter(|p| {
            if is_ip {
                distance(&host, &p.ip) == 1
            } else {
                let name = host.split('.').next().unwrap_or(&host);
                name.len() >= 5 && !p.dns.is_empty() && distance(name, &p.short().to_ascii_lowercase()) <= 2
            }
        })
        .collect();
    match close.as_slice() {
        [one] => Some(one),
        _ => None,
    }
}

/// Edits (insert, delete, change one character) from `a` to `b`.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(here + 1);
            prev = here;
        }
    }
    row[b.len()]
}

/// `2026-10-30T05:15:59.1Z` as unix seconds (UTC; fractions dropped).
fn unix_of(text: &str) -> Option<u64> {
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-').map(|n| n.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let mut t = time.split(':');
    let (h, min) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec = t.next()?.split('.').next()?.parse::<i64>().ok()?;
    // Days from 1970-01-01 to the date (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + min * 60 + sec).ok()
}

/// Whether a host answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Checking,
    /// Its SSH port took the connection.
    Answers,
    /// Nothing answered in time.
    Silent,
    /// Reached through another machine (a jump host): not checked.
    Unknown,
}

/// Whether `host`'s SSH port takes a connection, where `ssh` would go (an
/// alias of `~/.ssh/config` resolved by `ssh -G`). Blocks up to a few
/// seconds: call it off the UI thread.
pub fn reach(host: &SshHost) -> Reach {
    let mut args = vec!["-G".to_owned()];
    args.extend(host.args());
    let Ok(out) = Command::new("ssh").args(&args).output() else { return Reach::Unknown };
    let config = String::from_utf8_lossy(&out.stdout);
    let value = |key: &str| config.lines().find_map(|l| l.strip_prefix(key).map(|v| v.trim().to_owned()));
    if value("proxyjump ").is_some_and(|v| v != "none") || value("proxycommand ").is_some_and(|v| v != "none") {
        return Reach::Unknown;
    }
    let name = value("hostname ").unwrap_or_else(|| host.host.clone());
    let port = value("port ").and_then(|p| p.parse::<u16>().ok()).unwrap_or(22);
    let Ok(addrs) = (name.as_str(), port).to_socket_addrs() else { return Reach::Silent };
    if addrs.take(2).any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(2500)).is_ok()) { Reach::Answers } else { Reach::Silent }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = r#"{
        "Self": {"HostName": "Abel의 Mac mini", "DNSName": "abel-macmini.tail5abbbf.ts.net.", "OS": "macOS", "TailscaleIPs": ["100.106.17.55"], "Online": true},
        "Peer": {
            "a": {"HostName": "funnel-ingress-node", "DNSName": "", "OS": "", "TailscaleIPs": ["fd7a::1"], "Online": false},
            "b": {"HostName": "tinums-mac-mini", "DNSName": "tinums-mac-mini.tail3383cf.ts.net.", "OS": "macOS", "TailscaleIPs": ["100.66.18.3", "fd7a::2"], "Online": false, "LastSeen": "2026-10-30T05:15:59.1Z"},
            "c": {"HostName": "iphone-12", "DNSName": "iphone-12.tail5abbbf.ts.net.", "OS": "iOS", "TailscaleIPs": ["100.79.46.87"], "Online": true},
            "d": {"HostName": "Abel의 MacBook Air", "DNSName": "abels-macbook-air.tail5abbbf.ts.net.", "OS": "macOS", "TailscaleIPs": ["100.93.184.56"], "Online": true}
        }
    }"#;

    #[test]
    fn the_tailnets_computers_are_listed_by_when_last_seen_without_phones_or_itself() {
        let peers = parse(STATUS);
        let names: Vec<&str> = peers.iter().map(Peer::short).collect();
        assert_eq!(names, ["abels-macbook-air", "tinums-mac-mini"]);
        assert_eq!(peers[0].name, "Abel의 MacBook Air", "a no-break space reads as a space");
        assert_eq!(peers[1].host(), "tinums-mac-mini.tail3383cf.ts.net");
        assert_eq!(peers[1].last_seen, Some(1_793_337_359), "2026-10-30 05:15:59 UTC");
        assert!(parse("not json").is_empty());
        assert!(peers[1].fresh(1_793_337_359 + 86_400) && !peers[1].fresh(1_793_337_359 + STALE), "two weeks unseen: stale");
    }

    #[test]
    fn an_address_or_name_is_matched_to_its_device_and_a_typo_to_the_one_meant() {
        let peers = parse(STATUS);
        assert_eq!(peer_for(&peers, "100.66.18.3").map(Peer::short), Some("tinums-mac-mini"));
        assert_eq!(peer_for(&peers, "tinums-mac-mini").map(Peer::short), Some("tinums-mac-mini"));
        assert_eq!(peer_for(&peers, "TINUMS-MAC-MINI.tail3383cf.ts.net.").map(Peer::short), Some("tinums-mac-mini"));
        assert_eq!(near(&peers, "110.66.18.3").map(|p| p.ip.as_str()), Some("100.66.18.3"), "one digit off");
        assert_eq!(near(&peers, "tinums-macmini").map(Peer::short), Some("tinums-mac-mini"), "a dash short");
        assert_eq!(near(&peers, "100.66.18.3"), None, "the right one needs no guess");
        assert_eq!(near(&peers, "8.8.8.8"), None);
        assert_eq!(near(&peers, "ab"), None, "too short to guess at");
    }

    #[test]
    fn a_host_typed_with_a_port_is_checked_where_ssh_would_go() {
        // Nothing listens on port 1 of this Mac: silent, quickly.
        assert_eq!(reach(&SshHost::parse("127.0.0.1:1").unwrap()), Reach::Silent);
    }
}
