//! macOS's notification centre (UNUserNotificationCenter), from the app
//! bundle: the only way a Mac app's notices reach the person on today's macOS.
//! NSUserNotificationCenter, which this replaced, delivers nothing for a
//! signed app bundle on recent releases.
//!
//! The centre needs the bundle: run as a bare binary (`cargo run`) it would
//! abort the process ("bundleProxyForCurrentProcess is nil"), so a dev build
//! says once that it shows none, and goes on. The Dock's number needs no
//! bundle and works either way.
//!
//! The delegate is called on a thread of the system's: it touches nothing of
//! the window — clicks go through a channel, and whether the window is in
//! sight is an atomic the window keeps.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use block2::RcBlock;
use equipe_core::t;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, ProtocolObject};
use objc2::{AnyThread as _, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tokio::sync::mpsc::UnboundedSender;

use super::{ACTION_OPEN, ACTION_RELAUNCH, CATEGORY_OPEN, CATEGORY_UPDATE, Go, Permission, Toast};

/// The window is in front of the person: a notice arriving now (a race with
/// the window coming forward) isn't shown as a banner.
static IN_SIGHT: AtomicBool = AtomicBool::new(true);
/// Whether there is a centre: the app runs from its bundle.
static CENTRE: OnceLock<bool> = OnceLock::new();
/// Say each notice macOS took, and each click (`--notify-test`).
pub static VERBOSE: AtomicBool = AtomicBool::new(false);
/// Makes each notice's identifier its own.
static SENT: AtomicU64 = AtomicU64::new(0);
/// Where clicks go, once the window follows them; until then, kept.
static CLICKS: Mutex<(Option<UnboundedSender<Go>>, Vec<Go>)> = Mutex::new((None, Vec::new()));

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this has no Drop.
    #[unsafe(super(NSObject))]
    #[name = "EquipeNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// A notice (or one of its buttons) was clicked — maybe one that
        /// started the app: say where it goes.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _centre: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
            let id = response.notification().request().identifier().to_string();
            let action = response.actionIdentifier().to_string();
            if VERBOSE.load(Ordering::Relaxed) {
                eprintln!("equipe: clicked {id} ({action})");
            }
            if let Some(go) = super::clicked(&id, &action) {
                deliver(go);
            }
            done.call(());
        }

        /// A notice arrives while equipe is the app in front. The window posts
        /// only while it is out of sight (minimised, hidden, on another
        /// Space): show it then; in sight, the window says it already.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _centre: &UNUserNotificationCenter, notification: &UNNotification, done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
            // A browser waiting for the person's hands is shown in sight too:
            // the window posts it then only when that page isn't on screen.
            let hands = notification.request().identifier().to_string().starts_with("browser:");
            let options = if IN_SIGHT.load(Ordering::Relaxed) && !hands { UNNotificationPresentationOptions::empty() } else { UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List | UNNotificationPresentationOptions::Sound };
            done.call((options,));
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

fn deliver(go: Go) {
    let Ok(mut clicks) = CLICKS.lock() else { return };
    let sent = clicks.0.as_ref().is_some_and(|to| to.send(go).is_ok());
    if !sent {
        clicks.1.push(go);
    }
}

pub fn follow_clicks(to: UnboundedSender<Go>) {
    let Ok(mut clicks) = CLICKS.lock() else { return };
    for go in std::mem::take(&mut clicks.1) {
        let _ = to.send(go);
    }
    clicks.0 = Some(to);
}

pub fn set_in_sight(in_sight: bool) {
    IN_SIGHT.store(in_sight, Ordering::Relaxed);
}

/// The app's bundle identifier, when it runs from its bundle.
pub fn bundle_id() -> Option<String> {
    NSBundle::mainBundle().bundleIdentifier().map(|id| id.to_string())
}

/// The centre, when the app runs from its bundle.
fn centre() -> Option<Retained<UNUserNotificationCenter>> {
    CENTRE.get().copied().unwrap_or(false).then(UNUserNotificationCenter::currentNotificationCenter)
}

pub fn start() {
    let bundled = *CENTRE.get_or_init(|| {
        // A bundle identifier only comes with a real `.app` (and only there
        // does the centre not abort the process).
        let bundled = bundle_id().is_some() && NSBundle::mainBundle().bundlePath().to_string().ends_with(".app");
        if !bundled {
            eprintln!("equipe: not running from equipe.app (a dev build?) — macOS notifications are off; the window and the Dock still say what waits");
        }
        bundled
    });
    if !bundled {
        super::set_permission(Permission::NoBundle);
        return;
    }
    let Some(centre) = centre() else { return };
    // The centre holds its delegate weakly: this one lives as long as the app.
    let delegate = Delegate::new();
    centre.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    std::mem::forget(delegate);
    categories(&centre);
    refresh();
}

/// The buttons a notice carries, in the language in use.
fn categories(centre: &UNUserNotificationCenter) {
    let category = |id: &str, action: &str, title: &str| {
        let button = UNNotificationAction::actionWithIdentifier_title_options(&NSString::from_str(action), &NSString::from_str(title), UNNotificationActionOptions::Foreground);
        UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(&NSString::from_str(id), &NSArray::from_retained_slice(&[button]), &NSArray::new(), UNNotificationCategoryOptions::empty())
    };
    let all = [category(CATEGORY_OPEN, ACTION_OPEN, t!("notify.action_open")), category(CATEGORY_UPDATE, ACTION_RELAUNCH, t!("notify.action_relaunch"))];
    centre.setNotificationCategories(&NSSet::from_retained_slice(&all));
}

pub fn refresh() {
    let Some(centre) = centre() else { return };
    let done = RcBlock::new(|settings: std::ptr::NonNull<UNNotificationSettings>| {
        // SAFETY: the settings object macOS hands the block, valid during the call.
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        super::set_permission(Permission::from_status(status.0));
    });
    centre.getNotificationSettingsWithCompletionHandler(&done);
}

pub fn ask() {
    let Some(centre) = centre() else { return };
    let done = RcBlock::new(|granted: Bool, error: *mut NSError| {
        // SAFETY: when not null, an NSError valid during the call.
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("equipe: macOS refused to ask about notifications: {}", error.localizedDescription());
        } else if !granted.as_bool() {
            eprintln!("equipe: notifications not allowed");
        }
        refresh();
    });
    centre.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge, &done);
}

pub fn send(toast: &Toast, sound: bool) {
    let Some(centre) = centre() else { return };
    // The buttons, in the language in use now.
    categories(&centre);
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&toast.title));
    content.setBody(&NSString::from_str(&toast.body));
    content.setThreadIdentifier(&NSString::from_str(&toast.thread));
    content.setCategoryIdentifier(&NSString::from_str(toast.category()));
    if sound {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let id = toast.request_id(SENT.fetch_add(1, Ordering::Relaxed) + now_ms());
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None);
    let done = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: when not null, an NSError valid during the call.
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("equipe: macOS didn't take a notification: {}", error.localizedDescription());
        } else if VERBOSE.load(Ordering::Relaxed) {
            eprintln!("equipe: macOS took notification {id}");
        }
    });
    centre.addNotificationRequest_withCompletionHandler(&request, Some(&done));
}

/// Unique across launches, so a notice from before a restart keeps its own.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The Dock tile's badge: `[[NSApp dockTile] setBadgeLabel:]`. Main thread.
pub fn set_badge(count: Option<usize>) {
    let Some(class) = AnyClass::get(c"NSApplication") else { return };
    let label = count.map(|n| NSString::from_str(&n.to_string()));
    // SAFETY: NSApplication's shared instance and its dock tile, both
    // documented; a nil label takes the badge off.
    unsafe {
        let app: *mut AnyObject = msg_send![class, sharedApplication];
        if app.is_null() {
            return;
        }
        let tile: *mut AnyObject = msg_send![app, dockTile];
        if tile.is_null() {
            return;
        }
        let _: () = msg_send![tile, setBadgeLabel: label.as_deref()];
    }
}
