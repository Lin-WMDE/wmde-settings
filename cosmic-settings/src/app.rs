// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use crate::PageCommands;
use crate::config::Config;
#[cfg(feature = "page-accessibility")]
use crate::pages::accessibility;
#[cfg(feature = "page-bluetooth")]
use crate::pages::bluetooth;
use crate::pages::desktop::{self, appearance};
#[cfg(feature = "page-display")]
use crate::pages::display;
#[cfg(feature = "page-input")]
use crate::pages::input;
#[cfg(feature = "page-networking")]
use crate::pages::networking;
#[cfg(feature = "page-power")]
use crate::pages::power;
#[cfg(feature = "page-sound")]
use crate::pages::sound;
use crate::pages::{self, applications, system, time};
use crate::subscription::desktop_files;
use crate::widget::{page_title, search_header};
use cosmic::app::context_drawer::ContextDrawer;
use cosmic::app::{Core, Task};
#[cfg(feature = "wayland")]
use cosmic::cctk::{sctk::output::OutputInfo, wayland_client::protocol::wl_output::WlOutput};
use cosmic::iced::event::{self, PlatformSpecific};
use cosmic::iced::{self, Length, Subscription, keyboard, window};
use cosmic::prelude::*;
use cosmic::widget::{
    button, column, container, icon, id_container, nav_bar, row, scrollable, segmented_button,
    settings, text_input,
};
use cosmic::{Element, surface};
#[cfg(feature = "cosmic-comp-config")]
use cosmic_comp_config::CosmicCompConfig;
#[cfg(feature = "wayland")]
use cosmic_panel_config::CosmicPanelConfig;
use cosmic_settings_page::{self as page, section};
#[cfg(feature = "wayland")]
use desktop::panel::{self, applets_inner};
#[cfg(feature = "wayland")]
use event::wayland;
use page::Entity;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::str::FromStr;

#[allow(clippy::struct_excessive_bools)]
#[allow(clippy::module_name_repetitions)]
pub struct SettingsApp {
    last_active_page: Box<str>,
    active_page: page::Entity,
    active_context_page: Option<page::Entity>,
    loaded_pages: BTreeSet<page::Entity>,
    config: Config,
    core: Core,
    nav_model: nav_bar::Model,
    pages: page::Binder<crate::pages::Message>,
    search_active: bool,
    search_id: cosmic::widget::Id,
    search_input: String,
    search_selections: Vec<(page::Entity, section::Entity)>,
    context_title: Option<String>,
}

impl SettingsApp {
    fn subtask_to_page(&self, cmd: &PageCommands) -> Option<Entity> {
        match cmd {
            #[cfg(feature = "page-accessibility")]
            PageCommands::Accessibility => self.pages.page_id::<accessibility::Page>(),
            #[cfg(feature = "page-accessibility")]
            PageCommands::AccessibilityMagnifier => {
                self.pages.page_id::<accessibility::magnifier::Page>()
            }
            #[cfg(feature = "page-about")]
            PageCommands::About => self.pages.page_id::<system::about::Page>(),
            PageCommands::Appearance { command: _ } => {
                self.pages.page_id::<desktop::appearance::Page>()
            }
            PageCommands::Applications => self.pages.page_id::<applications::Page>(),
            #[cfg(feature = "page-bluetooth")]
            PageCommands::Bluetooth => self.pages.page_id::<bluetooth::Page>(),
            #[cfg(feature = "page-date")]
            PageCommands::DateTime => self.pages.page_id::<time::date::Page>(),
            #[cfg(feature = "page-default-apps")]
            PageCommands::DefaultApps => self.pages.page_id::<applications::default_apps::Page>(),
            PageCommands::Desktop => self.pages.page_id::<desktop::Page>(),
            #[cfg(feature = "page-display")]
            PageCommands::Displays => self.pages.page_id::<display::Page>(),
            #[cfg(feature = "page-input")]
            PageCommands::Input => self.pages.page_id::<input::Page>(),
            #[cfg(feature = "page-input")]
            PageCommands::Keyboard => self.pages.page_id::<input::keyboard::Page>(),
            #[cfg(feature = "page-legacy-applications")]
            PageCommands::LegacyApplications => self
                .pages
                .page_id::<applications::legacy_applications::Page>(),
            #[cfg(feature = "page-input")]
            PageCommands::Mouse => self.pages.page_id::<input::mouse::Page>(),
            #[cfg(feature = "page-networking")]
            PageCommands::Network => self.pages.page_id::<networking::Page>(),
            // The list of panels. A single panel's page is registered at runtime and has no
            // type to name here.
            #[cfg(feature = "wayland")]
            PageCommands::Panel => self.pages.page_id::<desktop::panel::Page>(),
            #[cfg(feature = "page-power")]
            PageCommands::Power => self.pages.page_id::<power::Page>(),
            #[cfg(feature = "page-region")]
            PageCommands::RegionLanguage => self.pages.page_id::<time::region::Page>(),
            #[cfg(feature = "page-sound")]
            PageCommands::Sound => self.pages.page_id::<sound::Page>(),
            PageCommands::StartupApps => self.pages.page_id::<applications::startup_apps::Page>(),
            PageCommands::System => self.pages.page_id::<system::Page>(),
            PageCommands::Time => self.pages.page_id::<time::Page>(),
            #[cfg(feature = "page-input")]
            PageCommands::Touchpad => self.pages.page_id::<input::touchpad::Page>(),
            #[cfg(feature = "page-users")]
            PageCommands::Users => self.pages.page_id::<system::users::Page>(),
            #[cfg(feature = "page-networking")]
            PageCommands::Vpn => self.pages.page_id::<networking::vpn::Page>(),
            PageCommands::Wallpaper => self.pages.page_id::<desktop::wallpaper::Page>(),
            #[cfg(feature = "page-window-management")]
            PageCommands::WindowManagement => {
                self.pages.page_id::<desktop::window_management::Page>()
            }
            #[cfg(feature = "page-networking")]
            PageCommands::Wired => self.pages.page_id::<networking::wired::Page>(),
            #[cfg(feature = "page-networking")]
            PageCommands::Wireless => self.pages.page_id::<networking::wifi::Page>(),
            #[cfg(feature = "page-workspaces")]
            PageCommands::Workspaces => self.pages.page_id::<desktop::workspaces::Page>(),
        }
    }

    fn id(&self) -> cosmic::iced::id::Id {
        let cur_page_name = self.pages.info[self.active_page].id.as_ref();
        cosmic::iced::id::Id::new(cur_page_name.to_owned())
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    #[cfg(feature = "wayland")]
    DesktopInfo,
    Error(String),
    None,
    OpenContextDrawer(Entity),
    #[cfg(feature = "wayland")]
    OutputAdded(OutputInfo, WlOutput),
    #[cfg(feature = "wayland")]
    OutputRemoved(WlOutput),
    Page(page::Entity),
    PageMessage(crate::pages::Message),
    #[cfg(feature = "wayland")]
    PanelConfig(Box<CosmicPanelConfig>),
    /// WMDE: the set of panels changed, so the pages that stand for them must follow.
    #[cfg(feature = "wayland")]
    PanelEntries(Vec<String>),
    #[cfg(feature = "cosmic-comp-config")]
    CompConfig(Box<CosmicCompConfig>),
    SearchActivate,
    SearchActivateWith(String),
    SearchChanged(String),
    SearchClear,
    SearchSubmit,
    SetTheme(cosmic::theme::Theme),
    SetWindowTitle,
    Surface(surface::Action),
}

impl cosmic::Application for SettingsApp {
    type Executor = cosmic::executor::single::Executor;
    type Flags = crate::Args;
    type Message = Message;

    const APP_ID: &'static str = "fun.wmde.Settings";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let config = Config::new();

        let mut app = SettingsApp {
            active_page: page::Entity::default(),
            active_context_page: None,
            last_active_page: config.active_page(),
            loaded_pages: BTreeSet::new(),
            config,
            core,
            nav_model: nav_bar::Model::default(),
            pages: page::Binder::default(),
            search_active: false,
            search_id: cosmic::widget::Id::unique(),
            search_input: String::new(),
            search_selections: Vec::default(),
            context_title: None,
        };

        // WMDE: About and Hardware are top-level pages, and About is first - the app
        // opens on it. Order of these calls IS the order of the nav bar.
        #[cfg(feature = "page-about")]
        app.insert_page::<system::about::Page>();
        #[cfg(feature = "page-about")]
        app.insert_page::<system::hardware::Page>();
        #[cfg(feature = "page-networking")]
        app.insert_page::<networking::Page>();
        #[cfg(feature = "page-bluetooth")]
        app.insert_page::<bluetooth::Page>();
        #[cfg(feature = "page-accessibility")]
        app.insert_page::<accessibility::Page>();
        let desktop_id = app.insert_page::<desktop::Page>().id();
        // WMDE: panels are a top-level entry, not a sub-page of the desktop. There can be
        // any number of them, and each one is a page of its own hanging off this list.
        #[cfg(feature = "wayland")]
        app.insert_page::<panel::Page>();
        #[cfg(feature = "page-display")]
        app.insert_page::<display::Page>();
        #[cfg(feature = "page-sound")]
        app.insert_page::<sound::Page>();
        #[cfg(feature = "page-power")]
        app.insert_page::<power::Page>();
        #[cfg(feature = "page-input")]
        app.insert_page::<input::Page>();
        // WMDE: shortcuts get their own entry instead of hiding under the keyboard.
        // They reach past the keyboard - the compositor runs some, applications run
        // the rest - so a user looking for them has no reason to open input devices.
        #[cfg(feature = "page-input")]
        let shortcuts_id = app.insert_page::<input::keyboard::shortcuts::Page>().id();
        app.insert_page::<applications::Page>();
        app.insert_page::<time::Page>();
        app.insert_page::<system::Page>();

        // WMDE: a page per configured panel, and per panel a page for its applets. How
        // many there are is a user's decision, so these cannot be registered by type.
        // WMDE: a page per application that declares key bindings of its own. What is
        // installed decides how many there are, so these cannot be registered by type.
        #[cfg(feature = "page-input")]
        input::keyboard::shortcuts::app_shortcuts::register_all(&mut app.pages, shortcuts_id);

        #[cfg(feature = "wayland")]
        panel::register_all(&mut app.pages);

        // WMDE: the settings pages of the applets themselves are registered per panel, by
        // `panel::register_all` above. This only reports a schema that matches nothing.
        #[cfg(feature = "wayland")]
        pages::applets::warn_orphan_schemas(&app.pages);

        // WMDE: always open on About. The remembered page survives only as a fallback for
        // a build without `page-about`, which also keeps `last_active_page` a field that
        // is still read rather than dead weight. An explicit CLI page still wins.
        #[cfg(feature = "page-about")]
        let landing_id = app.pages.page_id::<system::about::Page>();
        #[cfg(not(feature = "page-about"))]
        let landing_id: Option<Entity> = None;

        let active_id = match flags.sub_command {
            Some(p) => app.subtask_to_page(&p),
            None => landing_id.or_else(|| {
                app.pages
                    .find_page_by_id(&app.last_active_page)
                    .map(|(id, _info)| id)
            }),
        }
        .unwrap_or(desktop_id);

        let task = Task::batch([
            cosmic::command::set_theme(cosmic::theme::system_preference()),
            app.activate_page(active_id),
        ]);
        (app, task)
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav_model)
    }

    /// WMDE: the same navigation list the framework builds, with our own item style.
    ///
    /// This is a copy of the default `nav_bar()` (`libcosmic/src/app/mod.rs`) plus the body
    /// of `From<NavBar> for Container` (`libcosmic/src/widget/nav_bar.rs`), and it exists
    /// for exactly one reason: that `From` impl hardcodes
    /// `.style(theme::SegmentedButton::NavBar)` and `NavBar` exposes no way to override it.
    /// Everything else here - the 32 px rows, the paddings, the scrollable, the 280 px cap
    /// off the condensed mode - is upstream's, copied verbatim. Keep it that way: if the
    /// navigation ever looks wrong after a merge, diff it against those two places first.
    fn nav_bar(&self) -> Option<Element<'_, cosmic::Action<Self::Message>>> {
        if !self.core().nav_bar_active() {
            return None;
        }

        let nav_model = self.nav_model()?;
        let spacing = cosmic::theme::spacing();

        let nav = segmented_button::vertical(nav_model)
            .on_activate(|id| cosmic::Action::Cosmic(cosmic::app::Action::NavBar(id)))
            .on_context(|id| cosmic::Action::Cosmic(cosmic::app::Action::NavBarContext(id)))
            .context_menu(self.nav_context_menu())
            .window_id_maybe(self.core().main_window_id())
            .on_surface_action(|action| {
                cosmic::Action::Cosmic(cosmic::app::Action::Surface(action))
            })
            .button_height(32)
            .button_padding([
                spacing.space_s,
                spacing.space_xxs,
                spacing.space_s,
                spacing.space_xxs,
            ])
            .button_spacing(spacing.space_xxs)
            .spacing(spacing.space_xxs)
            .style(nav_item_style())
            .apply(container)
            .padding(spacing.space_xxs)
            .apply(scrollable)
            .class(cosmic::style::iced::Scrollable::Minimal)
            .height(Length::Fill)
            .apply(container)
            .height(Length::Fill)
            .class(cosmic::theme::Container::custom(nav_bar::nav_bar_style))
            .width(Length::Shrink);

        let nav = if self.core().is_condensed() {
            nav
        } else {
            nav.max_width(280)
        };

        Some(Element::from(nav))
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let mut widgets = Vec::new();

        widgets.push(if self.search_active {
            text_input::search_input("", &self.search_input)
                .width(Length::Fixed(240.0))
                .id(self.search_id.clone())
                .on_clear(Message::SearchClear)
                .on_input(Message::SearchChanged)
                .on_submit(|_| Message::SearchSubmit)
                .into()
        } else {
            icon::from_name("system-search-symbolic")
                .apply(button::icon)
                .padding(8)
                .on_press(Message::SearchActivate)
                .into()
        });

        widgets
    }

    fn on_app_exit(&mut self) -> Option<Self::Message> {
        self.pages.on_leave(self.active_page);
        None
    }

    fn on_escape(&mut self) -> Task<Self::Message> {
        if self.search_active {
            self.search_active = false;
            self.search_clear();
        }

        Task::none()
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<Self::Message> {
        if let Some(page) = self.nav_model.data::<page::Entity>(id).copied() {
            return self.activate_page(page);
        }

        Task::none()
    }

    fn on_search(&mut self) -> Task<Self::Message> {
        self.search_active = true;
        cosmic::widget::text_input::focus(self.search_id.clone())
    }

    fn subscription(&self) -> Subscription<Message> {
        let page = &self.pages.page[self.active_page];

        let subscriptions = vec![
            #[cfg(feature = "ashpd")]
            crate::subscription::daytime().map(|daytime| {
                Message::PageMessage(pages::Message::Appearance(appearance::Message::Daytime(
                    daytime,
                )))
            }),
            event::listen_with(|event, status, _id| match event {
                #[cfg(feature = "wayland")]
                iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::Output(wayland::OutputEvent::Created(Some(info)), o),
                )) if info.name.is_some() => Some(Message::OutputAdded(info, o)),
                #[cfg(feature = "wayland")]
                iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::Output(wayland::OutputEvent::Removed, o),
                )) => Some(Message::OutputRemoved(o)),
                iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Character(c),
                    modifiers: m,
                    ..
                }) if status == event::Status::Ignored && !m.control() && !m.alt() && !m.logo() => {
                    Some(Message::SearchActivateWith(c.to_string()))
                }
                _ => None,
            }),
            #[cfg(feature = "wayland")]
            // Watch for changes to installed desktop entries
            desktop_files(0).map(|_| Message::DesktopInfo),
            // Watch which panels exist, and then each of them. Not a fixed pair of names
            // any more: the user decides how many there are, so the set of things to watch
            // is itself a setting.
            #[cfg(feature = "wayland")]
            self.core()
                .watch_config::<cosmic_panel_config::CosmicPanelContainerConfigEntry>(
                    cosmic_panel_config::NAME,
                )
                .map(|update| {
                    for why in update.errors {
                        tracing::error!(?why, "panel entries load error");
                    }

                    Message::PanelEntries(update.config.entries)
                }),
            // Not `watch_config`: that wants a `&'static str`, and a panel's config id is
            // built from a name the user chose.
            #[cfg(feature = "wayland")]
            Subscription::batch(
                self.pages
                    .page::<panel::Page>()
                    .map(panel::Page::names)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|name| {
                        cosmic::cosmic_config::config_subscription::<_, CosmicPanelConfig>(
                            name.clone(),
                            Cow::Owned(format!("{}.{name}", cosmic_panel_config::NAME)),
                            cosmic_panel_config::VERSION,
                        )
                        .map(|update| {
                            for why in update.errors {
                                tracing::error!(?why, "panel config load error");
                            }

                            Message::PanelConfig(Box::new(update.config))
                        })
                    }),
            ),
            page.subscription(self.core()).map(Message::PageMessage),
            #[cfg(feature = "cosmic-comp-config")]
            self.core()
                .watch_config::<CosmicCompConfig>(crate::config::COSMIC_COMP_CONFIG)
                .map(|update| {
                    for why in update.errors {
                        tracing::error!(?why, "comp config load error");
                    }

                    Message::CompConfig(Box::new(update.config))
                }),
        ];

        Subscription::batch(subscriptions)
    }

    #[allow(clippy::too_many_lines)]
    fn update(&mut self, message: Message) -> Task<Self::Message> {
        match message {
            Message::Page(page) => return self.activate_page(page),

            Message::None => (),

            Message::SetWindowTitle => return self.set_title(),

            Message::SearchChanged(phrase) => {
                return self.search_changed(phrase);
            }

            Message::SearchActivate => return self.on_search(),

            Message::SearchActivateWith(phrase) => {
                return self.on_search().chain(self.search_changed(phrase));
            }

            Message::SearchClear => {
                self.search_active = false;
                self.search_clear();
            }

            Message::SearchSubmit => {
                self.search_active = true;
            }

            Message::PageMessage(message) => match message {
                crate::pages::Message::CloseContextDrawer => return self.close_context_drawer(),

                #[cfg(feature = "page-accessibility")]
                crate::pages::Message::Accessibility(message) => {
                    if let Some(page) = self.pages.page_mut::<accessibility::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }
                #[cfg(feature = "page-about")]
                crate::pages::Message::About(message) => {
                    if let Some(page) = self.pages.page_mut::<system::about::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }
                // WMDE: the Hardware sub-page.
                #[cfg(feature = "page-about")]
                crate::pages::Message::Hardware(message) => {
                    if let Some(page) = self.pages.page_mut::<system::hardware::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }
                // WMDE: applet settings. Routed by the entity in the message rather than
                // by page type: there is one page per applet, all of the same type, so
                // `page_mut::<P>()` cannot tell them apart.
                #[cfg(feature = "wayland")]
                crate::pages::Message::AppletSettings(message) => {
                    return pages::applets::update(&mut self.pages, message).map(Into::into);
                }
                // WMDE: one of these pages exists per application, so the message
                // names its page rather than being routed by page type.
                #[cfg(feature = "page-input")]
                crate::pages::Message::AppShortcuts(message) => {
                    return input::keyboard::shortcuts::app_shortcuts::update(
                        &mut self.pages,
                        message,
                    )
                    .map(Into::into);
                }
                #[cfg(feature = "page-accessibility")]
                crate::pages::Message::AccessibilityMagnifier(message) => {
                    if let Some(page) = self.pages.page_mut::<accessibility::magnifier::Page>() {
                        return page.update(self.active_page, message).map(Into::into);
                    }
                }
                #[cfg(feature = "page-input")]
                crate::pages::Message::AccessibilityShortcuts(message) => {
                    if let Some(page) = self
                        .pages
                        .page_mut::<input::keyboard::shortcuts::accessibility::Page>()
                    {
                        return page.update(message).map(Into::into);
                    }
                }
                crate::pages::Message::Appearance(message) => {
                    if let Some(page) = self.pages.page_mut::<appearance::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                crate::pages::Message::Applications(message) => {
                    page::update!(self.pages, message, applications::Page);
                }

                #[cfg(feature = "page-bluetooth")]
                crate::pages::Message::Bluetooth(message) => {
                    if let Some(page) = self.pages.page_mut::<bluetooth::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-date")]
                crate::pages::Message::DateAndTime(message) => {
                    if let Some(page) = self.pages.page_mut::<time::date::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-default-apps")]
                crate::pages::Message::DefaultApps(message) => {
                    if let Some(page) = self.pages.page_mut::<applications::default_apps::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                crate::pages::Message::Desktop(message) => {
                    page::update!(self.pages, message, desktop::Page);
                }

                crate::pages::Message::DesktopWallpaper(message) => {
                    if let Some(page) = self.pages.page_mut::<desktop::wallpaper::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-workspaces")]
                crate::pages::Message::DesktopWorkspaces(message) => {
                    page::update!(self.pages, message, desktop::workspaces::Page);
                }

                #[cfg(feature = "page-display")]
                crate::pages::Message::Displays(message) => {
                    if let Some(page) = self.pages.page_mut::<display::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::Input(message) => {
                    if let Some(page) = self.pages.page_mut::<input::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::Keyboard(message) => {
                    if let Some(page) = self.pages.page_mut::<input::keyboard::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::KeyboardShortcuts(message) => {
                    if let Some(page) = self.pages.page_mut::<input::keyboard::shortcuts::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::CustomShortcuts(message) => {
                    if let Some(page) = self
                        .pages
                        .page_mut::<input::keyboard::shortcuts::custom::Page>()
                    {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-legacy-applications")]
                crate::pages::Message::LegacyApplications(message) => {
                    if let Some(page) = self
                        .pages
                        .page_mut::<applications::legacy_applications::Page>()
                    {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::ManageWindowShortcuts(message) => {
                    if let Some(page) = self
                        .pages
                        .page_mut::<input::keyboard::shortcuts::manage_windows::Page>()
                    {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::MoveWindowShortcuts(message) => {
                    if let Some(page) = self
                        .pages
                        .page_mut::<input::keyboard::shortcuts::move_window::Page>()
                    {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::NavShortcuts(message) => {
                    if let Some(page) = self
                        .pages
                        .page_mut::<input::keyboard::shortcuts::nav::Page>()
                    {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-region")]
                crate::pages::Message::Region(message) => {
                    if let Some(page) = self.pages.page_mut::<time::region::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-sound")]
                crate::pages::Message::Sound(message) => {
                    if let Some(page) = self.pages.page_mut::<sound::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-sound")]
                crate::pages::Message::SoundDeviceProfiles(message) => {
                    if let Some(page) = self.pages.page_mut::<sound::device_profiles::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                crate::pages::Message::StartupApps(message) => {
                    if let Some(page) = self.pages.page_mut::<applications::startup_apps::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-users")]
                crate::pages::Message::User(message) => {
                    if let Some(page) = self.pages.page_mut::<system::users::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::SystemShortcuts(message) => {
                    if let Some(page) = self
                        .pages
                        .page_mut::<input::keyboard::shortcuts::system::Page>()
                    {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-input")]
                crate::pages::Message::TilingShortcuts(message) => {
                    if let Some(page) = self
                        .pages
                        .page_mut::<input::keyboard::shortcuts::tiling::Page>()
                    {
                        return page.update(message).map(Into::into);
                    }
                }

                crate::pages::Message::External { .. } => {
                    todo!("external plugins not supported yet");
                }

                crate::pages::Message::Page(page) => {
                    return self.activate_page(page);
                }

                #[cfg(feature = "page-networking")]
                crate::pages::Message::Networking(message) => {
                    if let Some(page) = self.pages.page_mut::<networking::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "wayland")]
                crate::pages::Message::Panels(message) => {
                    return panel::update(&mut self.pages, message).map(Into::into);
                }

                // Routed by the entity the message carries, not by type: there is one of
                // these pages per panel.
                #[cfg(feature = "wayland")]
                crate::pages::Message::PanelApplet { page, message } => {
                    if let Some(page) = self
                        .pages
                        .page
                        .get_mut(page)
                        .and_then(|page| page.downcast_mut::<applets_inner::Page>())
                    {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-power")]
                crate::pages::Message::Power(message) => {
                    if let Some(page) = self.pages.page_mut::<power::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-networking")]
                crate::pages::Message::Vpn(message) => {
                    if let Some(page) = self.pages.page_mut::<networking::vpn::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-networking")]
                crate::pages::Message::WiFi(message) => {
                    if let Some(page) = self.pages.page_mut::<networking::wifi::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-window-management")]
                crate::pages::Message::WindowManagement(message) => {
                    if let Some(page) = self.pages.page_mut::<desktop::window_management::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-networking")]
                crate::pages::Message::Wired(message) => {
                    if let Some(page) = self.pages.page_mut::<networking::wired::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }

                #[cfg(feature = "page-workspaces")]
                crate::pages::Message::Workspaces(message) => {
                    if let Some(page) = self.pages.page_mut::<desktop::workspaces::Page>() {
                        return page.update(message).map(Into::into);
                    }
                }
            },

            #[cfg(feature = "wayland")]
            Message::OutputAdded(info, output) => {
                return panel::update(
                    &mut self.pages,
                    panel::Message::OutputAdded(info.name.unwrap_or_default(), output),
                )
                .map(Into::into);
            }

            #[cfg(feature = "wayland")]
            Message::OutputRemoved(output) => {
                return panel::update(&mut self.pages, panel::Message::OutputRemoved(output))
                    .map(Into::into);
            }

            #[cfg(feature = "wayland")]
            Message::PanelEntries(entries) => {
                return panel::update(&mut self.pages, panel::Message::Entries(entries))
                    .map(Into::into);
            }

            #[cfg(feature = "cosmic-comp-config")]
            Message::CompConfig(comp_config) => {
                let mut tasks = Vec::new();

                #[cfg(feature = "page-window-management")]
                if let Some(page) = self
                    .pages
                    .page_mut::<crate::pages::desktop::window_management::Page>()
                {
                    tasks.push(
                        page.update(
                            crate::pages::desktop::window_management::Message::CompConfigUpdate(
                                comp_config.clone(),
                            ),
                        )
                        .map(Into::into),
                    );
                }

                // WMDE: the Displays page caches the two pointer-crossing keys, so it has to
                // hear about changes made anywhere else too.
                #[cfg(feature = "page-display")]
                if let Some(page) = self.pages.page_mut::<crate::pages::display::Page>() {
                    tasks.push(
                        page.update(crate::pages::display::Message::CompConfigUpdate(
                            comp_config.clone(),
                        ))
                        .map(Into::into),
                    );
                }

                #[cfg(feature = "page-accessibility")]
                if let Some(page) = self.pages.page_mut::<accessibility::magnifier::Page>() {
                    tasks.push(
                        page.update(
                            self.active_page,
                            crate::pages::accessibility::magnifier::Message::CompConfigUpdate(
                                comp_config.clone(),
                            ),
                        )
                        .map(Into::into),
                    );
                }

                return Task::batch(tasks);
            }

            #[cfg(feature = "wayland")]
            Message::PanelConfig(config) => {
                let mut tasks = vec![
                    panel::update(&mut self.pages, panel::Message::Config(config.clone()))
                        .map(Into::into),
                ];

                // The applet list of that same panel reads the plugin lists out of the very
                // same config, and is a page of its own.
                if let Some(page) = self
                    .pages
                    .find_page_by_id(&applets_inner::page_id(&config.name))
                    .map(|(entity, _)| entity)
                    .and_then(|entity| self.pages.page.get_mut(entity))
                    .and_then(|page| page.downcast_mut::<applets_inner::Page>())
                {
                    tasks.push(
                        page.update(applets_inner::Message::PanelConfig(config))
                            .map(Into::into),
                    );
                }

                return Task::batch(tasks);
            }
            #[cfg(feature = "wayland")]
            Message::DesktopInfo => {
                let info_list: Vec<_> = freedesktop_desktop_entry::Iter::new(
                    freedesktop_desktop_entry::default_paths(),
                )
                .filter_map(|p| applets_inner::Applet::try_from(Cow::from(p)).ok())
                .collect();

                // Every panel has an applet list, and a newly installed applet belongs on
                // all of them.
                let entities: Vec<_> = self
                    .pages
                    .info
                    .iter()
                    .filter(|(_, info)| {
                        info.id.starts_with("panel:") && info.id.ends_with(":applets")
                    })
                    .map(|(entity, _)| entity)
                    .collect();

                let tasks: Vec<_> = entities
                    .into_iter()
                    .filter_map(|entity| {
                        self.pages
                            .page
                            .get_mut(entity)
                            .and_then(|page| page.downcast_mut::<applets_inner::Page>())
                            .map(|page| {
                                page.update(applets_inner::Message::Applets(info_list.clone()))
                                    .map(Into::into)
                            })
                    })
                    .collect();

                return Task::batch(tasks);
            }

            Message::SetTheme(t) => {
                return cosmic::command::set_theme(t);
            }
            Message::OpenContextDrawer(page) => {
                self.core.window.show_context = true;
                self.active_context_page = Some(page);
            }

            Message::Error(error) => {
                tracing::error!(error, "error occurred");
            }
            Message::Surface(a) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(a),
                ));
            }
        }

        Task::none()
    }

    #[cfg(feature = "single-instance")]
    fn dbus_activation(&mut self, msg: cosmic::dbus_activation::Message) -> Task<Self::Message> {
        match msg.msg {
            cosmic::dbus_activation::Details::Activate
            | cosmic::dbus_activation::Details::Open { .. } => None,
            cosmic::dbus_activation::Details::ActivateAction { action, .. } => {
                PageCommands::from_str(&action)
                    .ok()
                    .and_then(|action| self.subtask_to_page(&action))
                    .map(|e| self.activate_page(e))
            }
        }
        .unwrap_or_else(Task::none)
    }

    fn view(&self) -> Element<'_, Message> {
        let view = if self.search_active && !self.search_input.is_empty() {
            self.search_view()
        } else if let Some(content) = self.pages.content(self.active_page) {
            self.page_view(content)
        } else if let Some(sub_pages) = self.pages.sub_pages(self.active_page) {
            self.sub_page_view(sub_pages)
        } else {
            return self.page_container(cosmic::widget::space());
        };

        container(view).into()
    }

    #[allow(clippy::too_many_lines)]
    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        panic!("unknown window ID: {id:?}");
    }

    fn context_drawer(&self) -> Option<ContextDrawer<'_, Message>> {
        if self.core.window.show_context {
            self.active_context_page.and_then(|context_page| {
                self.pages.context_drawer(context_page).map(|cd| {
                    let cd = cd.map(Message::from);

                    // TODO: The page should handle this?
                    if let Some(title) = self.context_title.as_ref() {
                        cd.title(title)
                    } else {
                        cd
                    }
                })
            })
        } else {
            None
        }
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        self.pages
            .dialog(self.active_page)
            .map(|e| e.map(Message::PageMessage))
    }

    fn system_theme_update(
        &mut self,
        _keys: &[&'static str],
        new_theme: &cosmic::cosmic_theme::Theme,
    ) -> Task<Self::Message> {
        let mut tasks = Vec::new();
        #[cfg(feature = "page-accessibility")]
        if let Some(page) = self.pages.page_mut::<accessibility::Page>() {
            tasks.push(
                page.update(accessibility::Message::SystemTheme(Box::new(
                    new_theme.clone(),
                )))
                .map(Into::into),
            );
        }

        Task::batch(tasks)
    }
}

impl SettingsApp {
    /// Activates a page.
    fn activate_page(&mut self, page: page::Entity) -> Task<crate::Message> {
        let current_page = self.active_page;
        self.active_page = page;

        let mut leave_task = iced::Task::none();
        let mut close_context_drawer_task = iced::Task::none();

        if current_page != page {
            self.loaded_pages.remove(&current_page);

            close_context_drawer_task = self.close_context_drawer();

            leave_task = self
                .pages
                .on_leave(current_page)
                .unwrap_or(iced::Task::none())
                .map(Message::PageMessage)
                .map(Into::into);
            self.last_active_page = Box::from(&*self.pages.info[page].id);
            self.config.set_active_page(self.last_active_page.clone());
        }

        self.search_clear();
        self.search_active = false;
        self.activate_navbar(page);
        self.loaded_pages.insert(page);

        let page_task = self
            .pages
            .on_enter(page)
            .map(Message::PageMessage)
            .map(Into::into);

        Task::batch(vec![
            leave_task,
            page_task,
            close_context_drawer_task,
            cosmic::task::future(async { Message::SetWindowTitle }),
        ])
    }

    fn set_title(&mut self) -> Task<crate::Message> {
        self.set_window_title(
            format!(
                "{} - {}",
                self.pages.info[self.active_page].title,
                // Absolute path: `mod app` is declared before `#[macro_use] mod localize`
                // in main.rs, so the bare macro is not in scope here. Reordering main.rs
                // would put a local hunk into a file that has none.
                crate::fl!("app")
            ),
            self.core.main_window_id().unwrap(),
        )
    }

    /// Activates the navbar item associated with a page.
    fn activate_navbar(&mut self, mut page: page::Entity) {
        // WMDE: climb to the nearest ancestor that has a navbar item, rather than looking
        // one level up. Panels are a nav entry three deep - the panel, its applet list, an
        // applet's settings - and a single hop leaves the entry unhighlighted from the
        // second level down.
        loop {
            if let Some(nav_id) = self.pages.data::<segmented_button::Entity>(page).copied() {
                self.nav_model.activate(nav_id);
                return;
            }

            match self.pages.info.get(page).and_then(|info| info.parent) {
                Some(parent) => page = parent,
                None => return,
            }
        }
    }

    fn close_context_drawer(&mut self) -> Task<Message> {
        self.core.window.show_context = false;
        self.active_context_page = None;
        self.pages
            .on_context_drawer_close(self.active_page)
            .unwrap_or(iced::Task::none())
            .map(Message::PageMessage)
            .map(Into::into)
    }

    /// Adds a main page to the settings application.
    fn insert_page<P: page::AutoBind<crate::pages::Message>>(
        &mut self,
    ) -> page::Insert<'_, crate::pages::Message> {
        let id = self.pages.register::<P>().id();
        self.navbar_insert(id);

        page::Insert {
            model: &mut self.pages,
            id,
        }
    }

    fn navbar_insert(&mut self, id: page::Entity) -> segmented_button::SingleSelectEntityMut<'_> {
        let page = &self.pages.info[id];

        self.nav_model
            .insert()
            .text(page.title.clone())
            .icon(icon::from_name(&*page.icon_name))
            .data(id)
            .with_id(|nav_id| self.pages.data_set(id, nav_id))
    }

    /// Displays the view of a page.
    fn page_view(&self, content: &[section::Entity]) -> cosmic::Element<'_, Message> {
        let page = &self.pages.page[self.active_page];
        let page_info = &self.pages.info[self.active_page];
        let mut sections_column = Vec::with_capacity(content.len());

        let header = if let Some(custom_header) = page.header() {
            custom_header.map(Message::from)
        } else if let Some(parent) = page_info.parent {
            let page_header = crate::widget::sub_page_header(
                page.title().unwrap_or(page_info.title.as_str()),
                self.pages.info[parent].title.as_str(),
                Message::Page(parent),
            );

            let mut page_header_content: cosmic::iced::widget::Row<'_, Message, Theme> =
                row::with_capacity(2)
                    .align_y(iced::Alignment::End)
                    .push(page_header);

            if let Some(element) = page.header_view() {
                page_header_content = page_header_content.push(element.map(Message::from));
            }

            page_header_content.into()
        } else {
            cosmic::widget::text::title3(&page_info.title).into()
        };

        for id in content.iter().copied() {
            let section = &self.pages.sections[id];
            let model = &self.pages.page[self.active_page];

            if section
                .show_while
                .as_ref()
                .is_none_or(|func| func(model.as_ref()))
            {
                sections_column.push(
                    (section.view_fn)(&self.pages, model.as_ref(), section)
                        .map(Message::PageMessage),
                );
            }
        }

        let view = self
            .page_container(settings::view_column(sections_column))
            .apply(scrollable)
            .height(Length::Fill)
            .apply(|w| id_container(w, self.id()));

        column::with_capacity(2)
            .push(self.page_container(header))
            .push(view)
            .height(Length::Fill)
            .into()
    }

    fn search_changed(&mut self, phrase: String) -> Task<crate::Message> {
        // If the text was cleared, clear the search results too.
        if phrase.is_empty() {
            self.search_clear();
            return Task::none();
        }

        let mut tasks = Vec::new();

        // Create a case-insensitive regular expression for the search function.
        let search_expression = regex::RegexBuilder::new(&phrase)
            .case_insensitive(true)
            .unicode(true)
            .ignore_whitespace(true)
            .size_limit(16 * 1024)
            .build();

        if let Ok(expression) = search_expression {
            // With the new search expression, generate new search results.
            let results: Vec<_> = self.pages.search(&expression).collect();

            // Use the results if results were found.
            if !results.is_empty() {
                self.search_selections = results;

                let mut unload = BTreeSet::new();
                let mut load = BTreeSet::new();

                'outer: for loaded_page in &self.loaded_pages {
                    for (page, _) in &self.search_selections {
                        if loaded_page == page {
                            continue 'outer;
                        }
                    }

                    unload.insert(*loaded_page);
                }

                for (page, _) in &self.search_selections {
                    if !self.loaded_pages.contains(page) {
                        load.insert(*page);
                    }
                }

                for page in load {
                    self.loaded_pages.insert(page);
                    tasks.push(self.pages.on_enter(page));
                }

                for page in unload {
                    self.loaded_pages.remove(&page);
                    self.pages.on_leave(page);
                }
            }
        }

        self.search_input = phrase;

        if tasks.is_empty() {
            Task::none()
        } else {
            cosmic::task::batch(tasks)
                .map(Message::PageMessage)
                .map(Into::into)
        }
    }

    /// Clears the search results so that the search page will not be shown.
    fn search_clear(&mut self) {
        self.search_selections.clear();
        self.search_input.clear();
    }

    /// Displays the search view.
    fn search_view(&self) -> cosmic::Element<'_, Message> {
        let mut sections: Vec<cosmic::Element<Message>> = Vec::new();

        let mut current_page = page::Entity::default();
        for (page, section) in self.search_selections.iter().copied() {
            // WMDE: a hit can outlive what it points at. The results survive between
            // frames, and deleting a panel from a section drawn right here takes its page
            // and its sections out of the registry; indexing them would panic.
            let (Some(section), Some(model)) =
                (self.pages.sections.get(section), self.pages.page.get(page))
            else {
                continue;
            };

            if page != current_page {
                current_page = page;
                sections.push(search_header(&self.pages, page));
            }

            if section
                .show_while
                .as_ref()
                .is_none_or(|func| func(model.as_ref()))
            {
                let section = (section.view_fn)(&self.pages, model.as_ref(), section)
                    .map(Message::PageMessage)
                    .apply(container)
                    .padding([0, 0, 0, cosmic::theme::active().cosmic().space_l()]);

                sections.push(section.into());
            }
        }

        self.page_container(settings::view_column(sections))
            .apply(scrollable)
            .into()
    }

    /// Displays the sub-pages view of a page.
    fn sub_page_view(&self, sub_pages: &[page::Entity]) -> cosmic::Element<'_, Message> {
        let page_list = sub_pages
            .iter()
            .copied()
            .fold(
                column::with_capacity(sub_pages.len()),
                |page_list, entity| {
                    let sub_page = &self.pages.info[entity];

                    page_list.push(crate::widget::page_list_item(
                        sub_page.title.as_str(),
                        sub_page.description.as_str(),
                        "",
                        &sub_page.icon_name,
                        entity,
                    ))
                },
            )
            .spacing(cosmic::theme::spacing().space_s)
            .apply(|widget| scrollable(self.page_container(widget)).height(Length::Fill))
            .apply(Element::from)
            .map(Message::Page);

        column::with_capacity(2)
            .push(self.page_container(page_title(&self.pages.info[self.active_page])))
            .push(page_list)
            .height(Length::Fill)
            .into()
    }

    fn page_container<'a, Message: 'static>(
        &self,
        content: impl Into<cosmic::Element<'a, Message>>,
    ) -> cosmic::Element<'a, Message> {
        let theme = cosmic::theme::active();

        let padding = if self.core.is_condensed() {
            theme.cosmic().space_s()
        } else {
            theme.cosmic().space_l()
        };
        // prevents content from touching window edge on bottom of scroll
        let bottom_spacer = theme.cosmic().space_m();

        container(content.into())
            .max_width(800)
            .width(Length::Fill)
            .apply(container)
            .center_x(Length::Fill)
            .padding([0, padding, bottom_spacer, padding])
            .into()
    }
}

/// WMDE: how one row of the navigation list looks.
///
/// The list is a flat, square, full-width strip, like the start menu's category list
/// (`theme::Button::AppletMenu` there) and the Files sidebar. Upstream's
/// `SegmentedButton::NavBar` is a tab bar in disguise: it rounds items by `radius_m` and
/// tints text and icon with the accent as soon as an item is hovered or selected.
///
/// The four states have to be defined together, they cannot be fixed one at a time. The
/// widget resolves them in the order `pressed -> hover -> active -> inactive`
/// (`libcosmic/src/widget/segmented_button/widget.rs`), so **hover wins over active**:
/// hovering the selected row swaps in the hover appearance, and an accent left on `active`
/// alone would make the current page flip colour under the cursor.
///
/// The fills are upstream's numbers (`neutral_5` at 20/30/25 %). Hover being denser than
/// the selection is deliberate: the selected row must not dim when the cursor lands on it,
/// and once the cursor is elsewhere it is the only row with a fill at all.
fn nav_item_style() -> cosmic::theme::SegmentedButton {
    use cosmic::iced::{Background, Border, Color};
    use cosmic::widget::segmented_button::{Appearance, ItemAppearance, ItemStatusAppearance};

    cosmic::theme::SegmentedButton::Custom(Box::new(|theme| {
        let cosmic = theme.cosmic();
        let square = ItemAppearance {
            border: Border {
                radius: cosmic.corner_radii.radius_0.into(),
                ..Default::default()
            },
        };
        // Neutral in every state - the container's own foreground colour, the same one an
        // unhovered row already uses.
        let text_color = Color::from(theme.current_container().component.on);

        let state = |alpha: Option<f32>| ItemStatusAppearance {
            background: alpha.map(|alpha| {
                let mut fill = Color::from(cosmic.palette.neutral_5);
                fill.a = alpha;
                Background::Color(fill)
            }),
            first: square,
            middle: square,
            last: square,
            text_color,
        };

        Appearance {
            background: None,
            border: Border {
                radius: cosmic.corner_radii.radius_0.into(),
                ..Default::default()
            },
            // No tab underline: the list marks the current page with its fill.
            active_width: 0.0,
            active: state(Some(0.20)),
            inactive: state(None),
            hover: state(Some(0.30)),
            pressed: state(Some(0.25)),
        }
    }))
}
