//! Updates through [Sparkle](https://sparkle-project.org), the usual way a Mac
//! app outside the App Store updates itself. scripts/desktop-dmg.sh puts
//! Sparkle.framework in the bundle and the feed (`SUFeedURL`) and the key its
//! updates are signed with (`SUPublicEDKey`) in Info.plist; Sparkle checks the
//! feed once a day by itself, and the app menu's "업데이트 확인…" checks now.
//!
//! Run any other way (`cargo run`, a build without a signing key) there is no
//! framework next to the binary, so there is nothing to start and the menu item
//! is greyed out.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;

use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Bool};

/// Sparkle's standard updater: its own windows, its own schedule.
pub struct Updater {
    controller: *mut AnyObject,
}

impl Updater {
    /// Load the bundle's Sparkle and start it, if the app was packaged with it.
    /// Call on the main thread.
    pub fn start() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let framework = exe.parent()?.parent()?.join("Frameworks/Sparkle.framework/Sparkle");
        if !framework.is_file() {
            return None;
        }
        let path = CString::new(framework.as_os_str().as_bytes()).ok()?;
        // SAFETY: a framework of our own bundle, loaded once and never closed.
        if unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW) }.is_null() {
            eprintln!("equipe: couldn't load {}; updates are off", framework.display());
            return None;
        }
        let class = AnyClass::get(c"SPUStandardUpdaterController")?;
        let none: *mut AnyObject = std::ptr::null_mut();
        // SAFETY: Sparkle 2's documented initialiser; the controller is kept
        // for the life of the app (never released).
        let controller: *mut AnyObject = unsafe {
            let allocated: *mut AnyObject = msg_send![class, alloc];
            msg_send![allocated, initWithStartingUpdater: Bool::YES, updaterDelegate: none, userDriverDelegate: none]
        };
        (!controller.is_null()).then_some(Self { controller })
    }

    /// Look for an update now, saying so in Sparkle's window either way.
    pub fn check(&self) {
        let none: *mut AnyObject = std::ptr::null_mut();
        // SAFETY: an IBAction on the controller started above, on the main thread.
        unsafe {
            let _: () = msg_send![self.controller, checkForUpdates: none];
        }
    }
}
