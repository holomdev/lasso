//! Global hotkey combinations.
//!
//! Deliberately free of the Win32 API: a combination is stored as the plain
//! modifier flags and virtual-key code that `RegisterHotKey` expects, so this
//! module stays pure and can be tested without opening a window or touching the
//! keyboard.

use std::fmt;

/// Modifier flags, matching the `MOD_*` values `RegisterHotKey` takes.
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub alt: bool,
    pub control: bool,
    pub shift: bool,
    pub win: bool,
}

impl Modifiers {
    /// The flags in the order and form `RegisterHotKey` wants.
    pub fn bits(self) -> u32 {
        let mut bits = 0;
        if self.alt {
            bits |= MOD_ALT;
        }
        if self.control {
            bits |= MOD_CONTROL;
        }
        if self.shift {
            bits |= MOD_SHIFT;
        }
        if self.win {
            bits |= MOD_WIN;
        }
        bits
    }

    pub fn any(self) -> bool {
        self.alt || self.control || self.shift || self.win
    }
}

/// One modifier plus one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: Modifiers,
    /// A Win32 virtual-key code.
    pub key: u16,
}

/// Virtual-key codes used by the key names below. Kept as plain numbers so this
/// module needs no Win32 import.
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_SPACE: u16 = 0x20;
const VK_PRIOR: u16 = 0x21; // Page Up
const VK_NEXT: u16 = 0x22; // Page Down
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_SNAPSHOT: u16 = 0x2C; // Print Screen
const VK_INSERT: u16 = 0x2D;
const VK_DELETE: u16 = 0x2E;
const VK_F1: u16 = 0x70;
const VK_F24: u16 = 0x87;

impl Hotkey {
    /// The default trigger.
    ///
    /// Ctrl+Shift+X is used because the obvious choice is already taken:
    /// Windows 11 claims Print Screen for its own screenshot tool, and a global
    /// hotkey that fails to register leaves the user with a dead shortcut.
    pub const DEFAULT: Hotkey = Hotkey {
        modifiers: Modifiers {
            alt: false,
            control: true,
            shift: true,
            win: false,
        },
        key: 0x58, // X
    };

    /// Parse a combination such as `Ctrl+Shift+X`.
    ///
    /// At least one modifier is required. A bare key would be registered
    /// system-wide and swallowed from every other application, which is never
    /// what someone wants from a screenshot tool.
    pub fn parse(text: &str) -> Result<Hotkey, String> {
        let parts: Vec<&str> = text
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        let Some((last, rest)) = parts.split_last() else {
            return Err("the combination is empty".into());
        };

        let mut modifiers = Modifiers::default();
        for part in rest {
            match part.to_ascii_uppercase().as_str() {
                "CTRL" | "CONTROL" => modifiers.control = true,
                "SHIFT" => modifiers.shift = true,
                "ALT" => modifiers.alt = true,
                "WIN" | "WINDOWS" | "META" | "SUPER" => modifiers.win = true,
                // Reported as typed rather than upper-cased: an error that does
                // not echo what the user wrote is harder to act on.
                _ => return Err(format!("unknown modifier {part:?}")),
            }
        }

        if !modifiers.any() {
            return Err(format!("{text:?} needs at least one modifier"));
        }

        let key = parse_key(last)?;
        Ok(Hotkey { modifiers, key })
    }
}

fn parse_key(name: &str) -> Result<u16, String> {
    let upper = name.to_ascii_uppercase();

    // A single letter or digit.
    let mut characters = upper.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        if character.is_ascii_uppercase() || character.is_ascii_digit() {
            return Ok(character as u16);
        }
    }

    // Function keys.
    if let Some(number) = upper.strip_prefix('F') {
        if let Ok(index) = number.parse::<u16>() {
            if (1..=24).contains(&index) {
                return Ok(VK_F1 + index - 1);
            }
            return Err(format!("F{index} is out of range, F1 to F24"));
        }
    }

    for (alias, code) in NAMED_KEYS {
        if upper == *alias {
            return Ok(*code);
        }
    }

    Err(format!("unknown key {name:?}"))
}

/// Keys whose names are not a single character.
const NAMED_KEYS: &[(&str, u16)] = &[
    ("PRINTSCREEN", VK_SNAPSHOT),
    ("PRTSCN", VK_SNAPSHOT),
    ("PRINT", VK_SNAPSHOT),
    ("SPACE", VK_SPACE),
    ("TAB", VK_TAB),
    ("ENTER", VK_RETURN),
    ("RETURN", VK_RETURN),
    ("INSERT", VK_INSERT),
    ("INS", VK_INSERT),
    ("DELETE", VK_DELETE),
    ("DEL", VK_DELETE),
    ("HOME", VK_HOME),
    ("END", VK_END),
    ("PAGEUP", VK_PRIOR),
    ("PGUP", VK_PRIOR),
    ("PAGEDOWN", VK_NEXT),
    ("PGDN", VK_NEXT),
];

fn key_name(key: u16) -> String {
    if (VK_F1..=VK_F24).contains(&key) {
        return format!("F{}", key - VK_F1 + 1);
    }
    for (alias, code) in NAMED_KEYS {
        if key == *code {
            // The first alias for a code is the canonical name.
            return alias.to_string();
        }
    }
    if (0x30..=0x39).contains(&key) || (0x41..=0x5A).contains(&key) {
        return char::from(key as u8).to_string();
    }
    format!("0x{key:02X}")
}

impl fmt::Display for Hotkey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<&str> = Vec::new();
        if self.modifiers.control {
            parts.push("Ctrl");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        if self.modifiers.win {
            parts.push("Win");
        }
        let modifiers = parts.join("+");
        write!(formatter, "{modifiers}+{}", key_name(self.key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_a_modifier_combination() {
        assert_eq!(Hotkey::DEFAULT.to_string(), "Ctrl+Shift+X");
        assert_eq!(Hotkey::DEFAULT.modifiers.bits(), MOD_CONTROL | MOD_SHIFT);
    }

    #[test]
    fn combinations_are_parsed_case_insensitively_with_any_spacing() {
        let expected = Hotkey::parse("Ctrl+Shift+X").unwrap();
        for text in ["ctrl+shift+x", "CTRL + SHIFT + X", " Control+Shift+X "] {
            assert_eq!(Hotkey::parse(text).unwrap(), expected, "{text:?}");
        }
    }

    #[test]
    fn every_modifier_is_recognised() {
        let all = Hotkey::parse("Ctrl+Shift+Alt+Win+F5").unwrap();
        assert!(all.modifiers.control);
        assert!(all.modifiers.shift);
        assert!(all.modifiers.alt);
        assert!(all.modifiers.win);
        assert_eq!(all.to_string(), "Ctrl+Shift+Alt+Win+F5");
    }

    #[test]
    fn a_combination_without_a_modifier_is_refused() {
        // It would be swallowed from every other application.
        let error = Hotkey::parse("X").unwrap_err();
        assert!(error.contains("at least one modifier"), "{error}");
    }

    #[test]
    fn unknown_parts_are_reported_rather_than_ignored() {
        assert!(Hotkey::parse("Hyper+X").unwrap_err().contains("Hyper"));
        assert!(Hotkey::parse("Ctrl+Banana").unwrap_err().contains("Banana"));
        assert!(Hotkey::parse("Ctrl+F99").unwrap_err().contains("F99"));
        assert!(Hotkey::parse("").is_err());
    }

    #[test]
    fn function_keys_cover_their_full_range() {
        assert_eq!(Hotkey::parse("Ctrl+F1").unwrap().key, VK_F1);
        assert_eq!(Hotkey::parse("Ctrl+F24").unwrap().key, VK_F24);
        assert_eq!(Hotkey::parse("Ctrl+F12").unwrap().to_string(), "Ctrl+F12");
    }

    #[test]
    fn named_keys_round_trip() {
        for name in ["PrintScreen", "Space", "Tab", "Insert", "Delete", "Home", "End", "PageUp", "PageDown"] {
            let parsed = Hotkey::parse(&format!("Ctrl+{name}")).unwrap();
            let rendered = parsed.to_string();
            assert_eq!(
                Hotkey::parse(&rendered).unwrap(),
                parsed,
                "{name} rendered as {rendered} and did not survive a round trip"
            );
        }
    }

    #[test]
    fn rendering_round_trips_through_parsing() {
        for text in ["Ctrl+Shift+X", "Ctrl+Alt+A", "Win+Shift+PrintScreen", "Alt+F4"] {
            let parsed = Hotkey::parse(text).unwrap();
            assert_eq!(Hotkey::parse(&parsed.to_string()).unwrap(), parsed, "{text}");
        }
    }
}
