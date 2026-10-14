//! What macOS says about the person's session: the last input's age and
//! whether the screen is locked. Neither needs a permission; both need the
//! caller to be in the GUI (Aqua) session, which the daemon is once installed.

use std::ffi::{c_char, c_void};

use super::Screen;

type CFTypeRef = *const c_void;

const ANY_INPUT: u32 = u32::MAX;
const COMBINED_SESSION_STATE: i32 = 0;
const UTF8: u32 = 0x0800_0100;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(source: i32, event_type: u32) -> f64;
    fn CGSessionCopyCurrentDictionary() -> CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(alloc: CFTypeRef, text: *const c_char, encoding: u32) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
    fn CFNumberGetValue(number: CFTypeRef, kind: i64, out: *mut c_void) -> u8;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFRelease(cf: CFTypeRef);
}

/// Seconds since the last mouse or keyboard event of any source.
pub fn idle() -> Option<f64> {
    // SAFETY: a plain query with constant arguments.
    let secs = unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT) };
    secs.is_finite().then_some(secs.max(0.0))
}

/// Whether there is a GUI session, and whether its screen is locked.
pub fn screen() -> Screen {
    // SAFETY: the dictionary is ours (Copy rule) and released below; the keys
    // are created and released here; values are borrowed from the dictionary.
    unsafe {
        let dict = CGSessionCopyCurrentDictionary();
        if dict.is_null() {
            return Screen::NoSession;
        }
        let flag = |key: &std::ffi::CStr| -> Option<bool> {
            let key = CFStringCreateWithCString(std::ptr::null(), key.as_ptr(), UTF8);
            let value = CFDictionaryGetValue(dict, key);
            CFRelease(key);
            if value.is_null() {
                return None;
            }
            if CFGetTypeID(value) == CFBooleanGetTypeID() {
                return Some(CFBooleanGetValue(value) != 0);
            }
            if CFGetTypeID(value) == CFNumberGetTypeID() {
                let mut n: i64 = 0;
                // kCFNumberSInt64Type
                if CFNumberGetValue(value, 4, (&mut n as *mut i64).cast()) != 0 {
                    return Some(n != 0);
                }
            }
            None
        };
        let locked = flag(c"CGSSessionScreenIsLocked").unwrap_or(false);
        let on_console = flag(c"kCGSSessionOnConsoleKey").unwrap_or(true);
        let login_done = flag(c"kCGSessionLoginDoneKey").unwrap_or(true);
        CFRelease(dict);
        if !login_done || !on_console {
            Screen::NoSession
        } else if locked {
            Screen::Locked
        } else {
            Screen::Open
        }
    }
}
