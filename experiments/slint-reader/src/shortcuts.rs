//! Keyboard shortcuts the reader can change in Options.
//!
//! A shortcut is stored as text such as `Ctrl+KeyF` or `F11`: the modifiers in
//! a fixed order, then the physical key's name. An empty string means the
//! action has no shortcut.

use slint::winit_030::winit::keyboard::{KeyCode, ModifiersState};
use std::collections::BTreeMap;

pub struct Action {
    /// Name stored in the profile.
    pub name: &'static str,
    pub english: &'static str,
    pub arabic: &'static str,
    pub default: &'static str,
}

pub const ACTIONS: [Action; 7] = [
    Action {
        name: "search",
        english: "Search the Quran",
        arabic: "البحث في القرآن",
        default: "Ctrl+KeyF",
    },
    Action {
        name: "listen",
        english: "Listening",
        arabic: "الاستماع",
        default: "Ctrl+KeyL",
    },
    Action {
        name: "options",
        english: "Options",
        arabic: "الخيارات",
        default: "Ctrl+Comma",
    },
    Action {
        name: "downloads",
        english: "Downloads",
        arabic: "التنزيلات",
        default: "Ctrl+KeyD",
    },
    Action {
        name: "bookmarks",
        english: "Saved bookmarks",
        arabic: "العلامات المحفوظة",
        default: "Ctrl+KeyB",
    },
    Action {
        name: "playPause",
        english: "Pause or resume recitation",
        arabic: "إيقاف التلاوة مؤقتًا أو متابعتها",
        default: "Ctrl+Space",
    },
    Action {
        name: "fullScreen",
        english: "Full screen",
        arabic: "ملء الشاشة",
        default: "F11",
    },
];

/// The shortcut for each action, from the saved choices. Actions without a
/// saved choice use the earlier preset pickers' choice, then the default.
pub fn resolve(saved: &BTreeMap<String, String>, legacy: [u32; 3]) -> [String; 7] {
    let legacy_preset = |index: usize| -> Option<&'static str> {
        match index {
            0 => ["Ctrl+KeyF", "Ctrl+KeyK", "Ctrl+Slash"]
                .get(legacy[0] as usize)
                .copied(),
            1 => ["Ctrl+KeyL", "Ctrl+KeyP"].get(legacy[1] as usize).copied(),
            2 => ["Ctrl+Comma", "Ctrl+KeyO"].get(legacy[2] as usize).copied(),
            _ => None,
        }
    };
    std::array::from_fn(|i| {
        saved
            .get(ACTIONS[i].name)
            .filter(|value| value.is_empty() || parse(value).is_some())
            .cloned()
            .or_else(|| legacy_preset(i).map(str::to_string))
            .unwrap_or_else(|| ACTIONS[i].default.to_string())
    })
}

/// Keep only known actions with well-formed values.
pub fn sanitize(saved: &mut BTreeMap<String, String>) {
    saved.retain(|name, value| {
        ACTIONS.iter().any(|a| a.name == name) && (value.is_empty() || parse(value).is_some())
    });
}

fn is_modifier(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::ControlLeft
            | KeyCode::ControlRight
            | KeyCode::ShiftLeft
            | KeyCode::ShiftRight
            | KeyCode::AltLeft
            | KeyCode::AltRight
            | KeyCode::SuperLeft
            | KeyCode::SuperRight
    )
}

fn is_function_key(code: KeyCode) -> bool {
    format!("{code:?}")
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
}

/// The text for a key press, if it can be a shortcut: a modifier-only press
/// or a plain key (which would steal typing and page turning) cannot.
pub fn combo(modifiers: ModifiersState, code: KeyCode) -> Option<String> {
    if is_modifier(code) || modifiers.super_key() {
        return None;
    }
    let ctrl = modifiers.control_key();
    let alt = modifiers.alt_key();
    if !ctrl && !alt && !is_function_key(code) {
        return None;
    }
    // Windows reserves these for closing and switching windows.
    if alt && !ctrl && matches!(code, KeyCode::F4 | KeyCode::Tab | KeyCode::Space) {
        return None;
    }
    let mut text = String::new();
    if ctrl {
        text.push_str("Ctrl+");
    }
    if modifiers.shift_key() {
        text.push_str("Shift+");
    }
    if alt {
        text.push_str("Alt+");
    }
    text.push_str(&format!("{code:?}"));
    parse(&text).map(|_| text)
}

/// Checks a stored value: modifiers in order, then a key name of letters and digits.
fn parse(value: &str) -> Option<()> {
    if value.len() > 40 {
        return None;
    }
    let mut parts: Vec<&str> = value.split('+').collect();
    let key = parts.pop()?;
    let order = ["Ctrl", "Shift", "Alt"];
    let mut last = None;
    for part in &parts {
        let position = order.iter().position(|m| m == part)?;
        if last.is_some_and(|l| position <= l) {
            return None;
        }
        last = Some(position);
    }
    let has_ctrl_or_alt = parts.iter().any(|p| *p == "Ctrl" || *p == "Alt");
    let function = key
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    (!key.is_empty()
        && key.chars().all(|c| c.is_ascii_alphanumeric())
        && (has_ctrl_or_alt || function))
        .then_some(())
}

/// How a shortcut is shown, for example `Ctrl+F` or `Ctrl+,`.
pub fn display(value: &str, arabic: bool) -> String {
    if value.is_empty() {
        return if arabic {
            "بلا اختصار"
        } else {
            "None"
        }
        .to_string();
    }
    let mut parts: Vec<String> = value.split('+').map(str::to_string).collect();
    if let Some(key) = parts.last_mut() {
        let shown = match key.as_str() {
            "Comma" => ",".to_string(),
            "Period" => ".".to_string(),
            "Slash" => "/".to_string(),
            "Backslash" => "\\".to_string(),
            "Semicolon" => ";".to_string(),
            "Quote" => "'".to_string(),
            "BracketLeft" => "[".to_string(),
            "BracketRight" => "]".to_string(),
            "Minus" => "-".to_string(),
            "Equal" => "=".to_string(),
            "Backquote" => "`".to_string(),
            "ArrowLeft" => "←".to_string(),
            "ArrowRight" => "→".to_string(),
            "ArrowUp" => "↑".to_string(),
            "ArrowDown" => "↓".to_string(),
            other => other
                .strip_prefix("Key")
                .or_else(|| other.strip_prefix("Digit"))
                .unwrap_or(other)
                .to_string(),
        };
        *key = shown;
    }
    let text = parts.join("+");
    // Keep the key combination left-to-right inside Arabic text.
    if arabic {
        format!("\u{2066}{text}\u{2069}")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_need_a_modifier_or_function_key() {
        assert_eq!(
            combo(ModifiersState::CONTROL, KeyCode::KeyF).as_deref(),
            Some("Ctrl+KeyF")
        );
        assert_eq!(
            combo(
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                KeyCode::Digit2
            )
            .as_deref(),
            Some("Ctrl+Shift+Digit2")
        );
        assert_eq!(
            combo(ModifiersState::empty(), KeyCode::F11).as_deref(),
            Some("F11")
        );
        assert_eq!(combo(ModifiersState::empty(), KeyCode::KeyF), None);
        assert_eq!(combo(ModifiersState::CONTROL, KeyCode::ControlLeft), None);
        assert_eq!(combo(ModifiersState::ALT, KeyCode::F4), None);
    }

    #[test]
    fn display_and_resolution() {
        assert_eq!(display("Ctrl+Comma", false), "Ctrl+,");
        assert_eq!(display("Ctrl+Shift+KeyK", false), "Ctrl+Shift+K");
        assert_eq!(display("", false), "None");
        let mut saved = BTreeMap::new();
        saved.insert("listen".to_string(), "Alt+KeyL".to_string());
        saved.insert("downloads".to_string(), String::new());
        saved.insert("bogus".to_string(), "Ctrl+KeyZ".to_string());
        saved.insert("options".to_string(), "KeyO".to_string());
        sanitize(&mut saved);
        assert!(!saved.contains_key("bogus") && !saved.contains_key("options"));
        let resolved = resolve(&saved, [1, 0, 1]);
        assert_eq!(resolved[0], "Ctrl+KeyK");
        assert_eq!(resolved[1], "Alt+KeyL");
        assert_eq!(resolved[2], "Ctrl+KeyO");
        assert_eq!(resolved[3], "");
        assert_eq!(resolved[6], "F11");
    }
}
