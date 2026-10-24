//! GDK key events to mpv key names, as [`muso_core::input`] expects them.

use gtk::gdk::{Key, ModifierType};
use gtk::glib::translate::IntoGlib;

/// The mpv name of a key press, or `None` for keys that are only modifiers.
pub fn name(key: Key, state: ModifierType) -> Option<String> {
    let special = match key {
        Key::space => Some("SPACE"),
        Key::Return | Key::ISO_Enter => Some("ENTER"),
        Key::KP_Enter => Some("KP_ENTER"),
        Key::Escape => Some("ESC"),
        Key::Tab | Key::ISO_Left_Tab => Some("TAB"),
        Key::BackSpace => Some("BS"),
        Key::Delete | Key::KP_Delete => Some("DEL"),
        Key::Insert => Some("INS"),
        Key::Home => Some("HOME"),
        Key::End => Some("END"),
        Key::Page_Up => Some("PGUP"),
        Key::Page_Down => Some("PGDWN"),
        Key::Left => Some("LEFT"),
        Key::Right => Some("RIGHT"),
        Key::Up => Some("UP"),
        Key::Down => Some("DOWN"),
        Key::numbersign => Some("SHARP"),
        Key::AudioPlay => Some("PLAY"),
        Key::AudioPause => Some("PAUSE"),
        Key::AudioStop => Some("STOP"),
        Key::AudioNext => Some("NEXT"),
        Key::AudioPrev => Some("PREV"),
        Key::AudioMute => Some("MUTE"),
        Key::AudioRaiseVolume => Some("VOLUME_UP"),
        Key::AudioLowerVolume => Some("VOLUME_DOWN"),
        _ => None,
    };
    let function = (Key::F1.into_glib()..=Key::F24.into_glib())
        .contains(&key.into_glib())
        .then(|| format!("F{}", key.into_glib() - Key::F1.into_glib() + 1));
    let (base, printable) = match (special, function) {
        (Some(s), _) => (s.to_owned(), false),
        (None, Some(f)) => (f, false),
        (None, None) => {
            let c = key.to_unicode().filter(|c| !c.is_control())?;
            (c.to_string(), true)
        }
    };
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
    name.push_str(&base);
    Some(name)
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
        assert_eq!(name(Key::Shift_L, ModifierType::SHIFT_MASK), None);
    }
}
