// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: one settings page per installed applet.
//!
//! Every applet gets an instance of this one type. That is legal because
//! `Binder::register_page` takes an instance and, unlike `register::<P>()`, does not
//! record the type in `typed_page_ids` - so the "one page per Rust type" limit does not
//! apply here. It also means the usual `page_mut::<P>()` routing does not work for these
//! pages: every message carries the entity of the page it came from, and
//! [`super::update`] dispatches on that. See `.doc/settings/02-architecture.md`.

use super::schema::model::{Control, Group, Row, Schema, Setting};
use super::{Message, control, store::Store};
use crate::pages::desktop::panel::applets_inner::Applet;
use cosmic::Task;
use cosmic_settings_page::{self as page, Section, section};
use slab::Slab;
use slotmap::Key;
use slotmap::SlotMap;

pub struct Page {
    entity: page::Entity,
    /// Stable across runs, and what `activate_page` remembers as the last page open.
    id: String,
    applet: Applet<'static>,
    /// `None` when the applet ships no schema. The page still exists and says so:
    /// leaving it out would read as "no such applet".
    schema: Option<Schema>,
    /// Current raw RON text per setting slot; `None` until the page is first entered.
    values: Vec<Option<String>>,
    store: Option<Store>,
}

impl Page {
    pub fn new(parent_id: &str, applet: Applet<'static>, schema: Option<Schema>) -> Self {
        let values = vec![None; schema.as_ref().map_or(0, |schema| schema.slots)];

        Self {
            entity: page::Entity::null(),
            // Namespaced by parent so the panel's copy and the dock's copy of an applet
            // page stay distinct: `find_page_by_id` returns the first match, and two
            // pages sharing an id would make one of them unreachable.
            id: format!("applet:{parent_id}:{}", applet.id),
            applet,
            schema,
            values,
            store: None,
        }
    }

    pub fn entity(&self) -> page::Entity {
        self.entity
    }

    pub fn update(&mut self, message: Message) -> Task<crate::app::Message> {
        match message {
            Message::Surface(action) => {
                return cosmic::task::message(crate::app::Message::Surface(action));
            }

            Message::Toggle { slot, value, .. } => {
                self.write(slot, if value { "true" } else { "false" }.to_owned());
            }

            Message::Choose { slot, item, .. } => {
                if let Some(value) = self.option_value(slot, item) {
                    self.write(slot, value);
                }
            }
        }

        Task::none()
    }

    /// Write one key and, if it landed, remember the new value.
    ///
    /// One key at a time, never `write_entry`: the schema does not describe every field
    /// of the owner's config, and writing the struct would erase the rest.
    fn write(&mut self, slot: usize, value: String) {
        let Some(key) = self.setting(slot).map(|setting| setting.key.clone()) else {
            return;
        };

        let wrote = self
            .store
            .as_ref()
            .is_some_and(|store| store.write(&key, &value));

        if wrote && let Some(current) = self.values.get_mut(slot) {
            *current = Some(value);
        }
    }

    fn option_value(&self, slot: usize, item: usize) -> Option<String> {
        match &self.setting(slot)?.control {
            Control::Choice { items, .. } => items.get(item).map(|item| item.value.clone()),
            Control::Toggle => None,
        }
    }

    fn setting(&self, slot: usize) -> Option<&Setting> {
        self.schema.as_ref()?.groups.iter().find_map(|group| {
            group.rows.iter().find_map(|row| match row {
                Row::Setting(setting) if setting.slot == slot => Some(setting),
                _ => None,
            })
        })
    }

    /// Re-read every key the schema names.
    ///
    /// Reads are a handful of small files, so this runs inline the way `time::date` and
    /// the panel pages read their config; there is no blocking collection to push onto a
    /// worker thread.
    fn reload(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };

        let Some(schema) = self.schema.as_ref() else {
            return;
        };

        let mut values = vec![None; schema.slots];
        for group in &schema.groups {
            for row in &group.rows {
                if let Row::Setting(setting) = row
                    && let Some(slot) = values.get_mut(setting.slot)
                {
                    *slot = store.read(&setting.key);
                }
            }
        }

        self.values = values;
    }
}

impl page::Page<crate::pages::Message> for Page {
    fn set_id(&mut self, entity: page::Entity) {
        self.entity = entity;
    }

    fn info(&self) -> page::Info {
        // Title, description and icon come from the `.desktop` file the applet already
        // has to ship, so a schema never restates them and never contradicts the applet
        // list a click away.
        let icon = if self.applet.icon.is_empty() {
            "application-x-executable-symbolic".to_owned()
        } else {
            self.applet.icon.to_string()
        };

        page::Info::new(self.id.clone(), icon)
            .title(self.applet.name.to_string())
            .description(self.applet.description.to_string())
    }

    fn content(
        &self,
        sections: &mut SlotMap<section::Entity, Section<crate::pages::Message>>,
    ) -> Option<page::Content> {
        let Some(schema) = self.schema.as_ref() else {
            return Some(vec![sections.insert(message(fl!("applet-settings-none")))]);
        };

        if schema.too_new {
            return Some(vec![sections.insert(message(fl!("applet-settings-too-new")))]);
        }

        if schema.groups.is_empty() {
            return Some(vec![sections.insert(message(fl!("applet-settings-none")))]);
        }

        Some(
            (0..schema.groups.len())
                .map(|index| sections.insert(group_section(index, &schema.groups[index])))
                .collect(),
        )
    }

    fn on_enter(&mut self) -> Task<crate::pages::Message> {
        if self.store.is_none()
            && let Some(schema) = self.schema.as_ref()
        {
            // Opened here rather than at registration: `Config::new` creates the user's
            // config directory, and doing that for every installed applet at startup
            // would leave directories behind for applets that were never run.
            self.store = Store::open(&schema.config);
        }

        self.reload();
        Task::none()
    }
}

/// A section rendering one group of the schema.
///
/// The group is looked up by index at draw time instead of being captured: `content()`
/// runs before `set_id()`, so a closure that captured page state here would capture it
/// from the instance as it was at registration - including a null entity.
fn group_section(index: usize, group: &Group) -> Section<crate::pages::Message> {
    let mut descriptions = Slab::new();
    for row in &group.rows {
        if let Row::Setting(setting) = row {
            descriptions.insert(setting.label.clone());
        }
    }

    Section::default()
        .title(group.title.clone())
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, _section| {
            match page.schema.as_ref().and_then(|s| s.groups.get(index)) {
                Some(group) => control::group(group, &page.values, page.entity),
                None => cosmic::widget::space().into(),
            }
        })
}

/// A section that states why there is nothing to configure.
fn message(text: String) -> Section<crate::pages::Message> {
    Section::default()
        .search_ignore()
        .view::<Page>(move |_binder, _page, _section| control::message(text.clone()))
}
