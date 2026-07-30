// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: reading and writing a config whose Rust types this binary does not know.
//!
//! cosmic-config stores one key per file and the file holds RON text produced by
//! `ron::ser::to_string_pretty`. `ron::value::RawValue` serialises through a token the
//! RON serialiser recognises and re-emits verbatim, and deserialises the same way, so a
//! value can make the whole round trip byte for byte without anyone naming its type.
//! That is what makes a schema-driven page possible with no change to libcosmic and no
//! second implementation of the config path rules - version fallback and system
//! defaults included.
//!
//! Two rules here are not obvious and are load-bearing:
//!
//! * **Never `write_entry`.** It writes every field of the owner's struct, so it would
//!   erase whatever the schema does not mention. Settings writes one key at a time.
//! * **"Reset" writes the default, it does not delete the file.** The derive behind
//!   `CosmicConfigEntry` keeps the previous value in memory when a key fails to read, so
//!   a deleted file never reaches a running applet - it would look like the button did
//!   nothing until the next login.

use super::schema::model::ConfigRef;
use cosmic::cosmic_config::{Config, ConfigGet, ConfigSet};
use ron::value::RawValue;
use std::collections::HashSet;
use std::path::PathBuf;
use tracing::{error, warn};

pub struct Store {
    config: Config,
    /// Keys the owner shipped a default for. `None` means it installed no defaults at
    /// all. Used only to warn - see [`Store::write`].
    known_keys: Option<HashSet<String>>,
    id: String,
}

impl Store {
    /// Open the config a schema points at.
    ///
    /// Deliberately called when the page is entered rather than when it is registered:
    /// `Config::new` creates the user's config directory, and doing that for every
    /// installed applet at startup would litter `~/.config/wmde` with directories for
    /// applets the user never ran.
    pub fn open(reference: &ConfigRef) -> Option<Self> {
        match Config::new(&reference.id, reference.version) {
            Ok(config) => Some(Self {
                config,
                known_keys: known_keys(reference),
                id: reference.id.clone(),
            }),
            Err(err) => {
                error!(id = reference.id, %err, "cannot open applet config");
                None
            }
        }
    }

    /// The current value as raw RON text: the user's, else an older version's, else the
    /// system default. `None` means the key has never been set anywhere.
    pub fn read(&self, key: &str) -> Option<String> {
        self.config
            .get::<Box<RawValue>>(key)
            .ok()
            .map(|value| value.get_ron().trim().to_owned())
    }

    /// Write raw RON text to one key. Returns whether it landed.
    pub fn write(&self, key: &str, text: &str) -> bool {
        // A typo in a schema key is worth shouting about: an owner that declares
        // `deny_unknown_fields` drops its ENTIRE config to defaults when it meets one
        // file it does not recognise. It is only a warning, though, and must stay one -
        // the installed defaults are the keys somebody chose to override, not the keys
        // the applet accepts. `fun.wmde.AppList` ships two of its three, so refusing
        // anything absent here would block a correct schema.
        if let Some(known) = self.known_keys.as_ref()
            && !known.contains(key)
        {
            warn!(
                id = self.id,
                key, "applet ships no default for this key - check the schema for a typo"
            );
        }

        let value = match RawValue::from_ron(text) {
            Ok(value) => value,
            Err(err) => {
                error!(id = self.id, key, text, %err, "schema value is not valid RON");
                return false;
            }
        };

        if let Err(err) = self.config.set(key, value) {
            error!(id = self.id, key, %err, "cannot write applet config");
            return false;
        }

        true
    }
}

/// Keys the applet's package installed defaults for.
///
/// `cosmic_config` can read a system default but cannot list them, so the directory is
/// resolved the same way it resolves one: the XDG data directories, in order.
fn known_keys(reference: &ConfigRef) -> Option<HashSet<String>> {
    let suffix = PathBuf::from("wmde")
        .join(&reference.id)
        .join(format!("v{}", reference.version));

    for dir in super::schema::discover::data_dirs() {
        let Ok(entries) = std::fs::read_dir(dir.join(&suffix)) else {
            continue;
        };

        return Some(
            entries
                .flatten()
                .filter(|entry| entry.path().is_file())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect(),
        );
    }

    None
}

/// Do two raw RON texts mean the same value?
///
/// Textual equality is not enough: a schema may write `10` where the applet's own
/// default file says `10.0`, and an option list must still highlight the right row.
/// Numbers are therefore compared as numbers, everything else as trimmed text.
pub fn same_value(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());

    if a == b {
        return true;
    }

    if let (Ok(a), Ok(b)) = (a.parse::<i64>(), b.parse::<i64>()) {
        return a == b;
    }

    if let (Ok(a), Ok(b)) = (a.parse::<f64>(), b.parse::<f64>()) {
        return a == b;
    }

    false
}

/// Read a boolean out of raw RON text, for the toggle control.
pub fn as_bool(text: &str) -> Option<bool> {
    match text.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_compare_as_numbers() {
        assert!(same_value("10", "10"));
        assert!(same_value(" 10 ", "10"));
        assert!(same_value("10.0", "10"));
        assert!(same_value("50.4501", "50.45010"));
        assert!(!same_value("10", "11"));
    }

    #[test]
    fn identifiers_and_strings_compare_as_text() {
        assert!(same_value("Celsius", "Celsius"));
        assert!(!same_value("Celsius", "Fahrenheit"));
        // A quoted string and a bare identifier are different values on disk, and the
        // applet's own deserialiser treats them as different too.
        assert!(!same_value("\"Celsius\"", "Celsius"));
    }

    #[test]
    fn booleans_are_read_out_of_raw_text() {
        assert_eq!(as_bool("true"), Some(true));
        assert_eq!(as_bool(" false "), Some(false));
        assert_eq!(as_bool("Celsius"), None);
    }
}
