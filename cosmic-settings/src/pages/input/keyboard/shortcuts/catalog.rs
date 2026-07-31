// SPDX-License-Identifier: GPL-3.0-only

//! What a custom shortcut can be bound to, gathered from installed applications.
//!
//! An application declares its global actions in its desktop entry, the way the
//! specification already provides for: the `Exec` of the entry itself launches it,
//! and every `[Desktop Action]` group is one more thing it can be asked to do. That
//! costs an application nothing to opt into and third-party ones are already in,
//! which is why no format of our own is involved.
//!
//! The command that ends up stored is an ordinary shell command - the compositor
//! runs it through `/bin/sh -c` - so field codes have to go: nothing is going to
//! substitute a file list for `%U` at the time a key is pressed.

use cosmic::desktop::fde::IconSource;
use cosmic::desktop::{DesktopEntryData, load_applications};

/// One line of the picker: a thing a key combination can be bound to.
#[derive(Clone, Debug)]
pub struct Entry {
    /// What to call it.
    pub name: String,
    /// The application it belongs to, for an action that is not the application
    /// itself. Two applications may well both declare "New Window".
    pub application: Option<String>,
    pub icon: IconSource,
    /// The command stored in the shortcut.
    pub command: String,
    /// Lower-cased name and application, for matching against a search.
    haystack: String,
}

impl Entry {
    fn new(name: String, application: Option<String>, icon: IconSource, command: String) -> Self {
        let mut haystack = name.to_lowercase();
        if let Some(application) = &application {
            haystack.push(' ');
            haystack.push_str(&application.to_lowercase());
        }

        Self {
            name,
            application,
            icon,
            command,
            haystack,
        }
    }

    /// Whether a lower-cased search string matches this entry.
    #[must_use]
    pub fn matches(&self, search: &str) -> bool {
        search
            .split_whitespace()
            .all(|word| self.haystack.contains(word))
    }
}

/// Everything installed that can be launched, plus every action those declare.
///
/// Reads the disk, so call it when a picker opens rather than on every view.
#[must_use]
pub fn load(locales: &[String]) -> Vec<Entry> {
    let terminal = terminal_command();
    let mut entries = Vec::new();

    for app in load_applications(locales, false, None) {
        let icon = app.icon.clone();

        if let Some(command) = launch_command(&app, &terminal) {
            entries.push(Entry::new(app.name.clone(), None, icon.clone(), command));
        }

        for action in &app.desktop_actions {
            let command = strip_field_codes(&action.exec);
            if command.is_empty() {
                continue;
            }
            entries.push(Entry::new(
                action.name.clone(),
                Some(app.name.clone()),
                icon.clone(),
                command,
            ));
        }
    }

    entries.sort_by(|a, b| {
        // An application before its own actions, applications alphabetically.
        let a_key = (
            a.application.as_deref().unwrap_or(&a.name),
            a.application.is_some(),
            &a.name,
        );
        let b_key = (
            b.application.as_deref().unwrap_or(&b.name),
            b.application.is_some(),
            &b.name,
        );
        a_key.cmp(&b_key)
    });

    entries
}

/// The command that starts an application, wrapped for a terminal one.
fn launch_command(app: &DesktopEntryData, terminal: &str) -> Option<String> {
    let exec = strip_field_codes(app.exec.as_deref()?);
    if exec.is_empty() {
        return None;
    }

    Some(if app.terminal {
        format!("{terminal} -e {exec}")
    } else {
        exec
    })
}

/// What the desktop opens when something asks for a terminal.
fn terminal_command() -> String {
    cosmic_settings_config::shortcuts::context()
        .ok()
        .and_then(|config| {
            cosmic_settings_config::shortcuts::system_actions(&config)
                .get(&cosmic_settings_config::shortcuts::action::System::Terminal)
                .cloned()
        })
        .unwrap_or_else(|| String::from("wmde-term"))
}

/// Drops the desktop entry field codes from a command line.
///
/// `%U` and its siblings stand for the files or urls an application was opened
/// with. A shortcut opens it with nothing, and the compositor passes the string to
/// a shell verbatim, so a surviving code would arrive as a literal argument.
#[must_use]
pub fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|token| !is_field_code(token))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_field_code(token: &str) -> bool {
    let mut chars = token.chars();
    chars.next() == Some('%')
        && chars
            .next()
            .is_some_and(|code| "fFuUdDnNickvm".contains(code))
        && chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_codes_are_dropped_and_the_rest_survives() {
        assert_eq!(strip_field_codes("wmde-files %U"), "wmde-files");
        assert_eq!(
            strip_field_codes("wmde-files --trash"),
            "wmde-files --trash"
        );
        assert_eq!(
            strip_field_codes("firefox --private-window %u"),
            "firefox --private-window"
        );
        // Only a whole token is a field code.
        assert_eq!(strip_field_codes("run %d100"), "run %d100");
        assert_eq!(strip_field_codes("run 100%"), "run 100%");
    }

    #[test]
    fn a_search_matches_on_both_the_action_and_its_application() {
        let entry = Entry::new(
            "Нове вікно".to_string(),
            Some("WMDE Files".to_string()),
            IconSource::default(),
            "wmde-files".to_string(),
        );

        assert!(entry.matches("вікно"));
        assert!(entry.matches("files"));
        assert!(entry.matches("files вікно"));
        assert!(!entry.matches("термінал"));
    }
}
