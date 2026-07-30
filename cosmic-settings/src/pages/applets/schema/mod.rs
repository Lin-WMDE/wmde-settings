// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: the applet settings schema.

pub mod discover;
pub mod l10n;
pub mod model;

pub use discover::load_all;
pub use model::{ChoiceStyle, Control, Group, Row, Schema, Setting};
