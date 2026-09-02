// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: settings pages for parts of the desktop that are not applets.
//!
//! The schema mechanism was written for applets, which are found through the panel that
//! carries them. A screenshot tool sits on no panel and still has settings, so the same
//! `.ron` schemas are read from a second directory and given a list of their own. Nothing
//! here knows any particular component: the list is whatever schemas are installed.

use super::applets::AppletPage;
use super::applets::schema::{self, discover::COMPONENT_SUBDIR};
use crate::pages::desktop::panel::applets_inner::Applet;
use cosmic_settings_page as page;
use freedesktop_desktop_entry::{DesktopEntry, get_languages_from_env};
use std::borrow::Cow;

#[derive(Default)]
pub struct Page {
    entity: page::Entity,
}

impl page::Page<crate::pages::Message> for Page {
    fn set_id(&mut self, entity: page::Entity) {
        self.entity = entity;
    }

    fn info(&self) -> page::Info {
        page::Info::new("components", "preferences-desktop-apps-symbolic").title(fl!("components"))
    }
}

impl page::AutoBind<crate::pages::Message> for Page {}

/// Give every installed component schema a page under `parent`.
///
/// The pages are children in the navigation sense, so the framework draws the list itself
/// and this page needs no content of its own.
pub fn register_all(binder: &mut page::Binder<crate::pages::Message>, parent: page::Entity) {
    let mut children = Vec::new();

    for (id, schema) in schema::discover::load_all_in(COMPONENT_SUBDIR) {
        let Some(component) = describe(&id) else {
            tracing::warn!("{id}: a component schema with no desktop entry beside it");
            continue;
        };

        let child = binder.register_page(AppletPage::for_component(component, Some(schema)));
        binder.info[child].parent = Some(parent);
        children.push(child);
    }

    if !children.is_empty() {
        binder.sub_pages.insert(parent, children);
    }
}

/// Name and icon for a component, read from its desktop entry - the same place the applet
/// list reads them from, so a component is described once and shown the same way wherever
/// it appears.
fn describe(id: &str) -> Option<Applet<'static>> {
    let languages = get_languages_from_env();

    for dir in schema::discover::data_dirs() {
        let path = dir.join("applications").join(format!("{id}.desktop"));
        let Ok(entry) = DesktopEntry::from_path(path.clone(), Some(&languages)) else {
            continue;
        };

        return Some(Applet {
            id: Cow::Owned(id.to_string()),
            name: Cow::Owned(
                entry
                    .name(&languages)
                    .unwrap_or(Cow::Borrowed(id))
                    .to_string(),
            ),
            description: Cow::Owned(
                entry
                    .comment(&languages)
                    .unwrap_or(Cow::Borrowed(""))
                    .to_string(),
            ),
            icon: Cow::Owned(
                entry
                    .icon()
                    .unwrap_or("application-x-executable")
                    .to_string(),
            ),
            path: Cow::Owned(path),
            single_instance: false,
        });
    }

    None
}
