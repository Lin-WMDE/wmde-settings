// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: the panels of the desktop, however many there are.
//!
//! `fun.wmde.Panel/v1/entries` names the panels; each name is the key of its own config,
//! `fun.wmde.Panel.<name>`. This page lists them and creates and destroys them; one
//! [`instance::Page`] per name carries the settings of a single panel, and one
//! [`applets_inner::Page`] per name carries its applets.
//!
//! Those child pages are registered at runtime and are therefore not reachable by type -
//! see the note at the top of [`instance`]. Everything that has to reach into the page
//! model lives here, in [`update`], because a page cannot get at the `Binder` from its own
//! `update`.

pub mod applets_inner;
pub mod inner;
pub mod instance;

use cosmic::cctk::sctk::reexports::client::Proxy;
use cosmic::cctk::sctk::reexports::client::backend::ObjectId;
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::cosmic_config::{ConfigGet, ConfigSet, CosmicConfigEntry};
use cosmic::widget::{button, dropdown, settings};
use cosmic::{Apply, Element, Task, surface};
use cosmic_panel_config::{
    CosmicPanelConfig, CosmicPanelContainerConfig, CosmicPanelOuput, PanelAnchor, PanelLook,
};
use cosmic_settings_page::{self as page, Section, section};
use slotmap::{Key, SlotMap};
use std::collections::HashMap;

use inner::{Anchor, Look};

pub struct Page {
    entity: page::Entity,
    /// The panels, in the order `entries` lists them.
    panels: Vec<Panel>,
    /// The new-panel dialog, while it is open.
    dialog: Option<NewPanel>,
    /// Displays, learned from the Wayland subscription and handed to every panel page.
    outputs: Vec<String>,
    outputs_map: HashMap<ObjectId, (String, WlOutput)>,
    anchors: Vec<String>,
    looks: Vec<String>,
}

struct Panel {
    name: String,
    page: page::Entity,
    title: String,
}

/// What the new-panel dialog has been told so far. Indices into the page's dropdown lists.
struct NewPanel {
    anchor: usize,
    output: usize,
    look: usize,
}

impl Default for NewPanel {
    fn default() -> Self {
        // Top: the edge the shipped panel does not occupy.
        Self {
            anchor: 2,
            output: 0,
            look: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    /// Open the new-panel dialog.
    Add,
    DialogAnchor(usize),
    DialogOutput(usize),
    DialogLook(usize),
    DialogCancel,
    DialogConfirm,
    /// Put the panels back to what the packages install.
    RestoreDefaults,
    /// A message from one panel's page.
    Instance {
        page: page::Entity,
        message: inner::Message,
    },
    /// Destroy the panel whose page this is.
    Remove {
        page: page::Entity,
    },
    /// The set of panels changed, here or anywhere else.
    Entries(Vec<String>),
    /// One panel's config changed, here or anywhere else.
    Config(Box<CosmicPanelConfig>),
    OutputAdded(String, WlOutput),
    OutputRemoved(WlOutput),
    Surface(surface::Action),
}

/// Tag a panel message with the page it came from. There is one page per panel, so the page
/// cannot be told from the message alone.
pub fn message(page: page::Entity, message: inner::Message) -> crate::pages::Message {
    crate::pages::Message::Panels(Message::Instance { page, message })
}

impl Default for Page {
    fn default() -> Self {
        Self {
            entity: page::Entity::null(),
            panels: Vec::new(),
            dialog: None,
            outputs: vec![fl!("all-displays")],
            outputs_map: HashMap::new(),
            anchors: vec![
                Anchor(PanelAnchor::Left).to_string(),
                Anchor(PanelAnchor::Right).to_string(),
                Anchor(PanelAnchor::Top).to_string(),
                Anchor(PanelAnchor::Bottom).to_string(),
            ],
            looks: vec![
                Look(PanelLook::Bar).to_string(),
                Look(PanelLook::Island).to_string(),
            ],
        }
    }
}

impl Page {
    /// The panels that have a page, for the caller to watch their configs.
    pub fn names(&self) -> Vec<String> {
        self.panels.iter().map(|panel| panel.name.clone()).collect()
    }
}

impl page::AutoBind<crate::pages::Message> for Page {}

impl page::Page<crate::pages::Message> for Page {
    fn set_id(&mut self, entity: page::Entity) {
        self.entity = entity;
    }

    fn content(
        &self,
        sections: &mut SlotMap<section::Entity, Section<crate::pages::Message>>,
    ) -> Option<page::Content> {
        // Deliberately `content()` and not a sub-page list: `SettingsApp::view` draws
        // content in preference to sub-pages, so a page with both would never show the
        // sub-page list - and the list needs a button of its own anyway.
        Some(vec![
            sections.insert(panel_list()),
            sections.insert(restore_defaults()),
        ])
    }

    fn info(&self) -> page::Info {
        page::Info::new("panels", "preferences-panel-symbolic")
            .title(fl!("panels"))
            .description(fl!("xdg-entry-panel-comment"))
    }

    fn dialog(&self) -> Option<Element<'_, crate::pages::Message>> {
        let dialog = self.dialog.as_ref()?;

        cosmic::widget::dialog()
            .title(fl!("panel-add"))
            .control(
                settings::section()
                    .add(settings::item(
                        fl!("panel-behavior-and-position", "position"),
                        dropdown(
                            self.anchors.as_slice(),
                            Some(dialog.anchor),
                            Message::DialogAnchor,
                        ),
                    ))
                    .add(settings::item(
                        fl!("panel-behavior-and-position", "display"),
                        dropdown(
                            self.outputs.as_slice(),
                            Some(dialog.output),
                            Message::DialogOutput,
                        ),
                    ))
                    .add(settings::item(
                        fl!("panel-look"),
                        dropdown(
                            self.looks.as_slice(),
                            Some(dialog.look),
                            Message::DialogLook,
                        ),
                    )),
            )
            .primary_action(
                button::suggested(fl!("panel-add", "confirm")).on_press(Message::DialogConfirm),
            )
            .secondary_action(button::standard(fl!("cancel")).on_press(Message::DialogCancel))
            .apply(Element::from)
            .map(crate::pages::Message::Panels)
            .apply(Some)
    }
}

fn panel_list() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        add = fl!("panel-add");
    });

    Section::default()
        .title(fl!("panels"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let descriptions = &section.descriptions;
            let mut list = settings::section().title(&section.title);

            for panel in &page.panels {
                list = list.add(crate::widget::go_next_item(
                    &panel.title,
                    crate::pages::Message::Page(panel.page),
                ));
            }

            list.add(
                button::standard(&descriptions[add])
                    .on_press(crate::pages::Message::Panels(Message::Add)),
            )
            .apply(Element::from)
        })
}

fn restore_defaults() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        restore = fl!("panels", "restore-defaults");
    });

    Section::default()
        .descriptions(descriptions)
        .view::<Page>(move |_binder, _page, section| {
            let descriptions = &section.descriptions;
            button::standard(&descriptions[restore])
                .on_press(Message::RestoreDefaults)
                .apply(Element::from)
                .map(crate::pages::Message::Panels)
        })
}

/// The names of the panels, in the order they are configured.
fn entries() -> Vec<String> {
    let Ok(helper) = CosmicPanelContainerConfig::cosmic_config() else {
        return Vec::new();
    };

    helper.get::<Vec<String>>("entries").unwrap_or_else(|err| {
        tracing::error!(?err, "Failed to read the list of panels.");
        Vec::new()
    })
}

/// Give every configured panel a page. Called once, after the list page is registered.
pub fn register_all(binder: &mut page::Binder<crate::pages::Message>) {
    sync(binder, &entries());
}

/// Make the pages match the panels that exist, adding and removing as needed.
fn sync(binder: &mut page::Binder<crate::pages::Message>, names: &[String]) {
    let Some(list_entity) = binder.page_id::<Page>() else {
        return;
    };

    let known: Vec<String> = binder
        .page::<Page>()
        .map(|page| page.panels.iter().map(|panel| panel.name.clone()).collect())
        .unwrap_or_default();

    for name in known.iter().filter(|name| !names.contains(name)) {
        forget(binder, name);
    }

    let (outputs, outputs_map) = binder
        .page::<Page>()
        .map(|page| (page.outputs.clone(), page.outputs_map.clone()))
        .unwrap_or_default();

    let mut panels = Vec::with_capacity(names.len());

    for name in names {
        let entity = match find(binder, &instance::page_id(name)) {
            Some(entity) => entity,
            None => {
                let mut instance = instance::Page::new(name);
                instance.adopt_outputs(&outputs, &outputs_map);
                let entity = binder.register_page(instance);
                binder.info[entity].parent = Some(list_entity);

                // Not added to `binder.sub_pages`: the panel page has its own `content()`,
                // and `SettingsApp::view` checks content before sub-pages, so a sub-page
                // list there would never be drawn. The parent is set for the breadcrumb.
                let applets = binder.register_page(applets_inner::Page::new(name));
                binder.info[applets].parent = Some(entity);
                crate::pages::applets::register_for(binder, applets);

                entity
            }
        };

        let title = binder
            .page
            .get(entity)
            .and_then(|page| page.downcast_ref::<instance::Page>())
            .map_or_else(|| name.clone(), |page| page.title().to_owned());

        panels.push(Panel {
            name: name.clone(),
            page: entity,
            title,
        });
    }

    if let Some(page) = binder.page_mut::<Page>() {
        page.panels = panels;
    }
}

/// Drop the pages of a panel that no longer exists: the panel, its applet list, and the
/// settings page of every applet on that list.
fn forget(binder: &mut page::Binder<crate::pages::Message>, name: &str) {
    let applets_id = applets_inner::page_id(name);
    let settings_prefix = format!("applet:{applets_id}:");

    let settings: Vec<page::Entity> = binder
        .info
        .iter()
        .filter(|(_, info)| info.id.starts_with(&settings_prefix))
        .map(|(entity, _)| entity)
        .collect();

    for entity in settings {
        binder.remove_page(entity);
    }

    for id in [applets_id, instance::page_id(name)] {
        if let Some(entity) = find(binder, &id) {
            binder.remove_page(entity);
        }
    }
}

fn find(binder: &page::Binder<crate::pages::Message>, id: &str) -> Option<page::Entity> {
    binder
        .info
        .iter()
        .find(|(_, info)| info.id == id)
        .map(|(entity, _)| entity)
}

/// Route a message to the page that produced it, and act on the ones that change which
/// pages exist.
pub fn update(
    binder: &mut page::Binder<crate::pages::Message>,
    message: Message,
) -> Task<crate::app::Message> {
    match message {
        Message::Surface(action) => {
            return cosmic::task::message(crate::app::Message::Surface(action));
        }

        Message::Instance { page, message } => {
            return binder
                .page
                .get_mut(page)
                .and_then(|page| page.downcast_mut::<instance::Page>())
                .map_or_else(Task::none, |page| page.update(message));
        }

        Message::Add => {
            if let Some(page) = binder.page_mut::<Page>() {
                page.dialog = Some(NewPanel::default());
            }
        }

        Message::DialogCancel => {
            if let Some(page) = binder.page_mut::<Page>() {
                page.dialog = None;
            }
        }

        Message::DialogAnchor(i) => {
            if let Some(dialog) = binder.page_mut::<Page>().and_then(|p| p.dialog.as_mut()) {
                dialog.anchor = i;
            }
        }

        Message::DialogOutput(i) => {
            if let Some(dialog) = binder.page_mut::<Page>().and_then(|p| p.dialog.as_mut()) {
                dialog.output = i;
            }
        }

        Message::DialogLook(i) => {
            if let Some(dialog) = binder.page_mut::<Page>().and_then(|p| p.dialog.as_mut()) {
                dialog.look = i;
            }
        }

        Message::DialogConfirm => {
            let Some(page) = binder.page_mut::<Page>() else {
                return Task::none();
            };

            let Some(dialog) = page.dialog.take() else {
                return Task::none();
            };

            let anchor = [
                PanelAnchor::Left,
                PanelAnchor::Right,
                PanelAnchor::Top,
                PanelAnchor::Bottom,
            ]
            .into_iter()
            .find(|a| Anchor(*a).to_string() == page.anchors[dialog.anchor])
            .unwrap_or(PanelAnchor::Top);

            let output = if dialog.output == 0 {
                CosmicPanelOuput::All
            } else {
                CosmicPanelOuput::Name(page.outputs[dialog.output].clone())
            };

            let look = if dialog.look == 1 {
                PanelLook::Island
            } else {
                PanelLook::Bar
            };

            create(anchor, output, look);
            sync(binder, &entries());
        }

        Message::Remove { page } => {
            let Some(name) = binder
                .page
                .get(page)
                .and_then(|page| page.downcast_ref::<instance::Page>())
                .map(|page| page.name().to_owned())
            else {
                return Task::none();
            };

            let names: Vec<String> = entries().into_iter().filter(|n| *n != name).collect();
            write_entries(&names);
            sync(binder, &names);

            // The page the user was standing on is gone; the list is where they came from.
            if let Some(list) = binder.page_id::<Page>() {
                return cosmic::task::message(crate::app::Message::Page(list));
            }
        }

        Message::RestoreDefaults => {
            match cosmic::cosmic_config::Config::system(
                cosmic_panel_config::NAME,
                CosmicPanelConfig::VERSION,
            )
            .map(|c| CosmicPanelContainerConfig::load_from_config(&c, true))
            {
                Ok(Ok(container)) | Ok(Err((_, container))) => {
                    if let Err(err) = container.write_entries() {
                        tracing::error!(?err, "Failed to restore the default panels.");
                    }
                }
                Err(err) => tracing::error!(?err, "No system default for the panels."),
            }

            // The shipped values are what the packages install, not what this theme asks
            // for; put the theme back over them.
            let theme = cosmic::theme::system_preference();
            let theme = theme.cosmic();
            let roundness = theme.corner_radii.into();
            crate::pages::desktop::appearance::Page::update_panel_radii(roundness);
            crate::pages::desktop::appearance::Page::update_panel_padding(roundness);
            crate::pages::desktop::appearance::Page::update_panel_spacing(
                cosmic::cosmic_theme::Density::from(theme.spacing),
            );

            sync(binder, &entries());
        }

        Message::Entries(names) => sync(binder, &names),

        Message::Config(config) => {
            let Some(entity) = find(binder, &instance::page_id(&config.name)) else {
                return Task::none();
            };

            let Some(page) = binder
                .page
                .get_mut(entity)
                .and_then(|page| page.downcast_mut::<instance::Page>())
            else {
                return Task::none();
            };

            page.config_changed(*config);
            let title = page.title().to_owned();

            binder.info[entity].title.clone_from(&title);

            if let Some(page) = binder.page_mut::<Page>()
                && let Some(panel) = page.panels.iter_mut().find(|p| p.page == entity)
            {
                panel.title = title;
            }
        }

        Message::OutputAdded(name, output) => {
            if let Some(page) = binder.page_mut::<Page>() {
                page.outputs.push(name.clone());
                page.outputs_map
                    .insert(output.id(), (name.clone(), output.clone()));
            }

            return forward_to_panels(binder, inner::Message::OutputAdded(name, output));
        }

        Message::OutputRemoved(output) => {
            if let Some(page) = binder.page_mut::<Page>()
                && let Some((name, _)) = page.outputs_map.remove(&output.id())
                && let Some(pos) = page.outputs.iter().position(|o| *o == name)
            {
                page.outputs.remove(pos);
            }

            return forward_to_panels(binder, inner::Message::OutputRemoved(output));
        }
    }

    Task::none()
}

/// Hand a message to every panel page. Displays appear and disappear for all of them at
/// once, and no one of them can be the one to hear about it.
fn forward_to_panels(
    binder: &mut page::Binder<crate::pages::Message>,
    message: inner::Message,
) -> Task<crate::app::Message> {
    let entities: Vec<page::Entity> = binder
        .page::<Page>()
        .map(|page| page.panels.iter().map(|panel| panel.page).collect())
        .unwrap_or_default();

    let tasks: Vec<_> = entities
        .into_iter()
        .filter_map(|entity| {
            binder
                .page
                .get_mut(entity)
                .and_then(|page| page.downcast_mut::<instance::Page>())
                .map(|page| page.update(message.clone()))
        })
        .collect();

    Task::batch(tasks)
}

/// Write a new panel: its own config, then its name in the list.
fn create(anchor: PanelAnchor, output: CosmicPanelOuput, look: PanelLook) {
    let mut names = entries();
    let name = free_name(&names);

    let island = look == PanelLook::Island;
    let config = CosmicPanelConfig {
        name: name.clone(),
        anchor,
        output,
        look,
        expand_to_edges: !island,
        padding: u32::from(island) * 4,
        border_radius: if island { 12 } else { 0 },
        margin: 0,
        spacing: 0,
        exclusive_zone: true,
        // A new panel starts empty: what goes on it is the next thing the user does.
        plugins_wings: Some((Vec::new(), Vec::new())),
        plugins_center: Some(Vec::new()),
        ..CosmicPanelConfig::default()
    };

    let Ok(helper) = CosmicPanelConfig::cosmic_config(&name) else {
        tracing::error!(name, "Failed to open the config of the new panel.");
        return;
    };

    if let Err(err) = config.write_entry(&helper) {
        tracing::error!(?err, name, "Failed to write the new panel.");
        return;
    }

    names.push(name);
    write_entries(&names);
}

/// `Panel` is the shipped panel, so the first name this hands out is `Panel2`.
fn free_name(taken: &[String]) -> String {
    (2..)
        .map(|n| format!("Panel{n}"))
        .find(|name| !taken.contains(name))
        .expect("an unbounded search cannot come up empty")
}

fn write_entries(names: &[String]) {
    let Ok(helper) = CosmicPanelContainerConfig::cosmic_config() else {
        return;
    };

    if let Err(err) = helper.set("entries", names.to_vec()) {
        tracing::error!(?err, "Failed to write the list of panels.");
    }
}
