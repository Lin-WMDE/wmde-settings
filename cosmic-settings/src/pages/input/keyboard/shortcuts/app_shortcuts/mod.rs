// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: editing the key bindings an application handles itself.
//!
//! These never reach the compositor. An application declares what it can do and
//! where it stores the answer; this module reads that declaration, shows the
//! bindings and writes the user's changes back into the application's own config.

pub mod action_name;
pub mod declaration;

pub use action_name::ActionName;
pub use declaration::Declaration;
