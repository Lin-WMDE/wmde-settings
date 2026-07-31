// SPDX-License-Identifier: GPL-3.0-only

//! What an application says about the key bindings it handles itself.
//!
//! An application ships one of these next to its binary; the settings app finds it
//! and builds a page from it. Nothing here is compiled into Settings, so an
//! application installed later gets an editor without a rebuild - the same bargain
//! as applet settings schemas, and the same consequence: these strings get no
//! compiler check, and a missing `uk` degrades to English.
//!
//! The declaration says which actions exist, what to call them and which
//! combination each ships with. It does **not** say what an action does: Settings
//! only carries the name between here and the application's config file.

use crate::pages::applets::schema::model::L10n;
use cosmic::shortcuts::Binding;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Versions of the declaration format this build understands.
pub const SUPPORTED: u16 = 1;

/// Directory under each XDG data directory that holds the declarations.
pub const SUBDIR: &str = "wmde/app-shortcuts";

#[derive(Clone, Debug, Deserialize)]
pub struct Declaration {
    pub schema: u16,
    pub config: ConfigTarget,
    pub groups: Vec<Group>,
}

/// Where the edited bindings are written.
///
/// The application owns this file; Settings is a second writer to it, which is why
/// the key name is declared rather than assumed.
#[derive(Clone, Debug, Deserialize)]
pub struct ConfigTarget {
    pub id: String,
    pub version: u64,
    pub key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Group {
    pub title: L10n,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Action {
    /// The name the application spells in its own configuration.
    pub action: String,
    pub label: L10n,
    /// The combinations the application ships for this action.
    #[serde(default)]
    pub defaults: Vec<Binding>,
}

impl Declaration {
    /// Every declared action, in the order the groups list them.
    pub fn actions(&self) -> impl Iterator<Item = &Action> {
        self.groups.iter().flat_map(|group| group.actions.iter())
    }

    /// The bindings the application ships, gathered from the declaration.
    pub fn defaults(&self) -> cosmic::shortcuts::Shortcuts<super::ActionName> {
        self.actions()
            .flat_map(|action| {
                action
                    .defaults
                    .iter()
                    .map(|binding| (binding.clone(), super::ActionName::new(&action.action)))
            })
            .collect()
    }
}

/// Reads one declaration, refusing a format this build does not understand.
pub fn parse(path: &Path, text: &str) -> Result<Declaration, String> {
    let declaration: Declaration = ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .from_str(text)
        .map_err(|why| format!("{}: {why}", path.display()))?;

    if declaration.schema > SUPPORTED {
        return Err(format!(
            "{}: declaration version {} is newer than {SUPPORTED}",
            path.display(),
            declaration.schema
        ));
    }

    Ok(declaration)
}

/// Every declaration installed, keyed by the application id its file is named after.
///
/// Walks the XDG data directories, user first, so a declaration in the home
/// directory shadows the packaged one - which is how a developer tries a change
/// without reinstalling.
pub fn load_all() -> Vec<(String, Declaration)> {
    let mut found: Vec<(String, Declaration)> = Vec::new();

    for dir in crate::pages::applets::schema::discover::data_dirs() {
        let path = dir.join(SUBDIR);
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };

        for entry in entries.flatten() {
            let file = entry.path();
            if file.extension().is_none_or(|ext| ext != "ron") {
                continue;
            }

            let Some(id) = file.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };

            if found.iter().any(|(known, _)| known == id) {
                continue;
            }

            match read(&file) {
                Ok(declaration) => found.push((id.to_owned(), declaration)),
                Err(why) => tracing::warn!("{why}"),
            }
        }
    }

    found.sort_by(|(a, _), (b, _)| a.cmp(b));
    found
}

fn read(path: &PathBuf) -> Result<Declaration, String> {
    let text = std::fs::read_to_string(path).map_err(|why| format!("{}: {why}", path.display()))?;
    parse(path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::shortcuts::ModifierName;

    const SAMPLE: &str = r#"(
    schema: 1,
    config: (id: "fun.wmde.term", version: 1, key: "shortcuts_custom"),
    groups: [
        (
            title: { "": "Tabs", "uk": "Вкладки" },
            actions: [
                (
                    action: "TabNew",
                    label: { "": "New tab", "uk": "Нова вкладка" },
                    defaults: [(modifiers: [Ctrl, Shift], key: "T")],
                ),
                (
                    action: "TabClose",
                    label: { "": "Close tab", "uk": "Закрити вкладку" },
                    defaults: [(modifiers: [Ctrl, Shift], key: "W")],
                ),
            ],
        ),
    ],
)"#;

    #[test]
    fn a_declaration_parses_into_actions_and_defaults() {
        let declaration = parse(Path::new("sample.ron"), SAMPLE).expect("the sample has to parse");

        assert_eq!(declaration.config.id, "fun.wmde.term");
        assert_eq!(declaration.config.key, "shortcuts_custom");
        assert_eq!(declaration.actions().count(), 2);

        let defaults = declaration.defaults();
        assert_eq!(
            defaults.0.get(&Binding::new(
                [ModifierName::Ctrl, ModifierName::Shift],
                "T"
            )),
            Some(&super::super::ActionName::new("TabNew"))
        );
    }

    #[test]
    fn a_newer_format_is_refused_rather_than_half_read() {
        let newer = SAMPLE.replace("schema: 1", "schema: 2");
        assert!(parse(Path::new("sample.ron"), &newer).is_err());
    }

    #[test]
    fn an_action_without_a_default_is_allowed() {
        let without = SAMPLE.replace("defaults: [(modifiers: [Ctrl, Shift], key: \"T\")],", "");
        let declaration = parse(Path::new("sample.ron"), &without).expect("has to parse");

        assert_eq!(declaration.actions().count(), 2);
        assert_eq!(declaration.defaults().len(), 1);
    }
}
