//! Voice hotkey parsing: `"ctrl+grave"` to an X11 keysym plus a
//! modifier bitmask aligned with fcitx5's `KeyState` bits, so the addon
//! can compare against `KeyEvent::key().states().toInteger()` directly.

/// Modifier bits (fcitx5 `KeyState`: Shift=1<<0, Ctrl=1<<2, Alt=1<<3,
/// Super=1<<6). Lock/Hyper/Mod5 bits are deliberately absent: they are
/// keyboard state, not user intent.
pub const MOD_SHIFT: u32 = 1 << 0;
pub const MOD_CTRL: u32 = 1 << 2;
pub const MOD_ALT: u32 = 1 << 3;
pub const MOD_SUPER: u32 = 1 << 6;
/// Mask of every modifier this module can express.
pub const MOD_MASK: u32 = MOD_SHIFT | MOD_CTRL | MOD_ALT | MOD_SUPER;

/// X11 `XK_grave` (the ` key).
pub const KEY_GRAVE: u32 = 0x0060;
/// X11 `XK_Escape`.
pub const KEY_ESCAPE: u32 = 0xFF1B;
/// X11 `XK_space`.
pub const KEY_SPACE: u32 = 0x0020;
/// X11 `XK_F1`; F1..=F24 run consecutively.
pub const KEY_F1: u32 = 0xFFBE;
/// X11 `XK_F24` (end of the F-key range this module names).
pub const KEY_F24: u32 = KEY_F1 + 23;

/// A parsed hotkey: keysym plus required modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    /// X11 keysym of the main key.
    pub keysym: u32,
    /// Required modifier bits (subset of [`MOD_MASK`]).
    pub mods: u32,
}

/// Parses a hotkey spec: `mod+...+key`, case-insensitive, e.g.
/// `"ctrl+grave"`, `"ctrl+\`"`, `"Ctrl+Shift+F4"`. The plain key must be
/// last; duplicate modifiers and unknown names are rejected with `None`.
pub fn parse_hotkey(text: &str) -> Option<Hotkey> {
    let mut mods = 0u32;
    let mut keysym: Option<u32> = None;
    for part in text.split('+') {
        let part = part.trim();
        if part.is_empty() || keysym.is_some() {
            return None; // leading/trailing/double '+' or key not last
        }
        let lower = part.to_ascii_lowercase();
        let modifier = match lower.as_str() {
            "ctrl" | "control" => Some(MOD_CTRL),
            "shift" => Some(MOD_SHIFT),
            "alt" | "option" => Some(MOD_ALT),
            "super" | "meta" | "win" | "cmd" => Some(MOD_SUPER),
            _ => None,
        };
        match modifier {
            Some(bit) if mods & bit == 0 => mods |= bit,
            Some(_) => return None, // duplicate modifier
            None => keysym = Some(parse_key_name(&lower)?),
        }
    }
    keysym.map(|keysym| Hotkey { keysym, mods })
}

/// Canonical display name: `"ctrl+\`"` for the default, suitable for
/// preedit hints. Inverse of [`parse_hotkey`] on roundtrippable specs.
pub fn hotkey_name(hotkey: Hotkey) -> String {
    let mut name = String::new();
    for (bit, label) in [
        (MOD_CTRL, "ctrl"),
        (MOD_SUPER, "super"),
        (MOD_ALT, "alt"),
        (MOD_SHIFT, "shift"),
    ] {
        if hotkey.mods & bit != 0 {
            name.push_str(label);
            name.push('+');
        }
    }
    name.push_str(&key_name(hotkey.keysym));
    name
}

/// One key name (no modifiers) to its X11 keysym.
fn parse_key_name(name: &str) -> Option<u32> {
    match name {
        "grave" | "backquote" | "`" => Some(KEY_GRAVE),
        "space" => Some(KEY_SPACE),
        "escape" | "esc" => Some(KEY_ESCAPE),
        _ => {
            let bytes = name.as_bytes();
            if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
                // Printable ASCII maps to itself, matching
                // ffi::keysym_to_char.
                Some(u32::from(bytes[0]))
            } else if let Some(rest) = name.strip_prefix('f') {
                let n: u32 = rest.parse().ok()?;
                (1..=24).contains(&n).then(|| KEY_F1 - 1 + n)
            } else {
                None
            }
        }
    }
}

/// Display name for a keysym handled by [`parse_key_name`].
fn key_name(keysym: u32) -> String {
    match keysym {
        KEY_GRAVE => "`".to_string(),
        KEY_SPACE => "space".to_string(),
        KEY_ESCAPE => "esc".to_string(),
        ks @ (0x21..=0x7E) => char::from_u32(ks).unwrap_or('?').to_string(),
        ks @ (KEY_F1..=KEY_F24) => format!("f{}", ks - KEY_F1 + 1),
        ks => format!("0x{ks:04x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_default_hotkey_and_its_backtick_alias() {
        let expected = Hotkey {
            keysym: KEY_GRAVE,
            mods: MOD_CTRL,
        };
        assert_eq!(parse_hotkey("ctrl+grave"), Some(expected));
        assert_eq!(parse_hotkey("ctrl+`"), Some(expected));
        assert_eq!(parse_hotkey("CTRL+GRAVE"), Some(expected));
        assert_eq!(parse_hotkey("Control+backquote"), Some(expected));
    }

    #[test]
    fn parses_modifier_combinations_and_plain_keys() {
        assert_eq!(
            parse_hotkey("ctrl+shift+f4"),
            Some(Hotkey {
                keysym: KEY_F1 + 3,
                mods: MOD_CTRL | MOD_SHIFT
            })
        );
        assert_eq!(
            parse_hotkey("f4"),
            Some(Hotkey {
                keysym: KEY_F1 + 3,
                mods: 0
            })
        );
        assert_eq!(
            parse_hotkey("super+alt+a"),
            Some(Hotkey {
                keysym: 0x61,
                mods: MOD_SUPER | MOD_ALT
            })
        );
    }

    #[test]
    fn rejects_malformed_specs() {
        assert_eq!(parse_hotkey(""), None);
        assert_eq!(parse_hotkey("ctrl+"), None);
        assert_eq!(parse_hotkey("+grave"), None);
        assert_eq!(parse_hotkey("ctrl++grave"), None);
        assert_eq!(parse_hotkey("grave+ctrl"), None); // key must be last
        assert_eq!(parse_hotkey("ctrl+ctrl+grave"), None);
        assert_eq!(parse_hotkey("ctrl+hyper+grave"), None);
        assert_eq!(parse_hotkey("ctrl+notakey"), None);
        assert_eq!(parse_hotkey("ctrl+f25"), None);
    }

    #[test]
    fn names_roundtrip_through_parse() {
        for spec in ["ctrl+grave", "ctrl+`", "ctrl+shift+f4", "alt+1", "super+m"] {
            let hotkey = parse_hotkey(spec).expect(spec);
            let name = hotkey_name(hotkey);
            assert_eq!(parse_hotkey(&name), Some(hotkey), "roundtrip {spec}");
        }
    }
}
