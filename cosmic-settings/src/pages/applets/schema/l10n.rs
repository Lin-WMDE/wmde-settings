// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: picking one language out of a schema label.
//!
//! Settings embeds its own catalogues with `rust-embed` at compile time, so a package
//! installed later cannot add strings to this binary. A third-party applet therefore
//! ships its translations inside the schema file, the way a `.desktop` file carries
//! `Name[uk]`. The consequence is stated plainly in the handbook: these strings get no
//! compiler check, and a missing `uk` degrades to English instead of failing the build.

use super::model::L10n;

/// The key every schema must carry: English, used when nothing else matches.
const FALLBACK: &str = "";

/// Resolve a label against the environment's language preference.
///
/// `languages` comes from `freedesktop_desktop_entry::get_languages_from_env`, already
/// ordered most-specific first (`uk_UA` before `uk`), which is the same list the applet
/// list uses to read `Name`/`Comment` out of the `.desktop` file. Matching the two keeps
/// a page title and the labels under it in one language.
pub fn resolve(label: &L10n, languages: &[String]) -> String {
    for language in languages {
        if let Some(text) = label.get(language.as_str()) {
            return text.clone();
        }

        // `uk_UA` also answers to `uk`. get_languages_from_env usually supplies both, but
        // a schema that only spells one of them should still be found.
        if let Some((base, _)) = language.split_once(['_', '.', '@'])
            && let Some(text) = label.get(base)
        {
            return text.clone();
        }
    }

    label.get(FALLBACK).cloned().unwrap_or_default()
}

/// Same, for an optional label.
pub fn resolve_opt(label: Option<&L10n>, languages: &[String]) -> Option<String> {
    let text = resolve(label?, languages);
    (!text.is_empty()).then_some(text)
}
