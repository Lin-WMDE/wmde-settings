use cosmic::cctk::sctk::reexports::client::Proxy;
use cosmic::cctk::sctk::reexports::client::backend::ObjectId;
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::cosmic_theme::{Density, Roundness};
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{button, container, dropdown, row, settings, slider, space, text};
use cosmic::{Element, Task, surface};

use cosmic::Apply;
use cosmic_config::ConfigSet;
use cosmic_panel_config::{
    AutoHide, CosmicPanelBackground, CosmicPanelConfig, CosmicPanelOuput, PanelAnchor, PanelLook,
    PanelSize,
};
use cosmic_settings_page::{self as page, Section};
use std::collections::HashMap;
use std::time::Duration;

pub struct PageInner {
    pub(crate) config_helper: Option<cosmic_config::Config>,
    pub(crate) panel_config: Option<CosmicPanelConfig>,
    pub opacity: f32,
    pub opacity_changing: bool,
    pub size: PanelSize,
    pub outputs: Vec<String>,
    pub anchors: Vec<String>,
    pub backgrounds: Vec<String>,
    pub looks: Vec<String>,
    // TODO move these into panel config
    pub(crate) outputs_map: HashMap<ObjectId, (String, WlOutput)>,
    pub(crate) system_default: Option<CosmicPanelConfig>,
}

impl Default for PageInner {
    fn default() -> Self {
        Self {
            config_helper: Option::default(),
            panel_config: Option::default(),
            opacity: 0.0,
            opacity_changing: false,
            size: PanelSize::M,
            outputs: vec![fl!("all-displays")],
            anchors: vec![
                Anchor(PanelAnchor::Left).to_string(),
                Anchor(PanelAnchor::Right).to_string(),
                Anchor(PanelAnchor::Top).to_string(),
                Anchor(PanelAnchor::Bottom).to_string(),
            ],
            backgrounds: vec![
                Appearance::Match.to_string(),
                Appearance::Light.to_string(),
                Appearance::Dark.to_string(),
            ],
            looks: vec![
                Look(PanelLook::Bar).to_string(),
                Look(PanelLook::Island).to_string(),
            ],
            outputs_map: HashMap::default(),
            system_default: None,
        }
    }
}

/// Shared behaviour of a page that edits one panel.
///
/// The sections below are written against this trait rather than against a concrete page,
/// because every panel has a page and they are all the same Rust type.
pub trait PanelPage {
    fn inner(&self) -> &PageInner;

    fn inner_mut(&mut self) -> &mut PageInner;

    /// Which page this is.
    ///
    /// Sections must read it here, at draw time, and never capture it: `content()` builds
    /// the sections before `set_id()` has run, so an entity captured then is null. Getting
    /// this wrong writes into a neighbouring panel's config without a panic to show for it.
    fn entity(&self) -> page::Entity;

    fn autohide_label(&self) -> String;

    fn gap_label(&self) -> String;

    fn extend_label(&self) -> String;

    fn configure_applets_label(&self) -> String;

    /// `Info::id` of this panel's applet list page. One page per panel, so this is not a
    /// constant.
    fn applets_page_id(&self) -> String;
}

pub(crate) fn behavior_and_position<
    P: page::Page<crate::pages::Message> + PanelPage,
    T: Fn(page::Entity, Message) -> crate::pages::Message + Copy + Send + Sync + 'static,
>(
    p: &P,
    msg_map: T,
) -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        autohide_label = p.autohide_label();
        position = fl!("panel-behavior-and-position", "position");
        display = fl!("panel-behavior-and-position", "display");
    });

    Section::default()
        .title(fl!("panel-behavior-and-position"))
        .descriptions(descriptions)
        .view::<P>(move |_binder, page, section| {
            let descriptions = &section.descriptions;
            let entity = page.entity();
            let msg_map = move |m| msg_map(entity, m);
            let page = page.inner();
            let Some(panel_config) = page.panel_config.as_ref() else {
                return Element::from(text::body(fl!("unknown")));
            };
            settings::section()
                .title(&section.title)
                .add(
                    settings::item::builder(&descriptions[autohide_label])
                        .toggler(panel_config.autohide_enabled(), Message::AutoHidePanel),
                )
                .add(settings::item(
                    &descriptions[position],
                    dropdown::popup_dropdown(
                        page.anchors.as_slice(),
                        Some(panel_config.anchor as usize),
                        Message::PanelAnchor,
                        cosmic::iced::window::Id::RESERVED,
                        Message::Surface,
                        move |a| crate::app::Message::PageMessage(msg_map(a)),
                    ),
                ))
                .add(settings::item(
                    &descriptions[display],
                    dropdown::popup_dropdown(
                        page.outputs.as_slice(),
                        match &panel_config.output {
                            CosmicPanelOuput::All => Some(0),
                            CosmicPanelOuput::Active => None,
                            CosmicPanelOuput::Name(n) => page.outputs.iter().position(|o| o == n),
                        },
                        Message::Output,
                        cosmic::iced::window::Id::RESERVED,
                        Message::Surface,
                        move |a| crate::app::Message::PageMessage(msg_map(a)),
                    ),
                ))
                .apply(Element::from)
                .map(msg_map)
        })
}

pub(crate) fn style<
    P: page::Page<crate::pages::Message> + PanelPage,
    T: Fn(page::Entity, Message) -> crate::pages::Message + Copy + Send + Sync + 'static,
>(
    p: &P,
    msg_map: T,
) -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        gap_label = p.gap_label();
        extend_label = p.extend_label();
        look = fl!("panel-look");
        appearance = fl!("panel-style", "appearance");
        background_opacity = fl!("panel-style", "background-opacity");
        size = fl!("panel-style", "size");
    });

    Section::default()
        .title(fl!("panel-style"))
        .descriptions(descriptions)
        .view::<P>(move |_binder, page, section| {
            let descriptions = &section.descriptions;
            let entity = page.entity();
            let msg_map = move |m| msg_map(entity, m);
            let inner = page.inner();
            let Some(panel_config) = inner.panel_config.as_ref() else {
                return Element::from(text::body(fl!("unknown")));
            };
            settings::section()
                .title(&section.title)
                .add(settings::item(
                    &descriptions[look],
                    dropdown::popup_dropdown(
                        inner.looks.as_slice(),
                        match panel_config.effective_look() {
                            PanelLook::Island => Some(1),
                            PanelLook::Bar | PanelLook::Auto => Some(0),
                        },
                        Message::Look,
                        cosmic::iced::window::Id::RESERVED,
                        Message::Surface,
                        move |a| crate::app::Message::PageMessage(msg_map(a)),
                    ),
                ))
                .add(
                    settings::item::builder(&descriptions[gap_label])
                        .toggler(panel_config.anchor_gap, Message::AnchorGap),
                )
                .add(
                    settings::item::builder(&descriptions[extend_label])
                        .toggler(panel_config.expand_to_edges, Message::ExtendToEdge),
                )
                .add(settings::item(
                    &descriptions[appearance],
                    dropdown::popup_dropdown(
                        inner.backgrounds.as_slice(),
                        match panel_config.background {
                            CosmicPanelBackground::ThemeDefault => Some(0),
                            CosmicPanelBackground::Light => Some(1),
                            CosmicPanelBackground::Dark => Some(2),
                            CosmicPanelBackground::Color(_) => None,
                        },
                        Message::Appearance,
                        cosmic::iced::window::Id::RESERVED,
                        Message::Surface,
                        move |a| crate::app::Message::PageMessage(msg_map(a)),
                    ),
                ))
                // Pixels, not a ladder of named sizes: the thickness is the one panel
                // measurement a user has an exact number in mind for. A named size still
                // reads correctly here - it reports the height it comes out as.
                .add(settings::item::builder(&descriptions[size]).control(
                    cosmic::widget::spin_button(
                        format!("{} px", inner.size.thickness()),
                        "panel thickness",
                        inner.size.thickness(),
                        1,
                        cosmic_panel_config::MIN_PANEL_THICKNESS,
                        MAX_PANEL_THICKNESS,
                        Message::PanelThickness,
                    ),
                ))
                .add(
                    settings::item::builder(&descriptions[background_opacity]).flex_control({
                        row::with_capacity(2)
                            .align_y(Alignment::Center)
                            .spacing(8)
                            .width(Length::Fill)
                            .push(
                                text::body(fl!(
                                    "number",
                                    HashMap::from_iter(vec![(
                                        "number",
                                        (panel_config.opacity * 100.0) as i32
                                    )])
                                ))
                                .width(Length::Fixed(22.0))
                                .align_x(Alignment::Center),
                            )
                            .push(
                                slider(0..=100, (panel_config.opacity * 100.0) as i32, |v| {
                                    Message::OpacityRequest(v as f32 / 100.0)
                                })
                                .width(Length::Fill)
                                .apply(container)
                                .max_width(250),
                            )
                    }),
                )
                .apply(Element::from)
                .map(msg_map)
        })
}

pub(crate) fn configuration<P: page::Page<crate::pages::Message> + PanelPage>(
    p: &P,
) -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        applets_label = p.configure_applets_label();
    });

    Section::default()
        .title(fl!("panel-applets"))
        .descriptions(descriptions)
        .view::<P>(move |binder, page, section| {
            let mut settings = settings::section().title(&section.title);
            let descriptions = &section.descriptions;
            let applets_page_id = page.applets_page_id();
            settings = if let Some((panel_applets_entity, _panel_applets_info)) =
                binder.info.iter().find(|(_, v)| v.id == applets_page_id)
            {
                settings.add(crate::widget::go_next_item(
                    &descriptions[applets_label],
                    crate::pages::Message::Page(panel_applets_entity),
                ))
            } else {
                settings
            };

            Element::from(settings)
        })
}

#[allow(clippy::too_many_lines)]
pub fn reset_button<
    P: page::Page<crate::pages::Message> + PanelPage,
    T: Fn(page::Entity, Message) -> crate::pages::Message + Copy + 'static,
>(
    msg_map: T,
) -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        reset_to_default = fl!("reset-to-default");
    });

    Section::default()
        .descriptions(descriptions)
        .view::<P>(move |_binder, page, section| {
            let descriptions = &section.descriptions;
            let entity = page.entity();
            let inner = page.inner();
            if inner.system_default == inner.panel_config {
                Element::from(space())
            } else {
                button::standard(&descriptions[reset_to_default])
                    .on_press(Message::ResetPanel)
                    .into()
            }
            .map(move |m| msg_map(entity, m))
        })
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Anchor(pub PanelAnchor);

impl std::fmt::Display for Anchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self.0 {
                PanelAnchor::Top => fl!("panel-top"),
                PanelAnchor::Bottom => fl!("panel-bottom"),
                PanelAnchor::Left => fl!("panel-left"),
                PanelAnchor::Right => fl!("panel-right"),
            }
        )
    }
}

/// WMDE: the thickest a panel may be set to from the interface. Past this a bar is a wall,
/// and the exclusive zone leaves nothing to work in.
pub const MAX_PANEL_THICKNESS: u32 = 200;

/// A [`PanelLook`] under its user-facing name. [`PanelLook::Auto`] has none: it is a rule for
/// reading old configs, not something to offer.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Look(pub PanelLook);

impl std::fmt::Display for Look {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self.0 {
                PanelLook::Island => fl!("panel-look", "island"),
                PanelLook::Bar | PanelLook::Auto => fl!("panel-look", "bar"),
            }
        )
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Appearance {
    Match,
    Light,
    Dark,
}

impl std::fmt::Display for Appearance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Appearance::Match => fl!("panel-appearance", "match"),
                Appearance::Light => fl!("panel-appearance", "light"),
                Appearance::Dark => fl!("panel-appearance", "dark"),
            }
        )
    }
}

impl TryFrom<CosmicPanelBackground> for Appearance {
    type Error = ();
    fn try_from(value: CosmicPanelBackground) -> Result<Self, Self::Error> {
        match value {
            CosmicPanelBackground::ThemeDefault => Ok(Appearance::Match),
            CosmicPanelBackground::Light => Ok(Appearance::Light),
            CosmicPanelBackground::Dark => Ok(Appearance::Dark),
            _ => Err(()),
        }
    }
}

impl From<Appearance> for CosmicPanelBackground {
    fn from(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Match => CosmicPanelBackground::ThemeDefault,
            Appearance::Light => CosmicPanelBackground::Light,
            Appearance::Dark => CosmicPanelBackground::Dark,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    // panel messages
    AutoHidePanel(bool),
    PanelAnchor(usize),
    Output(usize),
    AnchorGap(bool),
    PanelThickness(u32),
    Appearance(usize),
    Look(usize),
    ExtendToEdge(bool),
    OpacityRequest(f32),
    OpacityApply,
    OutputAdded(String, WlOutput),
    OutputRemoved(WlOutput),
    PanelConfig(Box<CosmicPanelConfig>),
    ResetPanel,
    Surface(surface::Action),
}

impl PageInner {
    pub(crate) fn update_defaults(&mut self) {
        let theme = cosmic::theme::system_preference();
        let theme = theme.cosmic();

        let Some(default) = self.system_default.as_mut() else {
            return;
        };

        let radius = theme.corner_radii;
        let roundness: Roundness = radius.into();

        if default.anchor_gap {
            let radii = theme.corner_radii.radius_xl[0] as u32;
            default.border_radius = radii;
        } else if matches!(roundness, Roundness::Round) && !default.expand_to_edges {
            default.border_radius = 12;
        } else {
            default.border_radius = 0;
        }

        let spacing = theme.spacing;
        let density = Density::from(spacing);
        default.spacing = match density {
            Density::Compact => 0,
            Density::Standard => 0,
            Density::Spacious => 4,
        };

        if self
            .panel_config
            .as_ref()
            .is_some_and(|c| c.effective_look() == PanelLook::Island)
        {
            default.padding = match roundness {
                Roundness::Round => 4,
                Roundness::SlightlyRound => 4,
                Roundness::Square => 0,
            };
        }
    }

    #[allow(clippy::too_many_lines)]
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let Some(helper) = self.config_helper.as_ref() else {
            return Task::none();
        };

        match &message {
            Message::ResetPanel => {
                if let Some((default, config)) = self
                    .system_default
                    .as_mut()
                    .zip(self.config_helper.as_ref())
                {
                    let theme = cosmic::theme::system_preference();
                    let theme = theme.cosmic();
                    let radius = theme.corner_radii;
                    let roundness: Roundness = radius.into();

                    if default.anchor_gap {
                        let radii = theme.corner_radii.radius_xl[0] as u32;
                        default.border_radius = radii;
                    } else if matches!(roundness, Roundness::Round) && !default.expand_to_edges {
                        default.border_radius = 12;
                    } else {
                        default.border_radius = 0;
                    }

                    if let Err(err) = default.write_entry(config) {
                        tracing::error!(?err, "Error resetting panel config.");
                    }
                    self.size.clone_from(&default.size);
                    self.system_default = Some(default.clone());
                    self.panel_config.clone_from(&self.system_default);
                } else {
                    tracing::error!("Panel config default is missing.");
                }
            }
            _ => {}
        };

        let Some(panel_config) = self.panel_config.as_mut() else {
            return Task::none();
        };

        match message {
            Message::AutoHidePanel(enabled) => {
                if enabled {
                    _ = panel_config.set_exclusive_zone(helper, false);
                    _ = panel_config.set_autohide(helper, AutoHide::OnOverlap);
                } else {
                    _ = panel_config.set_exclusive_zone(helper, true);
                    _ = panel_config.set_autohide(helper, AutoHide::Never);
                }
            }
            Message::PanelAnchor(i) => {
                if let Some(anchor) = [
                    PanelAnchor::Left,
                    PanelAnchor::Right,
                    PanelAnchor::Top,
                    PanelAnchor::Bottom,
                ]
                .iter()
                .find(|a| Anchor(**a).to_string() == self.anchors[i])
                {
                    _ = panel_config.set_anchor(helper, *anchor);
                }
            }
            Message::Output(i) => {
                if i == 0 {
                    _ = panel_config.set_output(helper, CosmicPanelOuput::All);
                } else {
                    _ = panel_config
                        .set_output(helper, CosmicPanelOuput::Name(self.outputs[i].clone()));
                }
            }
            Message::AnchorGap(enabled) => {
                _ = panel_config.set_anchor_gap(helper, enabled);

                if enabled {
                    _ = panel_config.set_margin(helper, 4);
                } else {
                    _ = panel_config.set_margin(helper, 0);
                }
                let theme = cosmic::theme::system_preference();
                let theme = theme.cosmic();
                let radius = theme.corner_radii.radius_xl[0] as u32;
                let new_radius = if enabled {
                    radius
                } else if !panel_config.expand_to_edges {
                    radius.min(12)
                } else {
                    0
                };
                _ = panel_config.set_border_radius(helper, new_radius).unwrap();
            }
            Message::PanelThickness(px) => {
                let size = PanelSize::Custom(px.clamp(
                    cosmic_panel_config::MIN_PANEL_THICKNESS,
                    MAX_PANEL_THICKNESS,
                ));
                self.size = size.clone();
                _ = panel_config.set_size(helper, size);
                // Wings and centre may carry a size of their own, which would keep their
                // applets at the old height on a bar that just changed.
                _ = panel_config.set_size_center(helper, None);
                _ = panel_config.set_size_wings(helper, None);
            }
            Message::Appearance(a) => {
                if let Some(b) = [Appearance::Match, Appearance::Light, Appearance::Dark]
                    .iter()
                    .find(|b| b.to_string() == self.backgrounds[a])
                {
                    _ = panel_config.set_background(helper, (*b).into());
                }
            }
            Message::Look(i) => {
                let look = if i == 1 {
                    PanelLook::Island
                } else {
                    PanelLook::Bar
                };
                _ = panel_config.set_look(helper, look);

                // The look is what applets read, but on its own it would leave the panel
                // the same shape it was. Carry the shape with it, to the values the two
                // looks are defined by.
                let island = look == PanelLook::Island;
                let theme = cosmic::theme::system_preference();
                let radius = theme.cosmic().corner_radii.radius_xl[0] as u32;

                _ = panel_config.set_expand_to_edges(helper, !island);
                _ = panel_config.set_padding(helper, u32::from(island) * 4);
                _ = panel_config.set_border_radius(
                    helper,
                    if panel_config.anchor_gap {
                        radius
                    } else if island {
                        radius.min(12)
                    } else {
                        0
                    },
                );
            }
            Message::ExtendToEdge(enabled) => {
                _ = panel_config.set_expand_to_edges(helper, enabled);

                let theme = cosmic::theme::system_preference();
                let theme = theme.cosmic();
                let radius = theme.corner_radii.radius_xl[0] as u32;
                let new_radius = if panel_config.anchor_gap {
                    radius
                } else if !enabled {
                    radius.min(12)
                } else {
                    0
                };
                _ = panel_config.set_border_radius(helper, new_radius).unwrap();
            }
            Message::OpacityRequest(opacity) => {
                panel_config.opacity = opacity;

                if self.opacity_changing {
                    return Task::none();
                }

                self.opacity_changing = true;
                return cosmic::Task::future(async move {
                    tokio::time::sleep(Duration::from_millis(125)).await;
                    Message::OpacityApply
                });
            }

            Message::OpacityApply => {
                self.opacity_changing = false;
                _ = helper.set("opacity", panel_config.opacity);
            }

            Message::OutputAdded(name, output) => {
                self.outputs.push(name.clone());
                self.outputs_map.insert(output.id(), (name, output));
                return Task::none();
            }
            Message::OutputRemoved(output) => {
                if let Some((name, _)) = self.outputs_map.remove(&output.id())
                    && let Some(pos) = self.outputs.iter().position(|o| o == &name)
                {
                    self.outputs.remove(pos);
                }
            }
            Message::PanelConfig(c) => {
                self.size = c.size.clone();
                self.panel_config = Some(*c);
                return Task::none();
            }
            Message::ResetPanel => {}
            Message::Surface(_) => {
                unimplemented!()
            }
        }

        Task::none()
    }
}
