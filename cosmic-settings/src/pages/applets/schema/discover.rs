// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: finding applet settings schemas on disk and turning them into pages.
//!
//! The lookup is deliberately the same shape as the one that finds the applets
//! themselves: walk the XDG data directories, first file of a given name wins. That is
//! what lets a third-party pacman package drop in a schema and appear in Settings with
//! no rebuild, and what lets a developer shadow a shipped schema from
//! `~/.local/share` while writing one.

use super::l10n;
use super::model::{
    ChoiceItem, Control, Group, Number, Row, SUPPORTED, Schema, Setting, Slider, TextField, raw,
};
use freedesktop_desktop_entry::get_languages_from_env;
use ron::value::RawValue;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

/// Directory under each XDG data directory that holds the schemas.
const SUBDIR: &str = "wmde/applet-settings";

/// RON insists on `Some(x)` for every `Option` field unless this extension is on. Schema
/// files are written by hand, so the parser carries that burden rather than the author.
fn options() -> ron::Options {
    ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
}

/// Data directories in precedence order: the user's own first, then the system's.
///
/// Shared with [`super::super::store`], which resolves an applet's installed default
/// directory the same way.
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(home) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(home));
    } else if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(home).join(".local/share"));
    }

    let system = std::env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(std::env::split_paths(&system));

    dirs
}

/// Every schema installed on this system, keyed by the id of the applet's `.desktop`
/// file. A file that fails to parse is skipped with a warning rather than taken as
/// "this applet has no settings": the two are different, and only one is a bug.
pub fn load_all() -> HashMap<String, Schema> {
    let languages = get_languages_from_env();
    let mut found: HashMap<String, Schema> = HashMap::new();

    for dir in data_dirs() {
        let Ok(entries) = std::fs::read_dir(dir.join(SUBDIR)) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "ron") {
                continue;
            }

            let Some(applet_id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };

            // First directory wins, so a user override shadows the packaged file.
            if found.contains_key(applet_id) {
                continue;
            }

            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) => {
                    warn!(?path, %err, "cannot read applet settings schema");
                    continue;
                }
            };

            match parse(&text, &languages) {
                Ok(schema) => {
                    found.insert(applet_id.to_owned(), schema);
                }
                Err(err) => warn!(?path, %err, "cannot parse applet settings schema"),
            }
        }
    }

    found
}

/// Parse one schema file. Split out so the tests can drive it without touching disk.
pub fn parse(text: &str, languages: &[String]) -> Result<Schema, ron::error::SpannedError> {
    let raw: raw::Schema = options().from_str(text)?;

    // A file from the future still gets a page. Its rows are not rendered, because this
    // build cannot know what they mean, but the applet does not silently look settingless.
    if raw.schema > SUPPORTED {
        return Ok(Schema {
            config: raw.config,
            groups: Vec::new(),
            slots: 0,
            too_new: true,
        });
    }

    let mut slots = 0;
    let mut groups = Vec::with_capacity(raw.groups.len());

    for raw_group in raw.groups {
        let title = l10n::resolve(&raw_group.title, languages);
        let mut rows = Vec::with_capacity(raw_group.rows.len());

        for raw_row in raw_group.rows {
            let row: raw::Row = match options().from_str(raw_row.get_ron()) {
                Ok(row) => row,
                // One row from a newer schema, or one typo, must not cost the whole page.
                Err(err) => {
                    warn!(row = raw_row.get_ron(), %err, "skipping applet settings row");
                    continue;
                }
            };

            if let Some(row) = convert(row, languages, &mut slots) {
                rows.push(row);
            }
        }

        if !rows.is_empty() {
            groups.push(Group { title, rows });
        }
    }

    Ok(Schema {
        config: raw.config,
        groups,
        slots,
        too_new: false,
    })
}

fn convert(row: raw::Row, languages: &[String], slots: &mut usize) -> Option<Row> {
    match row {
        raw::Row::Note { text } => Some(Row::Note(l10n::resolve(&text, languages))),

        raw::Row::Link { page, label } => Some(Row::Link {
            page,
            label: l10n::resolve(&label, languages),
        }),

        raw::Row::External { exec, label } => {
            // An empty command would render a button that does nothing at all.
            if exec.trim().is_empty() {
                warn!("dropping external row: no command");
                return None;
            }

            Some(Row::External {
                exec,
                label: l10n::resolve(&label, languages),
            })
        }

        raw::Row::Setting {
            key,
            label,
            description,
            control,
            default,
        } => {
            // A default the config layer would reject is worse than no default: "reset"
            // would write garbage into a config this page does not own.
            let default = default.filter(|text| {
                let valid = is_value(text);
                if !valid {
                    warn!(key, text, "ignoring applet setting default: not valid RON");
                }
                valid
            });

            let control = convert_control(control, languages, &key)?;
            let slot = *slots;
            *slots += 1;

            Some(Row::Setting(Setting {
                key,
                label: l10n::resolve(&label, languages),
                description: l10n::resolve_opt(description.as_ref(), languages),
                control,
                default,
                slot,
            }))
        }
    }
}

fn convert_control(control: raw::Control, languages: &[String], key: &str) -> Option<Control> {
    match control {
        raw::Control::Toggle => Some(Control::Toggle),

        raw::Control::Choice { items, style } => {
            let items: Vec<ChoiceItem> = items
                .into_iter()
                .filter(|item| {
                    let valid = is_value(&item.value);
                    if !valid {
                        warn!(key, value = item.value, "dropping choice: not valid RON");
                    }
                    valid
                })
                .map(|item| ChoiceItem {
                    value: item.value,
                    label: l10n::resolve(&item.label, languages),
                })
                .collect();

            // A choice with nothing to choose is a dead row; drop it rather than render
            // a control that cannot be operated.
            if items.is_empty() {
                warn!(key, "dropping choice setting: no usable options");
                return None;
            }

            let style = style.resolve(items.len());
            Some(Control::Choice { items, style })
        }

        raw::Control::Number {
            min,
            max,
            step,
            decimals,
            optional,
            suffix,
        } => {
            let (min, max, step) = range(min, max, step, key)?;
            Some(Control::Number(Number {
                min,
                max,
                step,
                decimals,
                optional,
                suffix: l10n::resolve_opt(suffix.as_ref(), languages),
            }))
        }

        raw::Control::Slider {
            min,
            max,
            step,
            decimals,
            min_label,
            max_label,
        } => {
            let (min, max, step) = range(min, max, step, key)?;
            Some(Control::Slider(Slider {
                min,
                max,
                step,
                decimals,
                min_label: l10n::resolve_opt(min_label.as_ref(), languages),
                max_label: l10n::resolve_opt(max_label.as_ref(), languages),
            }))
        }

        raw::Control::Text {
            placeholder,
            max_len,
        } => Some(Control::Text(TextField {
            placeholder: l10n::resolve_opt(placeholder.as_ref(), languages),
            max_len,
        })),
    }
}

/// Reject a range the control cannot be driven through.
///
/// A reversed range or a zero step produces a widget that looks operable and is not, which
/// is worse than an absent row: the user would keep clicking a control that cannot move.
fn range(min: f64, max: f64, step: f64, key: &str) -> Option<(f64, f64, f64)> {
    if !min.is_finite() || !max.is_finite() || !step.is_finite() || min > max || step <= 0.0 {
        warn!(key, min, max, step, "dropping setting: unusable range");
        return None;
    }

    Some((min, max, step))
}

/// Is this text something the config layer can store?
///
/// `RawValue::from_ron` accepts a bare identifier, so unit enum variants (`Bottom`,
/// `Celsius`) pass without the schema author quoting them - which matters, because that
/// is exactly how cosmic-config writes them to disk.
fn is_value(text: &str) -> bool {
    RawValue::from_ron(text).is_ok()
}

#[cfg(test)]
mod tests {
    use super::super::model::ChoiceStyle;
    use super::*;

    fn languages() -> Vec<String> {
        vec!["uk_UA".to_owned(), "uk".to_owned()]
    }

    const SCHEMA: &str = r#"
(
    schema: 1,
    config: (id: "fun.wmde.AppletNotifications", version: 1),
    groups: [
        (
            title: { "": "Behaviour", "uk": "Поведінка" },
            rows: [
                Setting(
                    key: "do_not_disturb",
                    label: { "": "Do not disturb", "uk": "Не турбувати" },
                    description: { "": "Hide banners", "uk": "Ховати сповіщення" },
                    control: Toggle,
                    default: "false",
                ),
                Setting(
                    key: "anchor",
                    label: { "": "Position", "uk": "Розташування" },
                    control: Choice(items: [
                        (value: "TopRight", label: { "": "Top right", "uk": "Згори праворуч" }),
                        (value: "TopLeft",  label: { "": "Top left",  "uk": "Згори ліворуч" }),
                    ]),
                    default: "TopRight",
                ),
                Setting(
                    key: "from_a_newer_wmde",
                    label: { "": "Future", "uk": "Майбутнє" },
                    control: Hologram(depth: 4),
                ),
                Note(text: { "": "Managed by the daemon", "uk": "Керується службою" }),
            ],
        ),
    ],
)
"#;

    #[test]
    fn one_unreadable_row_does_not_cost_the_page() {
        let schema = parse(SCHEMA, &languages()).expect("schema parses");

        assert!(!schema.too_new);
        assert_eq!(schema.config.id, "fun.wmde.AppletNotifications");
        assert_eq!(schema.config.version, 1);
        assert_eq!(schema.groups.len(), 1);
        assert_eq!(schema.groups[0].title, "Поведінка");
        assert_eq!(schema.groups[0].rows.len(), 3, "the future row is dropped alone");
        assert_eq!(schema.slots, 2, "only the two readable settings take a slot");
    }

    #[test]
    fn labels_come_out_in_the_environment_language() {
        let schema = parse(SCHEMA, &languages()).expect("schema parses");
        let rows = &schema.groups[0].rows;

        let Row::Setting(toggle) = &rows[0] else {
            panic!("expected a setting");
        };
        assert_eq!(toggle.label, "Не турбувати");
        assert_eq!(toggle.description.as_deref(), Some("Ховати сповіщення"));
        assert_eq!(toggle.slot, 0);
        assert!(matches!(toggle.control, Control::Toggle));

        let Row::Setting(choice) = &rows[1] else {
            panic!("expected a setting");
        };
        let Control::Choice { items, style } = &choice.control else {
            panic!("expected a choice");
        };
        assert_eq!(items[0].label, "Згори праворуч");
        assert_eq!(items[0].value, "TopRight");
        assert_eq!(*style, ChoiceStyle::Radio, "two options stay radio buttons");
        assert_eq!(choice.slot, 1);

        assert!(matches!(&rows[2], Row::Note(text) if text == "Керується службою"));
    }

    #[test]
    fn english_answers_when_the_language_is_missing() {
        let schema = parse(SCHEMA, &["de".to_owned()]).expect("schema parses");
        assert_eq!(schema.groups[0].title, "Behaviour");
    }

    /// Parse every schema in the directory named by `WMDE_APPLET_SCHEMA_DIR`.
    ///
    /// Aims the real parser at the files a package actually ships, which the fixtures
    /// above cannot do - they are written to exercise the parser, not to match what is
    /// installed. Without the variable this is a no-op, so `cargo test` never depends on
    /// where a checkout happens to sit.
    #[test]
    fn shipped_schemas_parse() {
        let Some(dir) = std::env::var_os("WMDE_APPLET_SCHEMA_DIR") else {
            return;
        };

        let entries = std::fs::read_dir(&dir).expect("schema directory must be readable");
        let mut checked = 0;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "ron") {
                continue;
            }

            let text = std::fs::read_to_string(&path).expect("schema must be readable");
            let schema = parse(&text, &languages())
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));

            assert!(
                !schema.groups.is_empty() && !schema.too_new,
                "{}: nothing left after parsing - every row was dropped",
                path.display()
            );
            checked += 1;
        }

        assert!(checked > 0, "{dir:?} held no schemas");
    }

    #[test]
    fn a_schema_from_the_future_still_produces_a_page() {
        let text = r#"(schema: 99, config: (id: "x", version: 1), groups: [])"#;
        let schema = parse(text, &languages()).expect("header parses at any version");
        assert!(schema.too_new);
        assert!(schema.groups.is_empty());
    }

    #[test]
    fn a_default_that_is_not_ron_is_refused() {
        let text = r#"
(
    schema: 1,
    config: (id: "x", version: 1),
    groups: [(
        title: { "": "G" },
        rows: [Setting(key: "k", label: { "": "L" }, control: Toggle, default: "(unclosed")],
    )],
)
"#;
        let schema = parse(text, &languages()).expect("schema parses");
        let Row::Setting(setting) = &schema.groups[0].rows[0] else {
            panic!("expected a setting");
        };
        assert!(setting.default.is_none(), "the broken default is dropped");
    }

    #[test]
    fn a_choice_with_no_usable_options_is_dropped() {
        let text = r#"
(
    schema: 1,
    config: (id: "x", version: 1),
    groups: [(
        title: { "": "G" },
        rows: [Setting(
            key: "k",
            label: { "": "L" },
            control: Choice(items: [(value: "(unclosed", label: { "": "Broken" })]),
        )],
    )],
)
"#;
        let schema = parse(text, &languages()).expect("schema parses");
        assert!(schema.groups.is_empty(), "an empty group is not rendered");
        assert_eq!(schema.slots, 0, "a dropped setting claims no slot");
    }

    #[test]
    fn numbers_sliders_and_text_parse() {
        let text = r#"
(
    schema: 1,
    config: (id: "x", version: 1),
    groups: [(
        title: { "": "G" },
        rows: [
            Setting(key: "a", label: { "": "A" }, control: Number(
                min: 500, max: 60000, step: 500, optional: true,
                suffix: { "": "ms", "uk": "мс" },
            ), default: "Some(5000)"),
            Setting(key: "b", label: { "": "B" }, control: Slider(
                min: 0.0, max: 1.0, step: 0.05, decimals: 2,
                min_label: { "": "quiet" }, max_label: { "": "loud" },
            )),
            Setting(key: "c", label: { "": "C" }, control: Text(max_len: 64)),
        ],
    )],
)
"#;
        let schema = parse(text, &languages()).expect("schema parses");
        let rows = &schema.groups[0].rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(schema.slots, 3);

        let Row::Setting(number) = &rows[0] else {
            panic!("expected a setting");
        };
        let Control::Number(number_control) = &number.control else {
            panic!("expected a number");
        };
        assert!(number_control.optional);
        assert_eq!(number_control.step, 500.0);
        assert_eq!(number_control.decimals, 0, "decimals default to whole numbers");
        assert_eq!(number_control.suffix.as_deref(), Some("мс"));
        assert_eq!(number.default.as_deref(), Some("Some(5000)"));

        let Row::Setting(slider) = &rows[1] else {
            panic!("expected a setting");
        };
        let Control::Slider(slider_control) = &slider.control else {
            panic!("expected a slider");
        };
        assert_eq!(slider_control.decimals, 2);
        assert_eq!(slider_control.min_label.as_deref(), Some("quiet"));

        let Row::Setting(field) = &rows[2] else {
            panic!("expected a setting");
        };
        let Control::Text(text_control) = &field.control else {
            panic!("expected a text field");
        };
        assert_eq!(text_control.max_len, Some(64));
        assert!(text_control.placeholder.is_none());
    }

    #[test]
    fn an_unusable_range_drops_the_setting() {
        let template = |control: &str| {
            format!(
                r#"(schema: 1, config: (id: "x", version: 1), groups: [(
                    title: {{ "": "G" }},
                    rows: [Setting(key: "k", label: {{ "": "L" }}, control: {control})],
                )])"#
            )
        };

        for control in [
            "Number(min: 10, max: 1)",
            "Number(min: 0, max: 10, step: 0)",
            "Slider(min: 0, max: 10, step: -1)",
        ] {
            let schema = parse(&template(control), &languages()).expect("schema parses");
            assert!(schema.groups.is_empty(), "{control} should have been dropped");
            assert_eq!(schema.slots, 0);
        }
    }

    #[test]
    fn five_options_become_a_dropdown() {
        let text = r#"
(
    schema: 1,
    config: (id: "x", version: 1),
    groups: [(
        title: { "": "G" },
        rows: [Setting(key: "k", label: { "": "L" }, control: Choice(items: [
            (value: "1", label: { "": "1" }),
            (value: "2", label: { "": "2" }),
            (value: "3", label: { "": "3" }),
            (value: "4", label: { "": "4" }),
            (value: "5", label: { "": "5" }),
        ]))],
    )],
)
"#;
        let schema = parse(text, &languages()).expect("schema parses");
        let Row::Setting(setting) = &schema.groups[0].rows[0] else {
            panic!("expected a setting");
        };
        let Control::Choice { style, .. } = &setting.control else {
            panic!("expected a choice");
        };
        assert_eq!(*style, ChoiceStyle::Dropdown);
    }
}
