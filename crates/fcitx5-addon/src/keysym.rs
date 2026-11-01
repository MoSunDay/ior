//! X keysym name table + paging-key resolution for the C++ shell.
//!
//! `[engine] prev_page_keys` / `next_page_keys` carry keysym **names**
//! (rime `key_binder` style, e.g. `"Page_Up"`, `"minus"`); the shell
//! compares raw 32-bit keysyms. This module bridges the two: a small
//! hand-written name → keysym table covering the keys that make sense as
//! paging keys, plus resolution of an [`EngineConfig`] into concrete
//! prev/next keysym sets. Names outside the table are dropped with a
//! warning, matching the addon's other config-warning behavior.

use engine_core::config::EngineConfig;

/// Resolved paging keysyms: what the shell compares key events against.
pub struct PageKeys {
    /// Keysyms that page backwards (rime `when: paging`).
    pub prev: Vec<u32>,
    /// Keysyms that page forwards (rime `when: has_menu` paging).
    pub next: Vec<u32>,
}

/// X keysym values for the names below (keys/defs `/usr/include/X11/keysymdef.h`).
const XK_PAGE_UP: u32 = 0xFF55;
const XK_PAGE_DOWN: u32 = 0xFF56;
const XK_KP_PAGE_UP: u32 = 0xFF9A;
const XK_KP_PAGE_DOWN: u32 = 0xFF9B;
const XK_UP: u32 = 0xFF52;
const XK_DOWN: u32 = 0xFF54;
const XK_LEFT: u32 = 0xFF51;
const XK_RIGHT: u32 = 0xFF53;
const XK_MINUS: u32 = 0x2D;
const XK_EQUAL: u32 = 0x3D;
const XK_COMMA: u32 = 0x2C;
const XK_PERIOD: u32 = 0x2E;
const XK_BRACKETLEFT: u32 = 0x5B;
const XK_BRACKETRIGHT: u32 = 0x5D;
const XK_SEMICOLON: u32 = 0x3B;
const XK_APOSTROPHE: u32 = 0x27;
const XK_SLASH: u32 = 0x2F;
const XK_UNDERSCORE: u32 = 0x5F;

/// Looks a keysym up by its X name (`xev` / keysymdef.h spelling).
pub fn keysym(name: &str) -> Option<u32> {
    match name {
        "Page_Up" => Some(XK_PAGE_UP),
        "Page_Down" => Some(XK_PAGE_DOWN),
        "KP_Page_Up" => Some(XK_KP_PAGE_UP),
        "KP_Page_Down" => Some(XK_KP_PAGE_DOWN),
        "Up" => Some(XK_UP),
        "Down" => Some(XK_DOWN),
        "Left" => Some(XK_LEFT),
        "Right" => Some(XK_RIGHT),
        "minus" => Some(XK_MINUS),
        "equal" => Some(XK_EQUAL),
        "comma" => Some(XK_COMMA),
        "period" => Some(XK_PERIOD),
        "bracketleft" => Some(XK_BRACKETLEFT),
        "bracketright" => Some(XK_BRACKETRIGHT),
        "semicolon" => Some(XK_SEMICOLON),
        "apostrophe" => Some(XK_APOSTROPHE),
        "slash" => Some(XK_SLASH),
        "underscore" => Some(XK_UNDERSCORE),
        _ => None,
    }
}

/// Resolves a config's paging-key names into keysyms, dropping unknown
/// names (warning on stderr) and deduplicating while preserving order.
pub fn resolve(config: &EngineConfig) -> PageKeys {
    PageKeys {
        prev: resolve_list(&config.prev_page_keys),
        next: resolve_list(&config.next_page_keys),
    }
}

/// -1 = prev-page key, 1 = next-page key, 0 = neither (prev wins when a
/// keysym is configured for both directions).
pub fn direction(keys: &PageKeys, keysym: u32) -> i32 {
    if keys.prev.contains(&keysym) {
        -1
    } else if keys.next.contains(&keysym) {
        1
    } else {
        0
    }
}

fn resolve_list(names: &[String]) -> Vec<u32> {
    let mut out = Vec::new();
    for name in names {
        match keysym(name) {
            Some(sym) if !out.contains(&sym) => out.push(sym),
            Some(_) => {}
            None => eprintln!("ior: unknown page key '{name}' ignored"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_lookups() {
        assert_eq!(keysym("Page_Up"), Some(0xFF55));
        assert_eq!(keysym("Page_Down"), Some(0xFF56));
        assert_eq!(keysym("KP_Page_Up"), Some(0xFF9A));
        assert_eq!(keysym("KP_Page_Down"), Some(0xFF9B));
        assert_eq!(keysym("Up"), Some(0xFF52));
        assert_eq!(keysym("Down"), Some(0xFF54));
        assert_eq!(keysym("Left"), Some(0xFF51));
        assert_eq!(keysym("Right"), Some(0xFF53));
        assert_eq!(keysym("minus"), Some(0x2D));
        assert_eq!(keysym("equal"), Some(0x3D));
        assert_eq!(keysym("comma"), Some(0x2C));
        assert_eq!(keysym("period"), Some(0x2E));
        assert_eq!(keysym("bracketleft"), Some(0x5B));
        assert_eq!(keysym("bracketright"), Some(0x5D));
        assert_eq!(keysym("semicolon"), Some(0x3B));
        assert_eq!(keysym("apostrophe"), Some(0x27));
        assert_eq!(keysym("slash"), Some(0x2F));
        assert_eq!(keysym("underscore"), Some(0x5F));
        assert_eq!(keysym("PageUp"), None);
        assert_eq!(keysym(""), None);
    }

    #[test]
    fn default_config_resolves_to_pageup_minus() {
        let keys = resolve(&EngineConfig::default());
        assert_eq!(keys.prev, [0xFF55, 0x2D]);
        assert_eq!(keys.next, [0xFF56, 0x3D]);
    }

    #[test]
    fn unknown_names_are_dropped_and_duplicates_removed() {
        let config = EngineConfig {
            prev_page_keys: vec![
                "minus".into(),
                "No_Such_Key".into(),
                "minus".into(),
                "Page_Up".into(),
            ],
            next_page_keys: vec![],
            ..EngineConfig::default()
        };
        let keys = resolve(&config);
        assert_eq!(keys.prev, [0x2D, 0xFF55]);
        assert!(keys.next.is_empty(), "empty array disables the direction");
    }

    #[test]
    fn direction_reports_prev_first() {
        let config = EngineConfig {
            prev_page_keys: vec!["comma".into()],
            next_page_keys: vec!["period".into()],
            ..EngineConfig::default()
        };
        let keys = resolve(&config);
        assert_eq!(direction(&keys, 0x2C), -1);
        assert_eq!(direction(&keys, 0x2E), 1);
        assert_eq!(direction(&keys, 0xFF55), 0);
        // A keysym configured for both directions resolves to prev.
        let both = EngineConfig {
            prev_page_keys: vec!["comma".into()],
            next_page_keys: vec!["comma".into()],
            ..EngineConfig::default()
        };
        assert_eq!(direction(&resolve(&both), 0x2C), -1);
    }
}
