// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: applet settings, built from data instead of code.
//!
//! An applet declares its settings in
//! `<datadir>/wmde/applet-settings/<desktop id>.ron`; Settings finds every such file at
//! startup and gives each applet a page. Nothing about any particular applet is compiled
//! in, so a third-party applet installed as its own pacman package gets a working
//! settings page with no change to this program. The contract is written up in
//! `.doc/settings/08-applet-schemas.md`.
//!
//! Three consequences of building pages at runtime, all of them easy to get wrong:
//!
//! * Pages are registered with `Binder::register_page`, which does not fill
//!   `typed_page_ids`. `page_mut::<P>()` therefore returns `None` for them and the usual
//!   routing idiom in `app.rs` does not apply.
//! * Every message carries the entity of the page that produced it. Routing on
//!   `active_page` instead would misdeliver during a search, where sections belonging to
//!   several pages are drawn at once - and would write into another applet's config with
//!   no panic and no compiler complaint.
//! * `content()` runs before `set_id()`, so a section closure must read the page's
//!   entity from the model it is handed at draw time, never capture it.

mod applet;
mod control;
pub mod schema;
mod store;

use crate::pages::desktop::panel::applets_inner::{self, Applet, AppletsPage};
use crate::pages::desktop::dock;
use cosmic::Task;
use cosmic::surface;
use cosmic_settings_page as page;
use std::collections::HashMap;

pub use applet::Page as AppletPage;

/// Applet list pages that applet settings pages hang off, by `Info::id`.
///
/// One page per applet per list: the panel's copy and the dock's copy are separate
/// registrations so that the "back" link in the header returns where the user came from.
/// They edit the same config, because an applet has one configuration no matter which
/// bar it sits in.
const PARENTS: [&str; 2] = ["panel_applets", "dock_applets"];

#[derive(Clone, Debug)]
pub enum Message {
    Toggle {
        page: page::Entity,
        slot: usize,
        value: bool,
    },
    Choose {
        page: page::Entity,
        slot: usize,
        item: usize,
    },
    /// A dropdown's popup surface. Not tied to a page - it goes straight to the shell.
    Surface(surface::Action),
}

impl Message {
    /// The page this message belongs to, if it belongs to one.
    fn entity(&self) -> Option<page::Entity> {
        match self {
            Self::Toggle { page, .. } | Self::Choose { page, .. } => Some(*page),
            Self::Surface(_) => None,
        }
    }
}

impl From<Message> for crate::pages::Message {
    fn from(message: Message) -> Self {
        crate::pages::Message::AppletSettings(message)
    }
}

impl From<Message> for crate::app::Message {
    fn from(message: Message) -> Self {
        crate::pages::Message::AppletSettings(message).into()
    }
}

/// Give every installed applet a settings page.
///
/// Called once from `SettingsApp::init`, after the applet list pages exist: the applets
/// themselves are read off those pages rather than scanned again, so the settings page
/// list and the applet list can never disagree about what is installed.
pub fn register_all(binder: &mut page::Binder<crate::pages::Message>) {
    let mut schemas = schema::load_all();

    let parents: Vec<(page::Entity, String)> = binder
        .info
        .iter()
        .filter(|(_, info)| PARENTS.contains(&info.id.as_ref()))
        .map(|(entity, info)| (entity, info.id.to_string()))
        .collect();

    for (parent, parent_id) in parents {
        let Some(applets) = applets_of(binder, parent) else {
            continue;
        };

        let mut registered = HashMap::with_capacity(applets.len());

        for applet in applets {
            let schema = schemas.get(applet.id.as_ref()).cloned();
            let id = applet.id.to_string();
            let child = binder.register_page(AppletPage::new(&parent_id, applet, schema));

            // Sets the breadcrumb in the page header and the nav bar item that stays
            // highlighted. Not added to `binder.sub_pages`: the applet list page has its
            // own `content()`, and `SettingsApp::view` checks content before sub-pages,
            // so a sub-page list there would never be drawn.
            binder.info[child].parent = Some(parent);
            registered.insert(id, child);
        }

        set_settings_pages(binder, parent, registered);
    }

    // Schemas whose applet is not installed. Worth saying out loud: the usual cause is a
    // schema file whose name does not match the applet's desktop id.
    schemas.retain(|id, _| {
        !binder
            .info
            .iter()
            .any(|(_, info)| info.id.ends_with(&format!(":{id}")))
    });

    for id in schemas.keys() {
        tracing::warn!(id, "applet settings schema has no matching applet");
    }
}

/// Route a message to the page that produced it.
pub fn update(
    binder: &mut page::Binder<crate::pages::Message>,
    message: Message,
) -> Task<crate::app::Message> {
    if let Message::Surface(action) = message {
        return cosmic::task::message(crate::app::Message::Surface(action));
    }

    let Some(entity) = message.entity() else {
        return Task::none();
    };

    binder
        .page
        .get_mut(entity)
        .and_then(|page| page.downcast_mut::<AppletPage>())
        .map_or_else(Task::none, |page| page.update(message))
}

/// The applets an applet list page already found on disk.
fn applets_of(
    binder: &page::Binder<crate::pages::Message>,
    parent: page::Entity,
) -> Option<Vec<Applet<'static>>> {
    let page = binder.page.get(parent)?;

    // Two concrete types wear the `AppletsPage` trait, and downcasting needs a concrete
    // one: the panel owns the page, the dock wraps it.
    if let Some(page) = page.downcast_ref::<applets_inner::Page>() {
        return Some(page.available_entries.clone());
    }

    page.downcast_ref::<dock::applets::Page>()
        .map(|page| page.inner().available_entries.clone())
}

/// Hand the applet list page the map it needs to draw its settings buttons.
fn set_settings_pages(
    binder: &mut page::Binder<crate::pages::Message>,
    parent: page::Entity,
    pages: HashMap<String, page::Entity>,
) {
    let Some(page) = binder.page.get_mut(parent) else {
        return;
    };

    if let Some(page) = page.downcast_mut::<applets_inner::Page>() {
        page.settings_pages = pages;
    } else if let Some(page) = page.downcast_mut::<dock::applets::Page>() {
        page.inner_mut().settings_pages = pages;
    }
}
