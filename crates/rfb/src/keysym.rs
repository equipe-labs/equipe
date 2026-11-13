//! X keysyms: what an RFB key event names.
//!
//! Text goes as the characters themselves, not as the keys that made them:
//! Latin-1 has keysyms equal to its code points, everything else is
//! `0x0100_0000 + code point` (X's Unicode keysyms). So a Korean syllable
//! composed by the Mac's input source arrives as that syllable — the remote
//! machine needs no Korean keyboard layout and no input method of its own.
//! (X servers map a keysym their keymap lacks onto a spare keycode: TigerVNC's
//! Xvnc and x0vncserver both do.)

pub const BACKSPACE: u32 = 0xff08;
pub const TAB: u32 = 0xff09;
pub const RETURN: u32 = 0xff0d;
pub const ESCAPE: u32 = 0xff1b;
pub const DELETE: u32 = 0xffff;
pub const HOME: u32 = 0xff50;
pub const LEFT: u32 = 0xff51;
pub const UP: u32 = 0xff52;
pub const RIGHT: u32 = 0xff53;
pub const DOWN: u32 = 0xff54;
pub const PAGE_UP: u32 = 0xff55;
pub const PAGE_DOWN: u32 = 0xff56;
pub const END: u32 = 0xff57;
pub const INSERT: u32 = 0xff63;
pub const MENU: u32 = 0xff67;
pub const F1: u32 = 0xffbe;
pub const SHIFT_L: u32 = 0xffe1;
pub const CONTROL_L: u32 = 0xffe3;
pub const META_L: u32 = 0xffe7;
pub const ALT_L: u32 = 0xffe9;
pub const SUPER_L: u32 = 0xffeb;

/// The keysym that types `c`.
pub fn of_char(c: char) -> u32 {
    match c {
        '\r' | '\n' => RETURN,
        '\t' => TAB,
        '\u{8}' => BACKSPACE,
        '\u{1b}' => ESCAPE,
        '\u{7f}' => DELETE,
        ' '..='~' | '\u{a0}'..='\u{ff}' => c as u32,
        _ => 0x0100_0000 + c as u32,
    }
}

/// A named key, as GPUI names it (`enter`, `left`, `f5`…).
pub fn of_key(key: &str) -> Option<u32> {
    Some(match key {
        "enter" => RETURN,
        "backspace" => BACKSPACE,
        "tab" => TAB,
        "escape" => ESCAPE,
        "space" => 0x20,
        "delete" => DELETE,
        "insert" => INSERT,
        "home" => HOME,
        "end" => END,
        "pageup" => PAGE_UP,
        "pagedown" => PAGE_DOWN,
        "left" => LEFT,
        "up" => UP,
        "right" => RIGHT,
        "down" => DOWN,
        "menu" => MENU,
        k if k.len() >= 2 && k.starts_with('f') => {
            let n: u32 = k[1..].parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            F1 + n - 1
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_goes_as_its_characters() {
        assert_eq!(of_char('a'), 0x61);
        assert_eq!(of_char('A'), 0x41);
        assert_eq!(of_char('é'), 0xe9, "Latin-1 is its own keysym");
        assert_eq!(of_char('한'), 0x0100_d55c, "a Hangul syllable, whole");
        assert_eq!(of_char('ㄱ'), 0x0100_3131);
        assert_eq!(of_char('€'), 0x0100_20ac);
        assert_eq!(of_char('\n'), RETURN);
    }

    #[test]
    fn named_keys() {
        assert_eq!(of_key("enter"), Some(RETURN));
        assert_eq!(of_key("f1"), Some(F1));
        assert_eq!(of_key("f12"), Some(0xffc9));
        assert_eq!(of_key("f25"), None);
        assert_eq!(of_key("fn"), None);
        assert_eq!(of_key("a"), None, "letters go by their character");
    }
}
