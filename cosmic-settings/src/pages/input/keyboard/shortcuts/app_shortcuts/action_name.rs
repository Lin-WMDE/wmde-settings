// SPDX-License-Identifier: GPL-3.0-only

//! An action named by the application whose shortcuts are being edited.

use cosmic::shortcuts::ShortcutAction;
use serde::de::{Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashSet;
use std::fmt;
use std::sync::{Mutex, OnceLock};

/// The action an application spells in its own configuration.
///
/// The settings app never knows what these mean; it only carries them between the
/// declaration an application ships and the application's config file. They are
/// stored the way the application's own enum serializes - a bare RON identifier,
/// `TabNew`, not `"TabNew"` - so the application reads back exactly what it wrote.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionName(pub String);

impl ActionName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The action that takes a shipped binding away.
    pub fn disable() -> Self {
        Self::new("Disable")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ShortcutAction for ActionName {
    fn is_disable(&self) -> bool {
        self.0 == "Disable"
    }
}

impl Serialize for ActionName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // A unit variant is what writes the bare identifier, and serde wants a
        // 'static name for it. Interning bounds the leak by the number of distinct
        // actions the installed applications declare.
        serializer.serialize_unit_variant("Action", 0, intern(&self.0))
    }
}

impl<'de> Deserialize<'de> for ActionName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NameVisitor;

        impl Visitor<'_> for NameVisitor {
            type Value = ActionName;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an action name")
            }

            fn visit_str<E>(self, value: &str) -> Result<ActionName, E> {
                Ok(ActionName::new(value))
            }
        }

        deserializer.deserialize_identifier(NameVisitor)
    }
}

fn intern(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let names = NAMES.get_or_init(|| Mutex::new(HashSet::new()));
    let mut names = names
        .lock()
        .expect("the action name table is never poisoned");

    if let Some(interned) = names.get(name) {
        return interned;
    }

    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::shortcuts::{Binding, ModifierName, Shortcuts};

    /// What an application writes into its own config file. The settings app has to
    /// read and write this shape byte for byte, or the application stops
    /// recognising its own shortcuts.
    const STORED: &str = r#"{
    (modifiers: [Ctrl, Shift], key: "T"): TabNew,
    (modifiers: [Ctrl], key: "Q"): Disable,
}"#;

    #[test]
    fn an_action_reads_and_writes_as_a_bare_identifier() {
        let parsed: Shortcuts<ActionName> =
            ron::from_str(STORED).expect("stored shortcuts have to parse");

        assert_eq!(
            parsed.0.get(&Binding::new(
                [ModifierName::Ctrl, ModifierName::Shift],
                "T"
            )),
            Some(&ActionName::new("TabNew"))
        );

        let written = ron::to_string(&parsed).expect("shortcuts have to serialize");
        assert!(
            written.contains("TabNew") && !written.contains("\"TabNew\""),
            "an action has to be written unquoted, got {written}"
        );

        let reparsed: Shortcuts<ActionName> =
            ron::from_str(&written).expect("written shortcuts have to parse");
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn disable_is_recognised() {
        assert!(ActionName::disable().is_disable());
        assert!(!ActionName::new("TabNew").is_disable());
    }

    #[test]
    fn interning_hands_back_the_same_string() {
        assert_eq!(intern("TabNew").as_ptr(), intern("TabNew").as_ptr());
    }
}
