// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: the settings of one panel.
//!
//! Every panel gets an instance of this one type, registered at runtime with
//! `Binder::register_page`. Two consequences, both of them quiet when got wrong:
//!
//! * These pages are not in `typed_page_ids`, so `page_mut::<Page>()` returns `None` for
//!   them. Messages carry the entity of the page they came from and [`super::update`]
//!   dispatches on it. Routing on the active page instead would misdeliver during a search,
//!   where sections of several pages are drawn at once - and would write into a
//!   neighbouring panel's config with nothing to show for it.
//! * `content()` runs before `set_id()`, so a section closure must read the entity from the
//!   page it is handed at draw time, never capture it.

use cosmic::cctk::sctk::reexports::client::backend::ObjectId;
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::widget::button;
use cosmic::{Apply, Element, Task};
use cosmic_panel_config::{CosmicPanelConfig, CosmicPanelOuput};
use cosmic_settings_page::{self as page, Section, section};
use slotmap::{Key, SlotMap};
use std::collections::HashMap;

use super::applets_inner;
use super::inner::{
    self, Anchor, PageInner, PanelPage, behavior_and_position, configuration, reset_button, style,
};

pub struct Page {
    entity: page::Entity,
    /// The panel this page edits. Also its config key, `fun.wmde.Panel.<name>`.
    name: String,
    /// Where the panel sits, as shown in the header and in the panel list. Recomputed
    /// whenever the config changes, because both halves of it are settings on this page.
    title: String,
    inner: PageInner,
}

impl Page {
    pub fn new(name: &str) -> Self {
        let config_helper = CosmicPanelConfig::cosmic_config(name).ok();

        let panel_config = config_helper.as_ref().and_then(|config_helper| {
            let panel_config = CosmicPanelConfig::get_entry(config_helper).ok()?;
            // A missing config is created with default values, and its name will not match.
            (panel_config.name == name).then_some(panel_config)
        });

        let system_default = cosmic::cosmic_config::Config::system(
            &format!("{}.{name}", cosmic_panel_config::NAME),
            CosmicPanelConfig::VERSION,
        )
        .map(|c| match CosmicPanelConfig::get_entry(&c) {
            Ok(c) => c,
            Err((errs, c)) => {
                for err in errs.into_iter().filter(cosmic_config::Error::is_err) {
                    tracing::error!(?err, name, "Failed to load panel system config.");
                }
                c
            }
        })
        .ok();

        let title = panel_config
            .as_ref()
            .map_or_else(|| name.to_owned(), title_of);

        Self {
            entity: page::Entity::null(),
            name: name.to_owned(),
            title,
            inner: PageInner {
                config_helper,
                panel_config,
                outputs_map: HashMap::new(),
                system_default,
                ..Default::default()
            },
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Take the outputs another panel page already learned about, so that a page created
    /// after startup still offers the full list of displays.
    pub fn adopt_outputs(
        &mut self,
        outputs: &[String],
        map: &HashMap<ObjectId, (String, WlOutput)>,
    ) {
        self.inner.outputs = outputs.to_vec();
        self.inner.outputs_map = map.clone();
    }

    pub fn update(&mut self, message: inner::Message) -> Task<crate::app::Message> {
        if let inner::Message::Surface(a) = message {
            return cosmic::task::message(crate::app::Message::Surface(a));
        }

        let entity = self.entity;
        let task = self
            .inner
            .update(message)
            .map(move |m| super::message(entity, m))
            .map(crate::app::Message::PageMessage);

        self.retitle();

        task
    }

    /// Follow a config the panel wrote behind our back.
    pub fn config_changed(&mut self, config: CosmicPanelConfig) {
        if config.name != self.name {
            return;
        }

        self.inner.size = config.size.clone();
        self.inner.panel_config = Some(config);
        self.retitle();
    }

    fn retitle(&mut self) {
        if let Some(config) = self.inner.panel_config.as_ref() {
            self.title = title_of(config);
        }
    }
}

/// Where a panel sits, in the words the settings use for it: "Bottom, all displays".
pub fn title_of(config: &CosmicPanelConfig) -> String {
    let display = match &config.output {
        CosmicPanelOuput::All => fl!("all-displays"),
        CosmicPanelOuput::Active => fl!("panel-active-display"),
        CosmicPanelOuput::Name(name) => name.clone(),
    };

    format!("{}, {display}", Anchor(config.anchor))
}

impl PanelPage for Page {
    fn inner(&self) -> &PageInner {
        &self.inner
    }

    fn inner_mut(&mut self) -> &mut PageInner {
        &mut self.inner
    }

    fn entity(&self) -> page::Entity {
        self.entity
    }

    fn autohide_label(&self) -> String {
        fl!("panel-behavior-and-position", "autohide")
    }

    fn gap_label(&self) -> String {
        fl!("panel-style", "anchor-gap")
    }

    fn extend_label(&self) -> String {
        fl!("panel-style", "extend")
    }

    fn configure_applets_label(&self) -> String {
        fl!("panel-applets", "desc")
    }

    fn applets_page_id(&self) -> String {
        applets_inner::page_id(&self.name)
    }
}

impl page::Page<crate::pages::Message> for Page {
    fn set_id(&mut self, entity: page::Entity) {
        self.entity = entity;
    }

    fn content(
        &self,
        sections: &mut SlotMap<section::Entity, Section<crate::pages::Message>>,
    ) -> Option<page::Content> {
        Some(vec![
            sections.insert(behavior_and_position::<Page, _>(self, super::message)),
            sections.insert(style::<Page, _>(self, super::message)),
            sections.insert(configuration::<Page>(self)),
            sections.insert(reset_button::<Page, _>(super::message)),
            sections.insert(remove_button()),
        ])
    }

    fn info(&self) -> page::Info {
        page::Info::new(page_id(&self.name), "preferences-panel-symbolic").title(self.title.clone())
    }

    fn title(&self) -> Option<&str> {
        Some(&self.title)
    }

    fn on_enter(&mut self) -> Task<crate::pages::Message> {
        self.inner.update_defaults();

        Task::none()
    }
}

/// `Info::id` of a panel's page. Stable across runs, so that reopening Settings returns to
/// the page it was left on.
pub fn page_id(panel: &str) -> String {
    format!("panel:{panel}")
}

fn remove_button() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        remove = fl!("panel-remove");
    });

    Section::default()
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let descriptions = &section.descriptions;
            button::destructive(&descriptions[remove])
                .on_press(super::Message::Remove { page: page.entity })
                .apply(Element::from)
                .map(crate::pages::Message::Panels)
        })
}
