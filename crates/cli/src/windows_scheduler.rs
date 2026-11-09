//! The per-user Task Scheduler entry that starts the daemon at Windows logon.
//!
//! The task runs with the interactive user's ordinary token. `IgnoreNew` is
//! important: both logon and `equipe daemon start` may ask Task Scheduler to
//! run it, but only one daemon process may be kept.

#![cfg_attr(not(windows), allow(dead_code))]

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};

pub const TASK_NAME: &str = r"\equipe";

pub fn installed() -> bool {
    command([
        OsString::from("/Query"),
        OsString::from("/TN"),
        OsString::from(TASK_NAME),
    ])
    .is_ok_and(|out| out.status.success())
}

pub fn install(exe: &Path, task_file: &Path) -> io::Result<()> {
    // Updating an existing task while its old action still runs can leave the
    // old process beside the new one. The caller already requested a graceful
    // daemon stop; `/End` closes the scheduler instance before replacing it.
    if installed() {
        let _ = command([
            OsString::from("/End"),
            OsString::from("/TN"),
            OsString::from(TASK_NAME),
        ]);
    }
    let sid = current_user_sid()?;
    let xml = task_xml(exe, &sid);
    if let Some(parent) = task_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(task_file, xml.as_bytes())?;
    let result = command([
        OsString::from("/Create"),
        OsString::from("/TN"),
        OsString::from(TASK_NAME),
        OsString::from("/XML"),
        task_file.as_os_str().to_owned(),
        OsString::from("/F"),
    ]);
    let _ = std::fs::remove_file(task_file);
    successful(result?, "create the equipe logon task")
}

pub fn start() -> io::Result<()> {
    successful(
        command([
            OsString::from("/Run"),
            OsString::from("/TN"),
            OsString::from(TASK_NAME),
        ])?,
        "start the equipe logon task",
    )
}

pub fn restart() -> io::Result<()> {
    // `/End` also returns failure when the task is already stopped. That is
    // precisely the state wanted before `/Run`, so it is deliberately ignored.
    let _ = command([
        OsString::from("/End"),
        OsString::from("/TN"),
        OsString::from(TASK_NAME),
    ]);
    start()
}

pub fn uninstall() -> io::Result<()> {
    let _ = command([
        OsString::from("/End"),
        OsString::from("/TN"),
        OsString::from(TASK_NAME),
    ]);
    successful(
        command([
            OsString::from("/Delete"),
            OsString::from("/TN"),
            OsString::from(TASK_NAME),
            OsString::from("/F"),
        ])?,
        "remove the equipe logon task",
    )
}

fn command(args: impl IntoIterator<Item = OsString>) -> io::Result<Output> {
    Command::new("schtasks.exe")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
}

fn successful(output: Output, doing: &str) -> io::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    });
    Err(io::Error::other(format!(
        "could not {doing}: {}",
        said.trim()
    )))
}

fn current_user_sid() -> io::Result<String> {
    let output = Command::new("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            "could not find the current Windows user SID",
        ));
    }
    parse_whoami_sid(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| io::Error::other("whoami did not return the current Windows user SID"))
}

fn parse_whoami_sid(text: &str) -> Option<String> {
    // `/fo csv` quotes both fields. Looking for the SID instead of depending
    // on a localized column heading also handles spaces and non-ASCII names.
    text.trim_matches(['\r', '\n', ' '])
        .split(['"', ','])
        .map(str::trim)
        .find(|field| {
            field.starts_with("S-1-")
                && field
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == 'S' || c == '-')
        })
        .map(str::to_owned)
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn task_xml(exe: &Path, sid: &str) -> String {
    let exe = escape_xml(&exe.to_string_lossy());
    let sid = escape_xml(sid);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>equipe daemon</Description><URI>\equipe</URI></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>daemon run</Arguments></Exec></Actions>
</Task>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_runs_once_at_logon_with_a_unicode_long_path() {
        let exe = Path::new(r"C:\Users\고 의성\a very long folder & tools\equipe.exe");
        let xml = task_xml(exe, "S-1-5-21-1-2-3-1001");
        assert!(xml.contains("<LogonTrigger>"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<RestartOnFailure>"));
        assert!(xml.contains(r"C:\Users\고 의성\a very long folder &amp; tools\equipe.exe"));
        assert!(xml.contains("<Arguments>daemon run</Arguments>"));
    }

    #[test]
    fn reads_the_sid_from_crlf_whoami_csv() {
        assert_eq!(
            parse_whoami_sid("\"DESKTOP\\고 의성\",\"S-1-5-21-123-456-789-1001\"\r\n").as_deref(),
            Some("S-1-5-21-123-456-789-1001")
        );
    }
}
