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
//! Run any other way (`cargo run`, a build without a signing key) there is no
//! framework next to the binary, so there is nothing to start and the menu item
//! is greyed out.

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
        /// An update downloaded in the background, to go in when the app quits:
        /// keep Sparkle's way to put it in now, and tell the app. YES: no other
        /// update cycle runs meanwhile — this one waits for the person.
        // SAFETY: SPUUpdaterDelegate's
        // `- (BOOL)updater:(SPUUpdater *)updater willInstallUpdateOnQuit:(SUAppcastItem *)item
        //    immediateInstallationBlock:(void (^)(void))immediateInstallHandler`.
        #[unsafe(method(updater:willInstallUpdateOnQuit:immediateInstallationBlock:))]
        fn will_install_on_quit(&self, _updater: &AnyObject, item: &AnyObject, install: &Block<dyn Fn()>) -> Bool {
            INSTALL.with(|slot| *slot.borrow_mut() = Some(install.copy()));
            crate::relaunch::ready(version_of(item).unwrap_or_else(|| t!("update.new_version").to_owned()));
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
        (!controller.is_null()).then_some(Self { controller, _delegate: delegate })
    }

    /// Look for an update now, saying so in Sparkle's window either way.
    pub fn check(&self) {
        let none: *mut AnyObject = std::ptr::null_mut();
        // SAFETY: an IBAction on the controller started above, on the main thread.
        unsafe {
            let _: () = msg_send![self.controller, checkForUpdates: none];
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
