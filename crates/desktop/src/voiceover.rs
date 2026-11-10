//! Words for VoiceOver to say once, without the screen changing under the
//! reader: a summary beginning, taking long, done or failed (`design/specs/
//! compaction.md` §7). The seconds are never said — they are drawn, not read.

use equipe_core::t;

use crate::state::Say;

/// What a [`Say`] says, and whether it interrupts (a failure does).
pub fn words(say: Say) -> (String, bool) {
    match say {
        Say::Started(_, true) => (t!("compact.title.manual").to_owned(), false),
        Say::Started(_, false) => (t!("compact.title.auto").to_owned(), false),
        Say::Done => (t!("compact.done.say").to_owned(), false),
        Say::Failed => (t!("compact.fail.say").to_owned(), true),
    }
}

/// What it says when a summary has run long.
pub fn long() -> String {
    t!("compact.title.long").to_owned()
}

impl crate::Shell {
    /// Say to VoiceOver what the model has to say, once; and for a summary
    /// that begins, once more if it is still going after a minute.
    pub fn speak(&mut self, cx: &mut gpui_kit::Context<Self>) {
        for said in self.model.take_voice() {
            let (text, urgent) = words(said);
            say(&text, urgent);
            if let Say::Started(session, _) = said {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(crate::convo::COMPACT_LONG).await;
                    let _ = this.update(cx, |shell, _| {
                        if shell.model.compacting_of(session).is_some_and(|ran| ran >= crate::convo::COMPACT_LONG) {
                            say(&long(), false);
                        }
                    });
                })
                .detach();
            }
        }
    }
}

/// Tell VoiceOver, if it is listening (macOS's announcement notification).
#[cfg(target_os = "macos")]
pub fn say(text: &str, urgent: bool) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSDictionary, NSNumber, NSString};

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSAccessibilityAnnouncementRequestedNotification: &'static NSString;
        static NSAccessibilityAnnouncementKey: &'static NSString;
        static NSAccessibilityPriorityKey: &'static NSString;
        fn NSAccessibilityPostNotificationWithUserInfo(element: *mut AnyObject, notification: &NSString, user_info: &NSDictionary<NSString, AnyObject>);
    }
    let Some(app_class) = AnyClass::get(c"NSApplication") else { return };
    // SAFETY: AppKit's `NSAccessibilityPostNotificationWithUserInfo` with the
    // application as the element, on the main thread where the app runs; the
    // keys are AppKit's own constants, the values an NSString and an NSNumber
    // (90: high priority, 50: medium).
    unsafe {
        let app: *mut AnyObject = msg_send![app_class, sharedApplication];
        if app.is_null() {
            return;
        }
        let words = NSString::from_str(text);
        let priority = NSNumber::new_i64(if urgent { 90 } else { 50 });
        let info = NSDictionary::from_slices(&[NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey], &[&*words as &AnyObject, &*priority as &AnyObject]);
        NSAccessibilityPostNotificationWithUserInfo(app, NSAccessibilityAnnouncementRequestedNotification, &info);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn say(_text: &str, _urgent: bool) {}

#[cfg(test)]
mod tests {
    use super::words;
    use crate::state::Say;

    #[test]
    fn a_summary_is_said_when_it_begins_and_ends_never_with_its_seconds() {
        assert_eq!(words(Say::Started(1, false)).0, "대화를 요약하고 있어요");
        assert_eq!(words(Say::Started(1, true)).0, "요약해서 새로 시작하고 있어요");
        assert_eq!(words(Say::Done), ("요약했어요. 이어서 대화할 수 있어요".to_owned(), false));
        assert!(words(Say::Failed).1, "a failure interrupts");
        assert!(!words(Say::Failed).0.chars().any(|c| c.is_ascii_digit()));
    }
}
