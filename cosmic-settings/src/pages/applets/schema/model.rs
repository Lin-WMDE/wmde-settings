// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: the applet settings schema - the data an applet ships instead of code.
//!
//! Two layers live here. The `raw` types mirror the file exactly and are what serde
//! sees; the types above them are the resolved form the page renders, with every label
//! already reduced to one language and every setting given a slot in the page's value
//! vector.
//!
//! Keeping the layers apart is the whole point: a group captures its rows as
//! [`RawValue`] and converts them one at a time, so a row written against a newer
//! schema is dropped alone instead of taking the file with it.

use serde::Deserialize;
use std::collections::HashMap;

/// The schema language this build understands. A file declaring more still produces a
/// page - one that says so - because a silent omission reads as "this applet has no
/// settings", which is a different and wrong statement.
pub const SUPPORTED: u32 = 1;

/// A label in every language the applet author bothered to supply. The empty key is the
/// mandatory English fallback; see [`super::l10n`].
pub type L10n = HashMap<String, String>;

/// A resolved schema, ready to render.
#[derive(Clone)]
pub struct Schema {
    /// Where the settings are written. Not necessarily the applet's own id: the
    /// notifications applet configures the daemon, the tiling applet the compositor.
    pub config: ConfigRef,
    pub groups: Vec<Group>,
    /// Number of `Setting` rows, i.e. the length of the page's value vector.
    pub slots: usize,
    /// The file wants a newer Settings than this one.
    pub too_new: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ConfigRef {
    pub id: String,
    pub version: u64,
}

#[derive(Clone)]
pub struct Group {
    pub title: String,
    pub rows: Vec<Row>,
}

#[derive(Clone)]
pub enum Row {
    Setting(Setting),
    /// Read-only prose. The escape hatch for a setting that exists but is not editable
    /// here - a list managed by right-clicking the applet, a map keyed by panel name.
    Note(String),
}

#[derive(Clone)]
pub struct Setting {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub control: Control,
    /// Raw RON text. `None` means the schema declared no default, so "reset" falls back
    /// to whatever the system default directory holds.
    pub default: Option<String>,
    /// Index into the page's value vector.
    pub slot: usize,
}

#[derive(Clone)]
pub enum Control {
    Toggle,
    Choice {
        items: Vec<ChoiceItem>,
        style: ChoiceStyle,
    },
    Number(Number),
    Slider(Slider),
    Text(TextField),
}

/// A number entered with a spin button.
///
/// Everything is carried as `f64` and written back with `decimals` digits, so one code
/// path serves both `6` and `50.4501`; a schema declaring `decimals: 0` gets an integer
/// on disk, which is what an applet deserialising into `u32` requires.
#[derive(Clone)]
pub struct Number {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub decimals: u8,
    /// The key holds `None` or `Some(x)` rather than a bare number. The row then carries
    /// a toggle as well, because "no limit" is a value the spin button cannot express.
    pub optional: bool,
    pub suffix: Option<String>,
}

#[derive(Clone)]
pub struct Slider {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub decimals: u8,
    pub min_label: Option<String>,
    pub max_label: Option<String>,
}

#[derive(Clone)]
pub struct TextField {
    pub placeholder: Option<String>,
    pub max_len: Option<usize>,
}

#[derive(Clone)]
pub struct ChoiceItem {
    /// Raw RON text, written to the config verbatim: `Celsius`, `10`, `"large"`.
    pub value: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub enum ChoiceStyle {
    /// One row per option. Reads better and needs no popup, but eats vertical space.
    Radio,
    Dropdown,
    /// Radio up to [`Self::RADIO_LIMIT`] options, a dropdown beyond it.
    #[default]
    Auto,
}

impl ChoiceStyle {
    /// Above this many options a column of radio rows stops being scannable.
    const RADIO_LIMIT: usize = 4;

    pub fn resolve(self, items: usize) -> Self {
        match self {
            Self::Auto if items > Self::RADIO_LIMIT => Self::Dropdown,
            Self::Auto => Self::Radio,
            other => other,
        }
    }
}

/// The file as serde sees it.
pub mod raw {
    use super::{ConfigRef, L10n};
    use ron::value::RawValue;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct Schema {
        pub schema: u32,
        pub config: ConfigRef,
        #[serde(default)]
        pub groups: Vec<Group>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Group {
        pub title: L10n,
        /// Captured verbatim and converted one by one - see the module comment.
        #[serde(default)]
        pub rows: Vec<Box<RawValue>>,
    }

    #[derive(Debug, Deserialize)]
    pub enum Row {
        Setting {
            key: String,
            label: L10n,
            #[serde(default)]
            description: Option<L10n>,
            control: Control,
            #[serde(default)]
            default: Option<String>,
        },
        Note {
            text: L10n,
        },
    }

    #[derive(Debug, Deserialize)]
    pub enum Control {
        Toggle,
        Choice {
            items: Vec<ChoiceItem>,
            #[serde(default)]
            style: super::ChoiceStyle,
        },
        Number {
            min: f64,
            max: f64,
            #[serde(default = "one")]
            step: f64,
            #[serde(default)]
            decimals: u8,
            #[serde(default)]
            optional: bool,
            #[serde(default)]
            suffix: Option<L10n>,
        },
        Slider {
            min: f64,
            max: f64,
            #[serde(default = "one")]
            step: f64,
            #[serde(default)]
            decimals: u8,
            #[serde(default)]
            min_label: Option<L10n>,
            #[serde(default)]
            max_label: Option<L10n>,
        },
        Text {
            #[serde(default)]
            placeholder: Option<L10n>,
            #[serde(default)]
            max_len: Option<usize>,
        },
    }

    /// A step of zero would freeze the control, so the default is one rather than
    /// `f64::default()`.
    fn one() -> f64 {
        1.0
    }

    #[derive(Debug, Deserialize)]
    pub struct ChoiceItem {
        pub value: String,
        pub label: L10n,
    }
}
