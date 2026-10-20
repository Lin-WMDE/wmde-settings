// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use cosmic::iced::Length;
use cosmic::widget::{self, settings};
use cosmic::{Apply, Element, surface};

use cosmic_comp_config::{CosmicCompConfig, WindowSwitcherConfig, WindowSwitcherStyle};
use cosmic_config::{ConfigGet, ConfigSet};
use cosmic_settings_config::{Action, Binding, Shortcuts, shortcuts};
use cosmic_settings_page::{self as page, Section, section};
use slotmap::SlotMap;
use tracing::error;

#[derive(Clone, Debug)]
pub enum Message {
    SuperKey(usize),
    CompConfigUpdate(Box<CosmicCompConfig>),
    SetFocusFollowsCursor(bool),
    SaveFocusFollowsCursorDelay(bool),
    SetFocusFollowsCursorDelay(String),
    SetCursorFollowsFocus(bool),
    ShowActiveWindowHint(bool),
    SetEdgeSnapThreshold(u32),
    // WMDE: the Alt+Tab window switcher, the `window_switcher` key of the compositor.
    SwitcherStyle(usize),
    SwitcherMinimized(bool),
    SwitcherBlur(bool),
    SwitcherAnimate(bool),
    Surface(surface::Action<crate::app::Message>),
}

pub struct Page {
    pub super_key_selections: Vec<String>,
    pub super_key_active: Option<usize>,
    comp_config: cosmic_config::Config,
    focus_follows_cursor: bool,
    focus_follows_cursor_delay: u64,
    focus_delay_text: String,
    cursor_follows_focus: bool,
    show_active_hint: bool,
    edge_snap_threshold: u32,
    switcher: WindowSwitcherConfig,
    switcher_styles: Vec<String>,
}

impl Default for Page {
    fn default() -> Self {
        let comp_config = cosmic_config::Config::new(
            crate::config::COSMIC_COMP_CONFIG,
            crate::config::COSMIC_COMP_CONFIG_VERSION,
        )
        .unwrap();
        let focus_follows_cursor = comp_config
            .get("focus_follows_cursor")
            .unwrap_or_else(|err| {
                if err.is_err() {
                    error!(?err, "Failed to read config 'focus_follows_cursor'");
                }
                false
            });
        let cursor_follows_focus = comp_config
            .get("cursor_follows_focus")
            .unwrap_or_else(|err| {
                if err.is_err() {
                    error!(?err, "Failed to read config 'cursor_follows_focus'");
                }
                false
            });

        let focus_follows_cursor_delay = comp_config
            .get("focus_follows_cursor_delay")
            .inspect_err(|err| {
                if err.is_err() {
                    error!(?err, "Failed to read config 'focus_follows_cursor_delay'")
                }
            })
            .unwrap_or(250);

        let show_active_hint = comp_config
            .get("active_hint")
            .inspect_err(|err| {
                if err.is_err() {
                    error!(?err, "Failed to read config 'active_hint'")
                }
            })
            .unwrap_or(true);

        let edge_snap_threshold = comp_config
            .get("edge_snap_threshold")
            .inspect_err(|err| {
                if err.is_err() {
                    error!(?err, "Failed to read config 'edge_snap_threshold'")
                }
            })
            .unwrap_or(0);

        let switcher = comp_config
            .get::<WindowSwitcherConfig>("window_switcher")
            .inspect_err(|err| {
                if err.is_err() {
                    error!(?err, "Failed to read config 'window_switcher'")
                }
            })
            .unwrap_or_else(|_| CosmicCompConfig::default().window_switcher);

        Page {
            // WMDE: only the Start menu or nothing. The workspaces overview and the app
            // library are not in the stack, so upstream's other two choices led nowhere.
            super_key_selections: vec![fl!("super-key", "launcher"), fl!("super-key", "none")],
            super_key_active: super_key_active_config(),
            comp_config,
            focus_follows_cursor,
            focus_follows_cursor_delay,
            focus_delay_text: format!("{focus_follows_cursor_delay}"),
            cursor_follows_focus,
            show_active_hint,
            edge_snap_threshold,
            switcher,
            // In the order of `SWITCHER_STYLES`.
            switcher_styles: vec![
                fl!("window-switcher", "style-grid"),
                fl!("window-switcher", "style-none"),
            ],
        }
    }
}

/// The looks the window switcher has, in the order of the dropdown.
const SWITCHER_STYLES: [WindowSwitcherStyle; 2] =
    [WindowSwitcherStyle::Grid, WindowSwitcherStyle::None];

impl Page {
    pub fn update(&mut self, message: Message) -> cosmic::iced::Task<crate::app::Message> {
        match message {
            Message::SuperKey(id) => {
                let action = match id {
                    0 => Some(shortcuts::action::System::Launcher),
                    1 => None,
                    _ => return cosmic::iced::Task::none(),
                };

                self.super_key_active = Some(id);
                super_key_set(action);
            }
            Message::SetFocusFollowsCursor(value) => {
                self.focus_follows_cursor = value;
                if let Err(err) = self
                    .comp_config
                    .set("focus_follows_cursor", self.focus_follows_cursor)
                {
                    error!(?err, "Failed to set config 'focus_follows_cursor'");
                }
            }
            Message::SaveFocusFollowsCursorDelay(save) => {
                // Debounce to avoid spam writing config on user input
                if save {
                    if let Err(err) = self.comp_config.set(
                        "focus_follows_cursor_delay",
                        self.focus_follows_cursor_delay,
                    ) {
                        error!(?err, "Failed to set config 'focus_follows_cursor_delay'");
                    }

                    self.focus_delay_text = format!("{}", self.focus_follows_cursor_delay);
                }
            }
            Message::SetFocusFollowsCursorDelay(value) => {
                if let Ok(delay) = value.parse() {
                    self.focus_follows_cursor_delay = delay;
                }
                self.focus_delay_text = value;
            }
            Message::SetCursorFollowsFocus(value) => {
                self.cursor_follows_focus = value;
                if let Err(err) = self
                    .comp_config
                    .set("cursor_follows_focus", self.cursor_follows_focus)
                {
                    error!(?err, "Failed to set config 'cursor_follows_focus'");
                }
            }
            Message::ShowActiveWindowHint(value) => {
                self.show_active_hint = value;
                if let Err(err) = self.comp_config.set("active_hint", value) {
                    error!(?err, "Failed to set config 'active_hint'");
                }
            }
            Message::CompConfigUpdate(comp_config) => {
                if comp_config.active_hint != self.show_active_hint {
                    self.show_active_hint = comp_config.active_hint;
                }
                self.switcher = comp_config.window_switcher;
            }
            Message::SwitcherStyle(index) => {
                let Some(style) = SWITCHER_STYLES.get(index) else {
                    return cosmic::iced::Task::none();
                };
                self.switcher.style = *style;
                self.save_switcher();
            }
            Message::SwitcherMinimized(value) => {
                self.switcher.include_minimized = value;
                self.save_switcher();
            }
            Message::SwitcherBlur(value) => {
                self.switcher.blur_background = value;
                self.save_switcher();
            }
            Message::SwitcherAnimate(value) => {
                self.switcher.animate = value;
                self.save_switcher();
            }
            Message::SetEdgeSnapThreshold(value) => {
                self.edge_snap_threshold = value;
                if let Err(err) = self.comp_config.set("edge_snap_threshold", value) {
                    error!(?err, "Failed to set config 'edge_snap_threshold'");
                }
            }
            Message::Surface(a) => {
                return cosmic::task::message(crate::app::Message::Surface(a));
            }
        };
        cosmic::iced::Task::none()
    }

    /// The whole key at once: the compositor reads it as one value.
    fn save_switcher(&self) {
        if let Err(err) = self.comp_config.set("window_switcher", self.switcher) {
            error!(?err, "Failed to set config 'window_switcher'");
        }
    }
}

impl page::Page<crate::pages::Message> for Page {
    #[allow(clippy::too_many_lines)]
    fn content(
        &self,
        sections: &mut SlotMap<section::Entity, Section<crate::pages::Message>>,
    ) -> Option<page::Content> {
        Some(vec![
            sections.insert(window_management()),
            sections.insert(window_switcher()),
            sections.insert(window_controls()),
            sections.insert(focus_navigation()),
        ])
    }

    fn info(&self) -> page::Info {
        page::Info::new(
            "window-management",
            "preferences-window-management-symbolic",
        )
        .title(fl!("window-management"))
        .description(fl!("xdg-entry-window-management-comment"))
    }
}

impl page::AutoBind<crate::pages::Message> for Page {}

pub fn window_management() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        super_key = fl!("super-key");
        _launcher = fl!("super-key", "launcher");
        _none = fl!("super-key", "none");
        edge_gravity = fl!("edge-gravity");
    });

    Section::default()
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let descriptions = &section.descriptions;

            settings::section()
                .title(&section.title)
                .add(settings::item::builder(&descriptions[super_key]).control(
                    widget::dropdown::popup_dropdown(
                        &page.super_key_selections,
                        page.super_key_active,
                        Message::SuperKey,
                        cosmic::iced::window::Id::RESERVED,
                        Message::Surface,
                        |a| {
                            crate::app::Message::PageMessage(
                                crate::pages::Message::WindowManagement(a),
                            )
                        },
                    ),
                ))
                .add(
                    settings::item::builder(&descriptions[edge_gravity])
                        .toggler(page.edge_snap_threshold != 0, |is_enabled| {
                            Message::SetEdgeSnapThreshold(if is_enabled { 10 } else { 0 })
                        }),
                )
                .apply(Element::from)
                .map(crate::pages::Message::WindowManagement)
        })
}

// WMDE: the Alt+Tab window switcher drawn by the compositor. Its look is a dropdown because
// more looks are to come; the toggles only matter to the thumbnails, so they go with them.
pub fn window_switcher() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        style = fl!("window-switcher", "style");
        _grid = fl!("window-switcher", "style-grid");
        _none = fl!("window-switcher", "style-none");
        minimized = fl!("window-switcher", "minimized");
        blur = fl!("window-switcher", "blur");
        animate = fl!("window-switcher", "animate");
    });

    Section::default()
        .title(fl!("window-switcher"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let descriptions = &section.descriptions;
            let thumbnails = page.switcher.style == WindowSwitcherStyle::Grid;

            let mut section_view = settings::section().title(&section.title).add(
                settings::item::builder(&descriptions[style]).control(
                    widget::dropdown::popup_dropdown(
                        &page.switcher_styles,
                        SWITCHER_STYLES
                            .iter()
                            .position(|s| *s == page.switcher.style),
                        Message::SwitcherStyle,
                        cosmic::iced::window::Id::RESERVED,
                        Message::Surface,
                        |a| {
                            crate::app::Message::PageMessage(
                                crate::pages::Message::WindowManagement(a),
                            )
                        },
                    ),
                ),
            );
            if thumbnails {
                section_view = section_view
                    .add(
                        settings::item::builder(&descriptions[minimized])
                            .toggler(page.switcher.include_minimized, Message::SwitcherMinimized),
                    )
                    .add(
                        settings::item::builder(&descriptions[blur])
                            .toggler(page.switcher.blur_background, Message::SwitcherBlur),
                    )
                    .add(
                        settings::item::builder(&descriptions[animate])
                            .toggler(page.switcher.animate, Message::SwitcherAnimate),
                    );
            }
            section_view
                .apply(Element::from)
                .map(crate::pages::Message::WindowManagement)
        })
}

// WMDE: no minimize / maximize toggles. A window's title bar variant alone decides its
// buttons. The CosmicTk show_minimize / show_maximize keys stay at their default `true`,
// which settings-daemon mirrors into GNOME's button layout - all three buttons for GTK
// apps too, as on the normal title bar.
pub fn window_controls() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        active_window_hint = fl!("window-controls", "active-window-hint");
    });

    Section::default()
        .title(fl!("window-controls"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let descriptions = &section.descriptions;

            settings::section()
                .title(&section.title)
                .add(
                    settings::item::builder(&descriptions[active_window_hint])
                        .toggler(page.show_active_hint, Message::ShowActiveWindowHint),
                )
                .apply(Element::from)
                .map(crate::pages::Message::WindowManagement)
        })
}

pub fn focus_navigation() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        focus_follows_cursor = fl!("focus-navigation", "focus-follows-cursor");
        focus_follows_cursor_delay = fl!("focus-navigation", "focus-follows-cursor-delay");
        cursor_follows_focus = fl!("focus-navigation", "cursor-follows-focus");
    });

    Section::default()
        .title(fl!("focus-navigation"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let descriptions = &section.descriptions;

            settings::section()
                .title(&section.title)
                .add(
                    settings::item::builder(&descriptions[focus_follows_cursor])
                        .toggler(page.focus_follows_cursor, Message::SetFocusFollowsCursor),
                )
                .add(settings::item(
                    &descriptions[focus_follows_cursor_delay],
                    widget::editable_input("", &page.focus_delay_text, false, |editing| {
                        Message::SaveFocusFollowsCursorDelay(!editing)
                    })
                    .select_on_focus(true)
                    .on_input(Message::SetFocusFollowsCursorDelay)
                    .on_submit(|_| Message::SaveFocusFollowsCursorDelay(true))
                    .width(Length::Fixed(80.0)),
                ))
                .add(
                    settings::item::builder(&descriptions[cursor_follows_focus])
                        .toggler(page.cursor_follows_focus, Message::SetCursorFollowsFocus),
                )
                .apply(Element::from)
                .map(crate::pages::Message::WindowManagement)
        })
}

fn super_key_active_config() -> Option<usize> {
    let super_binding = Binding::new(shortcuts::Modifiers::new().logo(), None);
    let config = shortcuts::context().ok()?;

    shortcuts::shortcuts(&config)
        .iter()
        .find(|(binding, _action)| binding == &&super_binding)
        .and_then(|(_, action)| match action {
            Action::System(shortcuts::action::System::Launcher) => Some(0),
            // WMDE: "None" writes Disable, so show it as chosen instead of an empty field.
            Action::Disable => Some(1),
            _ => None,
        })
}

fn super_key_set(action: Option<shortcuts::action::System>) {
    let Ok(config) = shortcuts::context() else {
        return;
    };

    let mut shortcuts = config.get::<Shortcuts>("custom").unwrap_or_default();
    let action = action.map(Action::System).unwrap_or(Action::Disable);

    shortcuts.0.insert(
        Binding::new(shortcuts::Modifiers::new().logo(), None),
        action,
    );

    _ = config.set("custom", &shortcuts);
}
