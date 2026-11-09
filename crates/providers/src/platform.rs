//! Small host-platform operations shared by every equipe front end.

use chrono::{Datelike as _, Local, NaiveDate, TimeZone as _, Timelike as _, Utc};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// Sunday is zero, matching the old C `tm_wday` representation.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

fn calendar<T: chrono::TimeZone>(at: chrono::DateTime<T>) -> CalendarTime {
    CalendarTime {
        year: at.year(),
        month: at.month(),
        day: at.day(),
        weekday: at.weekday().num_days_from_sunday(),
        hour: at.hour(),
        minute: at.minute(),
        second: at.second(),
    }
}

pub fn local_time(unix: u64) -> Option<CalendarTime> {
    let unix = i64::try_from(unix).ok()?;
    Local.timestamp_opt(unix, 0).single().map(calendar)
}

pub fn utc_time(unix: u64) -> Option<CalendarTime> {
    let unix = i64::try_from(unix).ok()?;
    Utc.timestamp_opt(unix, 0).single().map(calendar)
}

/// Local calendar fields to Unix seconds. `day` may run beyond its month's
/// bounds; routine scheduling relies on that to carry into the next month.
pub fn local_unix(year: i32, month: u32, day: i32, hour: u8, minute: u8) -> Option<u64> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let date = first.checked_add_signed(chrono::Duration::days(i64::from(day) - 1))?;
    let local = date.and_hms_opt(u32::from(hour), u32::from(minute), 0)?;
    let at = Local.from_local_datetime(&local).earliest()?;
    u64::try_from(at.timestamp()).ok()
}

pub fn utc_unix(year: i32, month: u32, day: i32, hour: u8, minute: u8) -> Option<u64> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let date = first.checked_add_signed(chrono::Duration::days(i64::from(day) - 1))?;
    let at = date.and_hms_opt(u32::from(hour), u32::from(minute), 0)?.and_utc();
    u64::try_from(at.timestamp()).ok()
}

pub fn local_midnight(unix: u64) -> Option<u64> {
    let local = local_time(unix)?;
    local_unix(local.year, local.month, local.day as i32, 0, 0)
}

/// `$EQUIPE_HOME`, otherwise the current user's `.equipe` folder. On Windows
/// `dirs::home_dir` resolves `%USERPROFILE%`, so the result is
/// `%USERPROFILE%\.equipe`.
pub fn equipe_home() -> PathBuf {
    match std::env::var_os("EQUIPE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".equipe"),
    }
}

fn open_command(target: &OsStr) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(target);
        command
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("explorer");
        command.arg(target);
        command
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(target);
        command
    }
}

pub fn open(target: impl AsRef<OsStr>) -> std::io::Result<Child> {
    open_command(target.as_ref()).spawn()
}

pub fn open_status(target: impl AsRef<OsStr>) -> std::io::Result<ExitStatus> {
    open_command(target.as_ref()).status()
}

pub fn reveal(target: impl AsRef<Path>) -> std::io::Result<Child> {
    let target = target.as_ref();
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg("-R").arg(target).spawn()
    }
    #[cfg(windows)]
    {
        Command::new("explorer").arg(format!("/select,{}", target.display())).spawn()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        open(target.parent().unwrap_or(target))
    }
}

/// A URL opener whose exit status can be checked by login flows.
pub fn open_url_status(url: &str) -> std::io::Result<ExitStatus> {
    #[cfg(windows)]
    {
        Command::new("cmd").args(["/c", "start", "", url]).status()
    }
    #[cfg(not(windows))]
    {
        open_status(url)
    }
}
