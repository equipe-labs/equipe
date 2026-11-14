//! A sign-in page's last step brought back to the daemon's Mac. An OAuth
//! page opened here ends by redirecting to `http://localhost:<port>/…` —
//! the loopback listener of the sign-in that runs on the daemon's machine
//! (`equipe_core::cli_signin::loopback_port`). With the daemon on another
//! Mac over SSH, that port is forwarded there for the sign-in's length: the
//! window's own shared connection gets the forward (`ssh -O forward`), or,
//! without one, an `ssh -N -L` of its own, never asking anything. Closed
//! when the sign-in ends. Plain `ssh`, the person's own SSH setup.

use std::net::{Ipv4Addr, Ipv6Addr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::ssh::Launcher;
use crate::target::SshHost;

/// A forward up for one sign-in.
pub struct Tunnel {
    host: SshHost,
    port: u16,
    /// Its own `ssh -N`, when the shared connection didn't take it.
    own: Option<Child>,
}

/// Whether nothing on this Mac listens on `port` (both loopback addresses).
pub fn port_free(port: u16) -> bool {
    let four = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok();
    let six = match TcpListener::bind((Ipv6Addr::LOCALHOST, port)) {
        Ok(_) => true,
        Err(e) => e.kind() == std::io::ErrorKind::AddrNotAvailable,
    };
    four && six
}

/// The `-L` that brings `port` here to the same port on the far machine's loopback.
pub fn forward_spec(port: u16) -> String {
    format!("localhost:{port}:localhost:{port}")
}

fn control(launcher: &Launcher) -> String {
    format!("ControlPath={}", launcher.control_dir.join("c-%C").display())
}

fn quietly(command: &mut Command) -> &mut Command {
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
}

/// Forward `port` to `host`'s loopback for a sign-in. Blocking (run it off
/// the window's thread). Why not, when the port is taken here or no
/// connection would take the forward without asking.
pub fn open(host: &SshHost, port: u16) -> Result<Tunnel, String> {
    if !port_free(port) {
        return Err(format!("port {port} is in use on this Mac"));
    }
    let launcher = Launcher::system(None);
    // The window's shared connection, when it is up: the forward rides on it.
    let shared = quietly(Command::new(&launcher.program).args(["-O", "forward", "-L", &forward_spec(port), "-o", &control(&launcher)]).args(&launcher.extra).args(host.args())).status();
    if shared.is_ok_and(|s| s.success()) && reachable(port, Duration::from_secs(2)) {
        return Ok(Tunnel { host: host.clone(), port, own: None });
    }
    // Else one of its own, never asking (no password box for this).
    let mut own = quietly(
        Command::new(&launcher.program)
            .args(["-N", "-T", "-o", "ExitOnForwardFailure=yes", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ControlMaster=no", "-o", "ControlPath=none", "-L", &forward_spec(port)])
            .args(&launcher.extra)
            .args(crate::sshkey::identity_options(host))
            .args(host.args()),
    )
    .spawn()
    .map_err(|e| format!("ssh: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = own.try_wait() {
            return Err("ssh couldn't forward the port".to_owned());
        }
        if reachable(port, Duration::from_millis(200)) {
            return Ok(Tunnel { host: host.clone(), port, own: Some(own) });
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = own.kill();
    let _ = own.wait();
    Err("ssh didn't forward the port in time".to_owned())
}

/// Whether `port` here takes a connection within `wait`.
fn reachable(port: u16, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        if TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_millis(200)).is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Tunnel {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Take the forward down. Blocking.
    pub fn close(mut self) {
        match self.own.take() {
            Some(mut own) => {
                let _ = own.kill();
                let _ = own.wait();
            }
            None => {
                let launcher = Launcher::system(None);
                let _ = quietly(Command::new(&launcher.program).args(["-O", "cancel", "-L", &forward_spec(self.port), "-o", &control(&launcher)]).args(&launcher.extra).args(self.host.args())).status();
            }
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Some(mut own) = self.own.take() {
            let _ = own.kill();
            let _ = own.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_forward_is_the_same_port_on_both_loopbacks_and_a_taken_port_is_seen() {
        assert_eq!(forward_spec(55531), "localhost:55531:localhost:55531");
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert!(!port_free(port), "in use here: the panel instead");
        drop(taken);
        let host = SshHost { user: None, host: "far.example.invalid".into(), port: None };
        assert!(open(&host, port).is_err() || port_free(port), "never a forward over a port that is taken");
    }
}
