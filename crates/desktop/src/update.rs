//! Updates through [Sparkle](https://sparkle-project.org), the usual way a Mac
//! app outside the App Store updates itself. scripts/desktop-dmg.sh puts
//! Sparkle.framework in the bundle and the feed (`SUFeedURL`) and the key its
//! updates are signed with (`SUPublicEDKey`) in Info.plist. Sparkle checks the
//! feed every hour by itself and, with `SUAutomaticallyUpdate`, downloads a new
//! version in the background and checks its signature. Ready, it tells our
//! delegate — which keeps Sparkle's "install now" and shows the sidebar's
//! "다시 시작해 업데이트" (src/relaunch.rs) — and installs it when the app
//! quits in any case. The app menu's "업데이트 확인…" checks now.
//!
//! Every step Sparkle takes comes to the delegate — found, downloading,
//! downloaded, failed, the cycle's end — and goes to `relaunch` (the sidebar's
//! pill, settings, the update notices) and, with any error whole (domain,
//! code, userInfo, the underlying errors), to ~/Library/Logs/equipe/updates.log.
//!
//! Sparkle reads no feed while an update waits (it resumes the staged one);
//! the app reads it itself and a restart goes on to the newest
//! (`relaunch.rs`, "the newest, in one restart").
//!
//! The app never starts a session against one in progress: a quiet check only
//! when Sparkle has none and no update is found, downloading or waiting
//! (`relaunch::may_check_quietly`), and "지금 확인" with an update waiting
//! installs it (Sparkle's own block, kept from `willInstallUpdateOnQuit`;
//! Sparkle 2.3+ lets it be called again if a quit is cancelled) instead of
//! starting a check.
//!
//! Run any other way (`cargo run`, a build without a signing key) there is no
//! framework next to the binary, so there is nothing to start and the menu item
//! is greyed out.

use crate::relaunch::Step;
use equipe_core::t;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::os::unix::ffi::OsStrExt as _;

use block2::{Block, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{AllocAnyThread as _, define_class, msg_send};

thread_local! {
    /// Sparkle's "install and relaunch now", once an update is downloaded.
    /// Sparkle calls its delegate on the main thread, and so does the app.
    static INSTALL: RefCell<Option<RcBlock<dyn Fn()>>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this has no Drop.
    #[unsafe(super(NSObject))]
    #[name = "EquipeUpdaterDelegate"]
    struct Delegate;

    impl Delegate {
        // SAFETY (each method below): SPUUpdaterDelegate's, as Sparkle 2.10's
        // SPUUpdaterDelegate.h declares it; the objects are Sparkle's, valid
        // during the call. Sparkle calls its delegate on the main thread.

        #[unsafe(method(updater:didFindValidUpdate:))]
        fn did_find(&self, _updater: &AnyObject, item: &AnyObject) {
            crate::relaunch::step(Step::Found(version_or_new(item)));
        }

        #[unsafe(method(updaterDidNotFindUpdate:))]
        fn did_not_find(&self, _updater: &AnyObject) {
            crate::relaunch::step(Step::NoUpdate);
        }

        #[unsafe(method(updater:willDownloadUpdate:withRequest:))]
        fn will_download(&self, _updater: &AnyObject, item: &AnyObject, _request: &AnyObject) {
            crate::relaunch::step(Step::Downloading(version_or_new(item)));
        }

        #[unsafe(method(updater:didDownloadUpdate:))]
        fn did_download(&self, _updater: &AnyObject, item: &AnyObject) {
            crate::relaunch::log(&format!("downloaded {}", version_or_new(item)));
        }

        #[unsafe(method(updater:failedToDownloadUpdate:error:))]
        fn failed_download(&self, _updater: &AnyObject, item: &AnyObject, error: &AnyObject) {
            crate::relaunch::log(&format!("failed to download {}: {}", version_or_new(item), describe(error)));
            crate::relaunch::step(Step::Failed { code: code_of(error) });
        }

        #[unsafe(method(userDidCancelDownload:))]
        fn cancelled_download(&self, _updater: &AnyObject) {
            crate::relaunch::step(Step::Cancelled);
        }

        #[unsafe(method(updater:willInstallUpdate:))]
        fn will_install(&self, _updater: &AnyObject, item: &AnyObject) {
            crate::relaunch::log(&format!("installing {}", version_or_new(item)));
        }

        #[unsafe(method(updater:didAbortWithError:))]
        fn did_abort(&self, _updater: &AnyObject, error: &AnyObject) {
            crate::relaunch::log(&format!("aborted: {}", describe(error)));
            let code = code_of(error);
            forget_install_on(code);
            crate::relaunch::step(if code == NO_UPDATE { Step::NoUpdate } else { Step::Failed { code } });
        }

        #[unsafe(method(updater:didFinishUpdateCycleForUpdateCheck:error:))]
        fn did_finish_cycle(&self, _updater: &AnyObject, check: isize, error: *mut AnyObject) {
            let kind = match check {
                0 => "check (asked)",
                1 => "background check",
                _ => "information check",
            };
            // SAFETY: nil, or Sparkle's NSError, valid during the call.
            match unsafe { error.as_ref() } {
                None => crate::relaunch::log(&format!("{kind} finished")),
                Some(error) => {
                    crate::relaunch::log(&format!("{kind} finished with an error: {}", describe(error)));
                    let code = code_of(error);
                    forget_install_on(code);
                    if code != NO_UPDATE {
                        crate::relaunch::step(Step::Failed { code });
                    }
                }
            }
        }

        /// An update downloaded in the background, to go in when the app quits:
        /// keep Sparkle's way to put it in now, and tell the app. YES: no other
        /// update cycle runs meanwhile — this one waits for the person.
        // SAFETY: SPUUpdaterDelegate's
        // `- (BOOL)updater:(SPUUpdater *)updater willInstallUpdateOnQuit:(SUAppcastItem *)item
        //    immediateInstallationBlock:(void (^)(void))immediateInstallHandler`.
        #[unsafe(method(updater:willInstallUpdateOnQuit:immediateInstallationBlock:))]
        fn will_install_on_quit(&self, _updater: &AnyObject, item: &AnyObject, install: &Block<dyn Fn()>) -> Bool {
            INSTALL.with(|slot| *slot.borrow_mut() = Some(install.copy()));
            crate::relaunch::step(Step::Ready(version_or_new(item)));
            Bool::YES
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

/// The install itself failed (SUErrors' 4000s): the block Sparkle handed us
/// belongs to a session that is over — never call it again.
fn forget_install_on(code: i64) {
    if (4000..5000).contains(&code) && code != 4007 && code != 4008 {
        INSTALL.with(|slot| slot.borrow_mut().take());
    }
}

/// SUNoUpdateError: a check found nothing newer — not a failure.
const NO_UPDATE: i64 = 1001;

fn version_or_new(item: &AnyObject) -> String {
    version_of(item).unwrap_or_else(|| t!("update.new_version").to_owned())
}

/// An error's code.
fn code_of(error: &AnyObject) -> i64 {
    // SAFETY: NSError's `code` (an NSInteger).
    let code: isize = unsafe { msg_send![error, code] };
    code as i64
}

/// An error whole, for the log: domain, code, what it says, its userInfo, and
/// the errors under it.
fn describe(error: &AnyObject) -> String {
    let mut out = String::new();
    let mut error: *const AnyObject = error;
    for depth in 0..6 {
        // SAFETY: an NSError (Sparkle's, or one under it), valid during the
        // call; `domain`, `code`, `localizedDescription`, `userInfo` and
        // `description` as Foundation documents them.
        let Some(this) = (unsafe { error.as_ref() }) else { break };
        let (domain, description, info) = unsafe {
            let domain: *mut AnyObject = msg_send![this, domain];
            let description: *mut AnyObject = msg_send![this, localizedDescription];
            let info: *mut AnyObject = msg_send![this, userInfo];
            let info_text: *mut AnyObject = if info.is_null() { std::ptr::null_mut() } else { msg_send![info, description] };
            (text_of(domain), text_of(description), text_of(info_text))
        };
        if depth > 0 {
            out.push_str(" <- under it: ");
        }
        out.push_str(&format!("[{} {}] {} userInfo={}", domain.unwrap_or_default(), code_of(this), description.unwrap_or_default(), info.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")));
        // SAFETY: NSError's userInfo dictionary and its NSUnderlyingErrorKey.
        error = unsafe {
            let info: *mut AnyObject = msg_send![this, userInfo];
            if info.is_null() {
                break;
            }
            let key = objc2_foundation::NSString::from_str("NSUnderlyingError");
            let under: *mut AnyObject = msg_send![info, objectForKey: &*key];
            under
        };
    }
    out
}

/// An NSString's text (nil: none).
fn text_of(text: *mut AnyObject) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: NSString's `UTF8String`, valid while the string is.
    unsafe {
        let utf8: *const c_char = msg_send![text, UTF8String];
        (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
    }
}

/// The version an appcast item names, as people read it.
fn version_of(item: &AnyObject) -> Option<String> {
    // SAFETY: SUAppcastItem's `displayVersionString` (an NSString), and
    // NSString's `UTF8String`, valid while the string is.
    unsafe {
        let text: *mut AnyObject = msg_send![item, displayVersionString];
        if text.is_null() {
            return None;
        }
        let utf8: *const c_char = msg_send![text, UTF8String];
        (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
    }
}

/// Put the downloaded update in now and start the app again — Sparkle quits,
/// installs and relaunches. False when there is none waiting.
pub fn install_now() -> bool {
    let Some(install) = INSTALL.with(|slot| slot.borrow_mut().take()) else {
        return false;
    };
    install.call(());
    true
}

/// Sparkle's standard updater: its own windows, its own schedule.
pub struct Updater {
    controller: *mut AnyObject,
    /// Sparkle holds its delegate weakly: kept here, for the app's life.
    _delegate: Retained<Delegate>,
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
        let delegate = Delegate::new();
        let updater_delegate: *const AnyObject = (&*delegate as *const Delegate).cast();
        // SAFETY: Sparkle 2's documented initialiser; the controller is kept
        // for the life of the app (never released), and its delegate with it.
        let controller: *mut AnyObject = unsafe {
            let allocated: *mut AnyObject = msg_send![class, alloc];
            msg_send![allocated, initWithStartingUpdater: Bool::YES, updaterDelegate: updater_delegate, userDriverDelegate: none]
        };
        if controller.is_null() {
            crate::relaunch::log("Sparkle didn't start");
            return None;
        }
        // Where it runs decides whether it can replace itself: say so at once.
        let misplaced = crate::relaunch::misplaced();
        crate::relaunch::log(&format!("started from {} ({})", crate::relaunch::bundle().map(|b| b.display().to_string()).unwrap_or_default(), misplaced.map_or("can update itself".into(), |m| format!("{m:?}"))));
        // (A folder the person can't write to is Sparkle's to ask about: it
        // asks for an administrator's password when it installs.)
        match misplaced {
            Some(crate::relaunch::Misplaced::DiskImage) => crate::relaunch::step(Step::Failed { code: 1003 }),
            Some(crate::relaunch::Misplaced::Translocated) => crate::relaunch::step(Step::Failed { code: 1005 }),
            _ => {}
        }
        Some(Self { controller, _delegate: delegate })
    }

    /// "지금 확인": an update waiting goes in now (never a second session
    /// against it); else look, saying so in Sparkle's window either way —
    /// unless Sparkle is downloading on its own, which it then finishes.
    pub fn check(&self) {
        if install_now() {
            crate::relaunch::log("check asked with an update waiting: installing it");
            return;
        }
        let updater = self.updater();
        if !updater.is_null() {
            // SAFETY: SPUUpdater's `canCheckForUpdates`, on the main thread.
            let can: Bool = unsafe { msg_send![updater, canCheckForUpdates] };
            if !can.as_bool() {
                crate::relaunch::log("check asked while Sparkle works in the background: left to it");
                return;
            }
        }
        crate::relaunch::log("check asked");
        let none: *mut AnyObject = std::ptr::null_mut();
        // SAFETY: an IBAction on the controller started above, on the main thread.
        unsafe {
            let _: () = msg_send![self.controller, checkForUpdates: none];
        }
    }

    /// Look for an update without a window, if updates are automatic and the
    /// last look is older than `stale` — Sparkle on its own skips the app's
    /// first launch and then waits a whole interval, so a release could sit
    /// unseen for an hour or more.
    pub fn check_quietly(&self, stale: std::time::Duration) {
        let updater = self.updater();
        if updater.is_null() || !self.automatic() {
            return;
        }
        // SAFETY: SPUUpdater's `canCheckForUpdates`, `sessionInProgress`,
        // `lastUpdateCheckDate` (an NSDate or nil) and
        // `checkForUpdatesInBackground`, on the main thread.
        unsafe {
            let can: Bool = msg_send![updater, canCheckForUpdates];
            let busy: Bool = msg_send![updater, sessionInProgress];
            if !can.as_bool() || !crate::relaunch::may_check_quietly(&crate::relaunch::current(), busy.as_bool()) {
                return;
            }
            let last: *mut AnyObject = msg_send![updater, lastUpdateCheckDate];
            if !last.is_null() {
                let since: f64 = msg_send![last, timeIntervalSinceNow];
                if -since < stale.as_secs_f64() {
                    return;
                }
            }
            let _: () = msg_send![updater, checkForUpdatesInBackground];
        }
    }

    /// Sparkle's own updater, under the controller.
    fn updater(&self) -> *mut AnyObject {
        // SAFETY: `-[SPUStandardUpdaterController updater]`, on the main thread.
        unsafe { msg_send![self.controller, updater] }
    }

    /// Whether updates are looked for and downloaded on their own. Sparkle
    /// keeps the answer in the app's defaults.
    pub fn automatic(&self) -> bool {
        let updater = self.updater();
        if updater.is_null() {
            return false;
        }
        // SAFETY: SPUUpdater's `automaticallyChecksForUpdates` and
        // `automaticallyDownloadsUpdates` (BOOL properties).
        unsafe {
            let checks: Bool = msg_send![updater, automaticallyChecksForUpdates];
            let downloads: Bool = msg_send![updater, automaticallyDownloadsUpdates];
            checks.as_bool() && downloads.as_bool()
        }
    }

    pub fn set_automatic(&self, on: bool) {
        let updater = self.updater();
        if updater.is_null() {
            return;
        }
        // SAFETY: the setters of the same properties.
        unsafe {
            let _: () = msg_send![updater, setAutomaticallyChecksForUpdates: Bool::new(on)];
            let _: () = msg_send![updater, setAutomaticallyDownloadsUpdates: Bool::new(on)];
        }
    }
}

/// The app's updater, for the app menu and settings; `None` when the app
/// wasn't packaged with Sparkle.
pub struct Updates(pub Option<std::rc::Rc<Updater>>);

impl gpui_kit::Global for Updates {}
