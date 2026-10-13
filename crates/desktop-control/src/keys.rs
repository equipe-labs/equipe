//! Shortcuts as members write them — `cmd+s`, `shift+tab`, `return` — as the
//! key codes and modifier flags macOS posts.

/// One key press, with its modifiers held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    /// The virtual key code (ANSI layout).
    pub code: u16,
    /// `CGEventFlags`.
    pub flags: u64,
}

pub const SHIFT: u64 = 1 << 17;
pub const CONTROL: u64 = 1 << 18;
pub const OPTION: u64 = 1 << 19;
pub const COMMAND: u64 = 1 << 20;
pub const FUNCTION: u64 = 1 << 23;

/// The key codes for a newline and a tab in typed text.
pub const RETURN: u16 = 36;
pub const TAB: u16 = 48;

fn modifier(name: &str) -> Option<u64> {
    Some(match name {
        "cmd" | "command" | "⌘" | "meta" | "super" => COMMAND,
        "shift" | "⇧" => SHIFT,
        "alt" | "option" | "opt" | "⌥" => OPTION,
        "ctrl" | "control" | "⌃" => CONTROL,
        "fn" => FUNCTION,
        _ => return None,
    })
}

fn key(name: &str) -> Option<u16> {
    const LETTERS: &[(char, u16)] = &[
        ('a', 0), ('s', 1), ('d', 2), ('f', 3), ('h', 4), ('g', 5), ('z', 6), ('x', 7), ('c', 8), ('v', 9),
        ('b', 11), ('q', 12), ('w', 13), ('e', 14), ('r', 15), ('y', 16), ('t', 17), ('1', 18), ('2', 19),
        ('3', 20), ('4', 21), ('6', 22), ('5', 23), ('=', 24), ('9', 25), ('7', 26), ('-', 27), ('8', 28),
        ('0', 29), (']', 30), ('o', 31), ('u', 32), ('[', 33), ('i', 34), ('p', 35), ('l', 37), ('j', 38),
        ('\'', 39), ('k', 40), (';', 41), ('\\', 42), (',', 43), ('/', 44), ('n', 45), ('m', 46), ('.', 47),
        ('`', 50),
    ];
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return LETTERS.iter().find(|(l, _)| *l == c).map(|(_, code)| *code);
    }
    Some(match name {
        "return" | "enter" | "⏎" => RETURN,
        "tab" => TAB,
        "space" => 49,
        "delete" | "backspace" | "⌫" => 51,
        "escape" | "esc" => 53,
        "forwarddelete" | "del" => 117,
        "home" => 115,
        "end" => 119,
        "pageup" => 116,
        "pagedown" => 121,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        _ => return None,
    })
}

/// One shortcut: modifiers and a key joined by `+` (`cmd+shift+s`).
pub fn parse(combo: &str) -> Result<Combo, String> {
    let lower = combo.trim().to_lowercase();
    // "cmd++" is ⌘ and the plus key, which is shift and "=".
    let (mods, last, plus) = match lower.strip_suffix("++") {
        Some(rest) => (rest, "=", SHIFT),
        None => match lower.rsplit_once('+') {
            Some((mods, last)) => (mods, last, 0),
            None => ("", lower.as_str(), 0),
        },
    };
    let code = key(last.trim()).ok_or_else(|| format!("I don't know the key {last:?} in {combo:?}."))?;
    let mut flags = plus;
    for part in mods.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        flags |= modifier(part).ok_or_else(|| format!("I don't know the modifier {part:?} in {combo:?}."))?;
    }
    Ok(Combo { code, flags })
}

/// Shortcuts pressed one after another, separated by spaces (`cmd+a delete`).
pub fn parse_all(keys: &str) -> Result<Vec<Combo>, String> {
    let combos: Vec<Combo> = keys.split_whitespace().map(parse).collect::<Result<_, _>>()?;
    if combos.is_empty() {
        return Err("Say which keys, e.g. \"cmd+s\" or \"return\".".into());
    }
    Ok(combos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_as_people_write_them() {
        assert_eq!(parse("cmd+s"), Ok(Combo { code: 1, flags: COMMAND }));
        assert_eq!(parse("Cmd+Shift+T"), Ok(Combo { code: 17, flags: COMMAND | SHIFT }));
        assert_eq!(parse("return"), Ok(Combo { code: RETURN, flags: 0 }));
        assert_eq!(parse("ctrl+option+left"), Ok(Combo { code: 123, flags: CONTROL | OPTION }));
        assert_eq!(parse("⌘+v"), Ok(Combo { code: 9, flags: COMMAND }));
        assert_eq!(parse("cmd++").map(|c| c.flags), Ok(COMMAND | SHIFT), "⌘ and the plus key");
        assert!(parse("cmd+banana").unwrap_err().contains("banana"));
        assert!(parse("hyper+a").unwrap_err().contains("hyper"));
        assert_eq!(parse_all("cmd+a delete").unwrap().len(), 2);
        assert!(parse_all("  ").is_err());
    }
}
