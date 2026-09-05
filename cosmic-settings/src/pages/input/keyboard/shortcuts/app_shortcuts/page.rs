// SPDX-License-Identifier: GPL-3.0-only

//! One page per application that declares key bindings of its own.
//!
//! The page is built from the declaration, never from code here: an application
//! installed after Settings was built gets a working page. What it edits is the
//! application's own config file, so the change reaches a running window the same
//! way any other setting of that application does.

use super::ActionName;
use super::declaration::{self, Declaration};
use crate::pages::applets::schema::l10n;
use cosmic::iced::Length;
use cosmic::iced::keyboard::{Key, Modifiers};
use cosmic::shortcuts::{Binding, BindingSource, Shortcuts, ShortcutsConfig, binding_display};
use cosmic::widget::{self, settings};
use cosmic::{Apply, Element, Task};
use cosmic_config::{ConfigGet, ConfigSet};
use cosmic_settings_page::{self as page, Section, section};
use slotmap::{Key as SlotKey, SlotMap};

/// Prefix of the page id, so that the shortcuts page can pick these out of the binder.
pub const ID_PREFIX: &str = "app-shortcuts:";

pub struct Page {
    entity: page::Entity,
    /// Application id: the desktop entry id the declaration file is named after.
    app_id: String,
    title: String,
    icon: String,
    declaration: Declaration,
    config: Option<cosmic_config::Config>,
    shortcuts: ShortcutsConfig<ActionName>,
    /// The action waiting for a key press, if the user is setting one.
    capturing: Option<String>,
    languages: Vec<String>,
}

/// A message together with the page it is meant for.
///
/// One of these pages exists per application, so a message cannot be routed by
/// page type the way a one-off page's can.
#[derive(Clone, Debug)]
pub struct Message {
    pub entity: page::Entity,
    pub kind: Kind,
}

#[derive(Clone, Debug)]
pub enum Kind {
    /// Wait for a key press to bind to this action.
    Capture(String),
    /// Stop waiting.
    CaptureCancel,
    /// A combination was pressed while waiting.
    Captured(Binding),
    /// Take one combination away from an action.
    Remove(String, Binding),
    /// Put an action back to what the application ships.
    Reset(String),
}

/// Hands a message to the page it names.
pub fn update(
    binder: &mut page::Binder<crate::pages::Message>,
    message: Message,
) -> Task<crate::app::Message> {
    binder
        .page
        .get_mut(message.entity)
        .and_then(|page| page.downcast_mut::<Page>())
        .map_or_else(Task::none, |page| page.update(message.kind))
}

impl Page {
    pub fn new(app_id: String, declaration: Declaration, title: String, icon: String) -> Self {
        let languages = cosmic::desktop::fde::get_languages_from_env();
        let defaults = declaration.defaults();

        Self {
            entity: page::Entity::null(),
            app_id,
            title,
            icon,
            declaration,
            config: None,
            shortcuts: ShortcutsConfig::new(defaults, Shortcuts::new()),
            capturing: None,
            languages,
        }
    }

    fn update(&mut self, kind: Kind) -> Task<crate::app::Message> {
        match kind {
            Kind::Capture(action) => self.capturing = Some(action),

            Kind::CaptureCancel => self.capturing = None,

            Kind::Captured(binding) => {
                if let Some(action) = self.capturing.take() {
                    // A combination answers to one action at a time. Writing it into
                    // the user's map is what takes it away from whatever held it,
                    // default or custom.
                    self.shortcuts
                        .custom
                        .0
                        .insert(binding, ActionName::new(action));
                    self.save();
                }
            }

            Kind::Remove(action, binding) => {
                match self.source_of(&action, &binding) {
                    // A shipped combination cannot be deleted, only shadowed.
                    Some(BindingSource::Default) => {
                        self.shortcuts
                            .custom
                            .0
                            .insert(binding, ActionName::disable());
                    }
                    _ => {
                        self.shortcuts.custom.0.remove(&binding);
                    }
                }
                self.save();
            }

            Kind::Reset(action) => {
                self.shortcuts.reset_action(&ActionName::new(action));
                self.save();
            }
        }

        Task::none()
    }

    fn source_of(&self, action: &str, binding: &Binding) -> Option<BindingSource> {
        let name = ActionName::new(action);
        self.shortcuts
            .bindings_for_action(&name)
            .0
            .into_iter()
            .find(|resolved| &resolved.binding == binding)
            .map(|resolved| resolved.source)
    }

    fn load(&mut self) {
        let target = &self.declaration.config;
        if self.config.is_none() {
            self.config = cosmic_config::Config::new(&target.id, target.version).ok();
        }

        let custom = self
            .config
            .as_ref()
            .and_then(|config| config.get::<Shortcuts<ActionName>>(&target.key).ok())
            .unwrap_or_default();

        self.shortcuts = ShortcutsConfig::new(self.declaration.defaults(), custom);
    }

    fn save(&mut self) {
        let Some(config) = self.config.as_ref() else {
            tracing::error!(app = %self.app_id, "no config handler for the application");
            return;
        };

        if let Err(why) = config.set(&self.declaration.config.key, &self.shortcuts.custom) {
            tracing::error!(?why, app = %self.app_id, "failed to save the application shortcuts");
        }
    }

    fn text(&self, label: &crate::pages::applets::schema::model::L10n) -> String {
        l10n::resolve(label, &self.languages)
    }
}

impl page::Page<crate::pages::Message> for Page {
    fn set_id(&mut self, entity: page::Entity) {
        self.entity = entity;
    }

    fn info(&self) -> page::Info {
        page::Info::new(format!("{ID_PREFIX}{}", self.app_id), self.icon.clone())
            .title(self.title.clone())
    }

    fn content(
        &self,
        sections: &mut SlotMap<section::Entity, Section<crate::pages::Message>>,
    ) -> Option<page::Content> {
        Some(
            (0..self.declaration.groups.len())
                .map(|index| sections.insert(group_section(index)))
                .collect(),
        )
    }

    fn on_enter(&mut self) -> Task<crate::pages::Message> {
        self.load();
        Task::none()
    }

    fn on_leave(&mut self) -> Task<crate::pages::Message> {
        self.capturing = None;
        Task::none()
    }

    fn subscription(
        &self,
        _core: &cosmic::Core,
    ) -> cosmic::iced::Subscription<crate::pages::Message> {
        if self.capturing.is_none() {
            return cosmic::iced::Subscription::none();
        }

        // Only while a row is waiting: this listens to every key press, and the rest
        // of the time the page has no business seeing them.
        //
        // Neither `listen_with` nor `map` accepts a closure that captures anything -
        // `map` enforces it in a const block, so a capturing one compiles under
        // `cargo check` and only fails during codegen. The page this belongs to
        // therefore travels through `with`, which carries the value alongside the
        // stream instead.
        cosmic::iced::event::listen_with(|event, _, _| match event {
            cosmic::iced::event::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                key,
                modifiers,
                ..
            }) => captured(modifiers, &key),
            _ => None,
        })
        .with(self.entity)
        .map(|(entity, kind)| crate::pages::Message::AppShortcuts(Message { entity, kind }))
    }
}

/// Turns a key press into either a binding, or a request to stop waiting.
fn captured(modifiers: Modifiers, key: &Key) -> Option<Kind> {
    use cosmic::iced::keyboard::key::Named;

    if matches!(key, Key::Named(Named::Escape)) {
        return Some(Kind::CaptureCancel);
    }

    cosmic::shortcuts::binding_from_key(modifiers, key).map(Kind::Captured)
}

fn group_section(index: usize) -> Section<crate::pages::Message> {
    Section::default().view::<Page>(move |_binder, page, _section| {
        let Some(group) = page.declaration.groups.get(index) else {
            return widget::column::with_capacity(0).into();
        };

        let mut list = settings::section().title(page.text(&group.title));

        for action in &group.actions {
            let name = ActionName::new(&action.action);
            let (bindings, changed) = page.shortcuts.bindings_for_action(&name);
            let waiting = page.capturing.as_deref() == Some(action.action.as_str());

            let mut controls = widget::row::with_capacity(3).spacing(8);

            if waiting {
                controls = controls.push(widget::text::body(fl!("app-shortcuts", "press")));
            } else {
                for resolved in &bindings {
                    let binding = resolved.binding.clone();
                    controls = controls.push(
                        widget::button::text(binding_display(&binding))
                            .trailing_icon(
                                widget::icon::from_name("window-close-symbolic").size(12),
                            )
                            .on_press(crate::pages::Message::AppShortcuts(Message {
                                entity: page.entity,
                                kind: Kind::Remove(action.action.clone(), binding),
                            })),
                    );
                }
            }

            controls = controls.push(
                widget::button::standard(if waiting {
                    fl!("cancel")
                } else {
                    fl!("app-shortcuts", "add")
                })
                .on_press(crate::pages::Message::AppShortcuts(Message {
                    entity: page.entity,
                    kind: if waiting {
                        Kind::CaptureCancel
                    } else {
                        Kind::Capture(action.action.clone())
                    },
                })),
            );

            if changed {
                // WMDE: appears only for a changed shortcut, and an undo arrow reads as
                // "undo my typing" unless it says it restores the default.
                controls = controls.push(widget::tooltip(
                    widget::button::icon(widget::icon::from_name("edit-undo-symbolic").size(16))
                        .on_press(crate::pages::Message::AppShortcuts(Message {
                            entity: page.entity,
                            kind: Kind::Reset(action.action.clone()),
                        })),
                    widget::text::body(fl!("app-shortcuts", "reset")),
                    widget::tooltip::Position::Top,
                ));
            }

            list = list.add(settings::item(
                page.text(&action.label),
                controls.apply(widget::container).width(Length::Shrink),
            ));
        }

        list.apply(Element::from)
    })
}

/// Registers a page for every declaration installed.
///
/// Called after the shortcuts page exists, because these hang off it: how many
/// there are is a matter of what is installed, so they cannot be registered by type.
pub fn register_all(binder: &mut page::Binder<crate::pages::Message>, parent: page::Entity) {
    let languages = cosmic::desktop::fde::get_languages_from_env();
    let applications: Vec<_> = cosmic::desktop::load_applications(&languages, true, None).collect();

    for (app_id, declaration) in declaration::load_all() {
        let entry = applications.iter().find(|app| app.id == app_id);
        let title = entry.map_or_else(|| app_id.clone(), |app| app.name.clone());
        let icon = entry
            .and_then(|app| match &app.icon {
                cosmic::desktop::fde::IconSource::Name(name) => Some(name.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| String::from("input-keyboard-symbolic"));

        let child = binder.register_page(Page::new(app_id, declaration, title, icon));
        binder.info[child].parent = Some(parent);
    }
}
