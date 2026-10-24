//! GDK key events to mpv key names, as [`muso_core::input`] expects them.

use gtk::gdk::{Key, ModifierType};
use gtk::glib::translate::IntoGlib;

/// GDK names that mpv spells differently. Every other named key is the GDK
/// name in capitals (`Left` is `LEFT`, `F5` is `F5`, `KP_Enter` is
/// `KP_ENTER`).
const MPV_NAMES: &[(&str, &str)] = &[
    ("space", "SPACE"),
    ("numbersign", "SHARP"),
    ("Escape", "ESC"),
    ("Return", "ENTER"),
    ("ISO_Enter", "ENTER"),
    ("ISO_Left_Tab", "TAB"),
    ("BackSpace", "BS"),
    ("Delete", "DEL"),
    ("Insert", "INS"),
    ("Page_Up", "PGUP"),
    ("Page_Down", "PGDWN"),
    ("KP_Delete", "KP_DEL"),
    ("KP_Decimal", "KP_DEC"),
    ("AudioPlay", "PLAY"),
    ("AudioPause", "PAUSE"),
    ("AudioStop", "STOP"),
    ("AudioNext", "NEXT"),
    ("AudioPrev", "PREV"),
    ("AudioMute", "MUTE"),
    ("AudioRaiseVolume", "VOLUME_UP"),
    ("AudioLowerVolume", "VOLUME_DOWN"),
];

/// The mpv name of a key press, or `None` for keys that are only modifiers.
pub fn name(key: Key, state: ModifierType) -> Option<String> {
    let gdk_name = key.name()?;
    let alias = MPV_NAMES
        .iter()
        .find(|(gdk, _)| *gdk == gdk_name.as_str())
        .map(|(_, mpv)| *mpv);
    let keypad = gdk_name.starts_with("KP_");
    let char = key.to_unicode().filter(|c| !c.is_control() && !keypad);
    let (base, printable) = match (alias, char) {
        (Some(mpv), _) => (mpv.to_owned(), false),
        (None, Some(c)) => (c.to_string(), true),
        (None, None) if is_modifier(key) => return None,
        (None, None) => {
            let upper = gdk_name.to_ascii_uppercase();
            // mpv writes keypad digits without the underscore: KP0, not KP_0.
            match upper.strip_prefix("KP_") {
                Some(d) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
                    (format!("KP{d}"), false)
                }
                _ => (upper, false),
            }
        }
    };
    Some(with_modifiers_for(&base, state, printable))
}

/// `base` (an mpv key name such as `WHEEL_UP`) with the held modifiers.
pub fn with_modifiers(base: &str, state: ModifierType) -> String {
    with_modifiers_for(base, state, false)
}

fn with_modifiers_for(base: &str, state: ModifierType, printable: bool) -> String {
    let mut name = String::new();
    // A printable key already carries Shift in its character (`<`, `A`).
    if state.contains(ModifierType::SHIFT_MASK) && !printable {
        name.push_str("Shift+");
    }
    if state.contains(ModifierType::CONTROL_MASK) {
        name.push_str("Ctrl+");
    }
    if state.contains(ModifierType::ALT_MASK) {
        name.push_str("Alt+");
    }
    if state.contains(ModifierType::SUPER_MASK) {
        name.push_str("Meta+");
    }
    name.push_str(base);
    name
}

fn is_modifier(key: Key) -> bool {
    (Key::Shift_L.into_glib()..=Key::Hyper_R.into_glib()).contains(&key.into_glib())
        || key == Key::ISO_Level3_Shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_default_bindings() {
        let none = ModifierType::empty();
        assert_eq!(name(Key::space, none).as_deref(), Some("SPACE"));
        assert_eq!(
            name(Key::less, ModifierType::SHIFT_MASK).as_deref(),
            Some("<")
        );
        assert_eq!(
            name(Key::Up, ModifierType::SHIFT_MASK).as_deref(),
            Some("Shift+UP")
        );
        assert_eq!(
            name(Key::comma, ModifierType::CONTROL_MASK).as_deref(),
            Some("Ctrl+,")
        );
        assert_eq!(name(Key::F5, none).as_deref(), Some("F5"));
        assert_eq!(name(Key::Page_Up, none).as_deref(), Some("PGUP"));
        assert_eq!(name(Key::KP_1, none).as_deref(), Some("KP1"));
        assert_eq!(name(Key::KP_Enter, none).as_deref(), Some("KP_ENTER"));
        assert_eq!(name(Key::a, none).as_deref(), Some("a"));
        assert_eq!(name(Key::A, ModifierType::SHIFT_MASK).as_deref(), Some("A"));
        assert_eq!(name(Key::Shift_L, ModifierType::SHIFT_MASK), None);
    }
}
