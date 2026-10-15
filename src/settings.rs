//! Persistent settings.
//!
//! Stored as a small INI-style file, because a settings file that can be read and
//! edited in a text editor is worth more than an opaque one: it can be copied
//! between machines, kept in a dotfiles repository, and inspected when something
//! behaves unexpectedly.
//!
//! Keys this version does not recognise are kept and written back rather than
//! dropped. A hand-written entry, or one written by a newer build, would
//! otherwise disappear the first time the settings window saved the file.
//!
//! Parsing is pure: it takes text and returns the settings plus a list of the
//! values it had to ignore. Reading and writing the file are thin wrappers over
//! that, so everything except the file handle itself is testable without a
//! temporary directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::hotkey::Hotkey;

/// Name of the settings file inside the application-data directory.
const FILE_NAME: &str = "lasso.ini";

/// Where the settings live, or `None` when the environment has no
/// application-data directory to put them in.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("lasso").join(FILE_NAME))
}

/// What a left click on the tray icon does.
///
/// Windows convention would have the primary action on the left button, but the
/// habit being matched here puts the menu there, so it is the default and the
/// other order is one setting away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayClick {
    /// Left click opens the menu; a right click captures straight away.
    Menu,
    /// Left click captures; a right click opens the menu.
    Capture,
}

impl TrayClick {
    fn parse(text: &str) -> Option<TrayClick> {
        match text.trim().to_ascii_lowercase().as_str() {
            "menu" => Some(TrayClick::Menu),
            "capture" => Some(TrayClick::Capture),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TrayClick::Menu => "menu",
            TrayClick::Capture => "capture",
        }
    }

    /// The order the option cycles through, for a control that offers a choice.
    pub fn toggled(self) -> TrayClick {
        match self {
            TrayClick::Menu => TrayClick::Capture,
            TrayClick::Capture => TrayClick::Menu,
        }
    }
}

/// Everything the settings file holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The global hotkey that starts a selection.
    pub hotkey: Hotkey,
    /// Put the result on the clipboard when a capture finishes.
    pub clipboard: bool,
    /// Also write the result to `output_dir`.
    pub save_file: bool,
    /// Where saved captures go.
    pub output_dir: String,
    /// What a left click on the tray icon does.
    pub tray_left_click: TrayClick,
    /// Keys this version does not know about, kept so they survive a save.
    unknown: BTreeMap<String, String>,
}

/// The folder captures are saved to when nothing says otherwise.
fn default_output_dir() -> String {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Pictures").join("lasso").to_string_lossy().into_owned()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey: Hotkey::DEFAULT,
            clipboard: true,
            // Writing a file is the surprising half of the two, so it is off
            // until asked for; the clipboard is the reason the tool exists.
            save_file: false,
            output_dir: default_output_dir(),
            tray_left_click: TrayClick::Menu,
            unknown: BTreeMap::new(),
        }
    }
}

/// Accept the spellings people actually write for a yes/no setting.
fn parse_bool(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

impl Settings {
    /// Parse the file's text, reporting every value that had to be ignored.
    ///
    /// Nothing here fails: a settings file with a typo in it should still start
    /// the program, with the affected setting left at its default and a note
    /// about which line to look at.
    pub fn parse(text: &str) -> (Settings, Vec<String>) {
        // A byte order mark is not whitespace, so trimming does not remove it. It
        // is dropped here because both Notepad and Windows PowerShell write UTF-8
        // with one by default, and left in place it would silently turn the first
        // key of the file into an unrecognised one.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);

        let mut settings = Settings::default();
        let mut warnings = Vec::new();

        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                warnings.push(format!("line {}: {raw:?} is not `key = value`", index + 1));
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();

            match key.as_str() {
                "hotkey" => match Hotkey::parse(value) {
                    Ok(hotkey) => settings.hotkey = hotkey,
                    Err(err) => warnings.push(format!(
                        "hotkey: {err}; keeping {}",
                        settings.hotkey
                    )),
                },
                "clipboard" => match parse_bool(value) {
                    Some(flag) => settings.clipboard = flag,
                    None => warnings.push(format!(
                        "clipboard: {value:?} is not a yes or no; keeping {}",
                        yes_no(settings.clipboard)
                    )),
                },
                "save_file" => match parse_bool(value) {
                    Some(flag) => settings.save_file = flag,
                    None => warnings.push(format!(
                        "save_file: {value:?} is not a yes or no; keeping {}",
                        yes_no(settings.save_file)
                    )),
                },
                "output_dir" => {
                    if value.is_empty() {
                        warnings.push("output_dir: is empty; keeping the default".to_string());
                    } else {
                        settings.output_dir = value.to_string();
                    }
                }
                "tray_left_click" => match TrayClick::parse(value) {
                    Some(click) => settings.tray_left_click = click,
                    None => warnings.push(format!(
                        "tray_left_click: {value:?} is neither menu nor capture; keeping {}",
                        settings.tray_left_click.as_str()
                    )),
                },
                _ => {
                    settings.unknown.insert(key, value.to_string());
                }
            }
        }

        (settings, warnings)
    }

    /// Render the settings back to the file's text.
    pub fn to_ini(&self) -> String {
        let mut text = String::new();
        text.push_str("# lasso settings\n");
        text.push_str("#\n");
        text.push_str("# hotkey           global shortcut that starts a selection, e.g. Ctrl+Shift+X\n");
        text.push_str("#                  at least one modifier is required\n");
        text.push_str("# clipboard        copy each capture to the clipboard (yes/no)\n");
        text.push_str("# save_file        also write each capture to output_dir (yes/no)\n");
        text.push_str("# output_dir       where saved captures go\n");
        text.push_str("# tray_left_click  what a left click on the tray icon does: menu or capture\n");
        text.push('\n');

        text.push_str(&format!("hotkey = {}\n", self.hotkey));
        text.push_str(&format!("clipboard = {}\n", yes_no(self.clipboard)));
        text.push_str(&format!("save_file = {}\n", yes_no(self.save_file)));
        text.push_str(&format!("output_dir = {}\n", self.output_dir));
        text.push_str(&format!(
            "tray_left_click = {}\n",
            self.tray_left_click.as_str()
        ));

        if !self.unknown.is_empty() {
            text.push_str("\n# kept from an older or newer version\n");
            for (key, value) in &self.unknown {
                text.push_str(&format!("{key} = {value}\n"));
            }
        }

        text
    }

    /// Read the settings from `path`, falling back to the defaults.
    pub fn load_from(path: &Path) -> Settings {
        let Ok(text) = std::fs::read_to_string(path) else {
            // A missing file is the normal first run, not a problem to report.
            return Settings::default();
        };
        let (settings, warnings) = Settings::parse(&text);
        for warning in warnings {
            eprintln!("[settings] {}: {warning}", path.display());
        }
        settings
    }

    /// Write the settings to `path`, creating the directory if needed.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        std::fs::write(path, self.to_ini())
            .with_context(|| format!("could not write {}", path.display()))
    }

    /// Read the settings from the standard location.
    pub fn load() -> Settings {
        match path() {
            Some(path) => Settings::load_from(&path),
            None => Settings::default(),
        }
    }

    /// Write the settings to the standard location.
    pub fn save(&self) -> Result<()> {
        let path = path().context("there is no application-data directory to save into")?;
        self.save_to(&path)
    }
}

fn yes_no(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_usable_without_a_file() {
        let settings = Settings::default();
        assert_eq!(settings.hotkey, Hotkey::DEFAULT);
        assert!(settings.clipboard, "the clipboard is the point of the tool");
        assert!(!settings.save_file, "writing files is opt-in");
        assert_eq!(settings.tray_left_click, TrayClick::Menu);
        assert!(!settings.output_dir.is_empty());
    }

    #[test]
    fn a_full_file_is_read() {
        let text = "\
hotkey = Ctrl+Alt+F5
clipboard = no
save_file = yes
output_dir = C:\\shots
tray_left_click = capture
";
        let (settings, warnings) = Settings::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(settings.hotkey.to_string(), "Ctrl+Alt+F5");
        assert!(!settings.clipboard);
        assert!(settings.save_file);
        assert_eq!(settings.output_dir, "C:\\shots");
        assert_eq!(settings.tray_left_click, TrayClick::Capture);
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_first_setting() {
        // Written by Notepad and by Windows PowerShell, which both default to
        // UTF-8 with a BOM.
        let text = "\u{feff}hotkey = Ctrl+Shift+Y\nclipboard = no\n";
        let (settings, warnings) = Settings::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(settings.hotkey.to_string(), "Ctrl+Shift+Y");
        assert!(!settings.clipboard);
    }

    #[test]
    fn blank_lines_comments_and_spacing_are_ignored() {
        let text = "\
# a comment
; another comment

   hotkey   =   Ctrl+Shift+Y
   
clipboard=yes
";
        let (settings, warnings) = Settings::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(settings.hotkey.to_string(), "Ctrl+Shift+Y");
        assert!(settings.clipboard);
    }

    #[test]
    fn a_bad_value_warns_and_keeps_the_default() {
        let text = "\
hotkey = X
clipboard = maybe
save_file = 7
output_dir =
tray_left_click = sideways
";
        let (settings, warnings) = Settings::parse(text);
        assert_eq!(warnings.len(), 5, "{warnings:?}");
        assert_eq!(settings.hotkey, Hotkey::DEFAULT);
        assert!(settings.clipboard);
        assert!(!settings.save_file);
        assert_eq!(settings.output_dir, Settings::default().output_dir);
        assert_eq!(settings.tray_left_click, TrayClick::Menu);
        // Every message has to point at the setting it came from.
        for (warning, expected) in warnings.iter().zip([
            "hotkey",
            "clipboard",
            "save_file",
            "output_dir",
            "tray_left_click",
        ]) {
            assert!(warning.starts_with(expected), "{warning:?}");
        }
    }

    #[test]
    fn a_line_without_an_equals_sign_is_reported_with_its_number() {
        let (_, warnings) = Settings::parse("hotkey = Ctrl+Shift+X\nnonsense\n");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("line 2"), "{warnings:?}");
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let text = "future_option = 42\nhotkey = Ctrl+Shift+Y\n";
        let (settings, warnings) = Settings::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");

        let (again, warnings) = Settings::parse(&settings.to_ini());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, settings, "a round trip must not lose anything");
        assert!(settings.to_ini().contains("future_option = 42"));
    }

    #[test]
    fn rendering_and_parsing_agree() {
        let original = Settings {
            hotkey: Hotkey::parse("Ctrl+Alt+PrintScreen").unwrap(),
            clipboard: false,
            save_file: true,
            output_dir: "D:\\capturas".to_string(),
            tray_left_click: TrayClick::Capture,
            unknown: BTreeMap::new(),
        };
        let (parsed, warnings) = Settings::parse(&original.to_ini());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed, original);
    }

    #[test]
    fn saving_and_loading_a_file_keeps_the_settings() {
        // Inside the workspace, so the test never depends on a writable temp
        // directory.
        let path = Path::new("target/test-output/settings-roundtrip/lasso.ini");
        let _ = std::fs::remove_file(path);

        let settings = Settings {
            hotkey: Hotkey::parse("Win+Shift+S").unwrap(),
            clipboard: false,
            save_file: true,
            output_dir: "C:\\capturas".to_string(),
            tray_left_click: TrayClick::Capture,
            unknown: BTreeMap::new(),
        };
        settings.save_to(path).expect("saving must work");
        assert_eq!(Settings::load_from(path), settings);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_file_reads_as_the_defaults() {
        let path = Path::new("target/test-output/settings-absent/lasso.ini");
        let _ = std::fs::remove_file(path);
        assert_eq!(Settings::load_from(path), Settings::default());
    }

    #[test]
    fn the_tray_click_option_cycles_between_its_two_values() {
        assert_eq!(TrayClick::Menu.toggled(), TrayClick::Capture);
        assert_eq!(TrayClick::Capture.toggled(), TrayClick::Menu);
        assert_eq!(TrayClick::Menu.toggled().toggled(), TrayClick::Menu);
    }
}
