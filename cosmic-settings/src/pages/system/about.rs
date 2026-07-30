// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use cosmic::iced::alignment::Horizontal;
use cosmic::iced::{Alignment, ContentFit, Length};
use cosmic_settings_page::{self as page, Section, section};

use super::info::Info;
use cosmic::widget::{self, editable_input, icon, text};
use cosmic::{Apply, Task, theme};
use slotmap::SlotMap;

/// Shipped with the app: there is no WMDE brand mark in any icon theme, and
/// `distributor-logo` resolves to the Ubuntu logo in the primary one.
const WMDE_LOGO: &[u8] = include_bytes!("../../../../resources/wmde-logo.svg");

/// Width of the label column. Fixed on purpose: the colons line up only when every label
/// occupies the same width, and `view_fn` has no measuring pass to derive one from.
const LABEL_WIDTH: f32 = 220.0;

/// Width of the whole `label: value` block.
///
/// Measured off `ref/kde_about.png` and `ref/kde_about_settings.webp`: in both, every row
/// of every group shares ONE colon column, and the block that holds them is centred on the
/// content area. Groups are separate sections here and are laid out independently, so they
/// only agree on a colon column while they agree on a width - hence a constant, and hence
/// the same constant for the device row, which is otherwise far wider than the rest.
const BLOCK_WIDTH: f32 = 540.0;

/// Width of the device-name field, and of the hint under it.
///
/// `editable_input` parks its pencil at the right edge of the field, so the field's width
/// is the distance between the name and the icon. The hint takes the same width, so field
/// and hint read as one column instead of the hint sprawling wider than the thing it
/// explains.
const FIELD_WIDTH: f32 = 200.0;

#[derive(Clone, Debug)]
pub enum Message {
    Error(String),
    HostnameEdit(bool),
    HostnameInput(String),
    HostnameSubmit,
    HostnameSuccess(String),
    Info(Box<Info>),
}

impl From<Message> for crate::app::Message {
    fn from(message: Message) -> Self {
        crate::pages::Message::About(message).into()
    }
}

impl From<Message> for crate::pages::Message {
    fn from(message: Message) -> Self {
        crate::pages::Message::About(message)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    entity: page::Entity,
    editing_device_name: bool,
    hostname_input: String,
    info: Info,
    on_enter_handle: Option<cosmic::iced::task::Handle>,
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
        // Software before hardware, as the reference lays it out.
        Some(vec![
            sections.insert(header()),
            sections.insert(device()),
            sections.insert(os()),
            sections.insert(hardware()),
        ])
    }

    fn info(&self) -> page::Info {
        page::Info::new("about", "help-about-symbolic")
            .title(fl!("about"))
            .description(fl!("xdg-entry-about-comment"))
    }

    fn on_enter(&mut self) -> Task<crate::pages::Message> {
        let (task, handle) = Task::future(async move {
            let info = Info::load().await;
            crate::pages::Message::About(Message::Info(Box::new(info)))
        })
        .abortable();

        self.on_enter_handle = Some(handle);
        task
    }

    fn on_leave(&mut self) -> Task<crate::pages::Message> {
        if let Some(handle) = self.on_enter_handle.take() {
            handle.abort();
        }

        Task::none()
    }
}

impl Page {
    pub fn update(&mut self, message: Message) -> cosmic::app::Task<crate::Message> {
        match message {
            Message::HostnameEdit(editing) => {
                self.editing_device_name = editing;
            }

            Message::HostnameInput(hostname) => {
                self.hostname_input = hostname;
            }

            Message::HostnameSubmit => return self.hostname_submit(),

            Message::Info(info) => {
                self.info = *info;
                self.hostname_input = self.info.device_name.clone();
            }

            Message::Error(_why) => {
                self.hostname_input = self.info.device_name.clone();
                // TODO: display errors
            }

            Message::HostnameSuccess(name) => {
                self.info.device_name = name;
            }
        }

        Task::none()
    }

    fn hostname_submit(&mut self) -> cosmic::app::Task<crate::app::Message> {
        if self.hostname_input == self.info.device_name {
            return Task::none();
        }

        if !hostname_validator::is_valid(&self.hostname_input) {
            return Task::none();
        }

        self.editing_device_name = false;
        let hostname = self.hostname_input.clone();

        cosmic::Task::future(async move { set_hostname(hostname).await })
            .map(crate::app::Message::from)
            .map(Into::into)
    }
}

/// Sets the system hostname via D-Bus.
async fn set_hostname(hostname: String) -> Message {
    match set_hostname_impl(&hostname).await {
        Ok(()) => Message::HostnameSuccess(hostname),
        Err(err) => {
            tracing::error!("failed to set hostname: {}", err);
            Message::Error(err)
        }
    }
}

/// Implementation of hostname setting that uses Result for cleaner error handling.
async fn set_hostname_impl(hostname: &str) -> Result<(), String> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|e| format!("failed to establish connection to dbus: {}", e))?;

    let hostname1 = hostname1_zbus::Hostname1Proxy::new(&connection)
        .await
        .map_err(|e| format!("failed to connect to org.freedesktop.hostname1: {}", e))?;

    hostname1
        .set_static_hostname(hostname, false)
        .await
        .map_err(|e| format!("failed to set static hostname: {}", e))?;

    Ok(())
}

/// WMDE: the header that opens the page - logo, operating system, machine.
///
/// A section does not have to be a settings card: `view_fn` may return any element, which
/// is the only way to get something non-tabular onto a settings page. `search_ignore`
/// keeps it out of the search index, since it carries data rather than a label.
///
/// The operating system is the headline and the machine the caption under it, not the
/// other way round: the machine is named again, in full, by the `model` and `motherboard`
/// lines of the hardware group.
fn header() -> Section<crate::pages::Message> {
    Section::default()
        .search_ignore()
        .view::<Page>(move |_binder, page, _section| {
            let spacing = theme::spacing();

            let logo = icon::icon(icon::from_svg_bytes(WMDE_LOGO))
                .size(96)
                .width(Length::Fixed(96.0))
                .height(Length::Fixed(96.0))
                .content_fit(ContentFit::Contain);

            let names = widget::column::with_capacity(2)
                .push(text::title2(page.info.operating_system.clone()))
                .push(text::body(page.info.hardware_model.clone()))
                .spacing(spacing.space_xxs);

            widget::row::with_capacity(2)
                .push(logo)
                .push(names)
                .spacing(spacing.space_m)
                .align_y(Alignment::Center)
                .apply(widget::container)
                .width(Length::Fill)
                .align_x(Horizontal::Center)
                .into()
        })
}

/// One `label: value` line. The label is right aligned against the fixed label column, so
/// every colon on the page sits at the same x.
fn detail<'a, M: 'static>(
    label: &str,
    value: impl Into<cosmic::Element<'a, M>>,
) -> cosmic::Element<'a, M> {
    widget::row::with_capacity(2)
        .push(
            text::body(format!("{label}:"))
                .align_x(Horizontal::Right)
                .width(Length::Fixed(LABEL_WIDTH)),
        )
        .push(value)
        .spacing(theme::spacing().space_xs)
        // Top, not centre: a value taller than one line would otherwise push its label
        // down into the middle of the cell, away from the line it names. Single-line rows
        // cannot tell the difference - label and value are the same height.
        .align_y(Alignment::Start)
        .width(Length::Fill)
        .into()
}

/// Centre a block of rows on the page.
///
/// Same idiom as `SettingsApp::page_container`: a child capped by `max_width` inside a
/// filling parent. The cap is what makes the block a known width, and a known width is
/// what makes the colon column of one group land on the colon column of the next.
fn block<'a, M: 'static>(content: impl Into<cosmic::Element<'a, M>>) -> cosmic::Element<'a, M> {
    widget::container(content)
        .max_width(BLOCK_WIDTH)
        .width(Length::Fill)
        .apply(widget::container)
        .center_x(Length::Fill)
        .into()
}

/// A titled group of `detail` lines. The heading is centred over the block, the lines keep
/// the common left edge that the block gives them.
fn group<'a, M: 'static>(title: &str, rows: Vec<cosmic::Element<'a, M>>) -> cosmic::Element<'a, M> {
    let spacing = theme::spacing();

    block(
        widget::column::with_capacity(2)
            .push(
                text::heading(title.to_owned())
                    .width(Length::Fill)
                    .align_x(Horizontal::Center),
            )
            .push(widget::column::with_children(rows).spacing(spacing.space_xxs))
            .spacing(spacing.space_xs),
    )
}

fn device() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        device = fl!("about-device");
        device_desc = fl!("about-device", "desc");
    });

    Section::default()
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let desc = &section.descriptions;

            let hostname_input = editable_input(
                "",
                &page.hostname_input,
                page.editing_device_name,
                Message::HostnameEdit,
            )
            .width(FIELD_WIDTH)
            .on_input(Message::HostnameInput)
            .on_unfocus(Message::HostnameSubmit)
            .on_submit(|_| Message::HostnameSubmit);

            // The hint rides under the field rather than under the label: keeping it in
            // the value cell leaves the label column, and so every colon, where it was.
            let value = widget::column::with_capacity(2)
                .push(hostname_input)
                .push(text::caption(&*desc[device_desc]).width(Length::Fixed(FIELD_WIDTH)))
                .spacing(theme::spacing().space_xxxs);

            block(detail(&desc[device], value)).map(crate::pages::Message::About)
        })
}

fn hardware() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        model = fl!("about-hardware", "model");
        motherboard = fl!("about-hardware", "motherboard");
        memory = fl!("about-hardware", "memory");
        swap = fl!("about-hardware", "swap");
        processor = fl!("about-hardware", "processor");
        graphics = fl!("about-hardware", "graphics");
        disk_capacity = fl!("about-hardware", "disk-capacity");
    });

    Section::default()
        .title(fl!("about-hardware"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let desc = &section.descriptions;

            let mut rows = Vec::with_capacity(6 + page.info.graphics.len());
            rows.push(detail(&desc[model], text::body(&page.info.hardware_model)));
            rows.push(detail(
                &desc[motherboard],
                text::body(&page.info.motherboard),
            ));
            rows.push(detail(&desc[memory], text::body(&page.info.memory)));
            rows.push(detail(&desc[swap], text::body(&page.info.swap)));
            rows.push(detail(&desc[processor], text::body(&page.info.processor)));

            for card in &page.info.graphics {
                rows.push(detail(&desc[graphics], text::body(card.as_str())));
            }

            rows.push(detail(
                &desc[disk_capacity],
                text::body(&page.info.disk_capacity),
            ));

            group(&section.title, rows)
        })
}

fn os() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        os = fl!("about-os", "os");
        os_arch = fl!("about-os", "os-architecture");
        kernel = fl!("about-os", "kernel");
        desktop = fl!("about-os", "desktop-environment");
        windowing_system = fl!("about-os", "windowing-system");
    });

    Section::default()
        .title(fl!("about-os"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let desc = &section.descriptions;

            group(
                &section.title,
                vec![
                    detail(&desc[os], text::body(&page.info.operating_system)),
                    detail(&desc[os_arch], text::body(&page.info.os_architecture)),
                    detail(&desc[kernel], text::body(&page.info.kernel_version)),
                    detail(&desc[desktop], text::body(&page.info.desktop_environment)),
                    detail(
                        &desc[windowing_system],
                        text::body(&page.info.windowing_system),
                    ),
                ],
            )
        })
}

// Related settings: reserved by upstream, not wired up in WMDE yet
// fn related() -> Section<crate::pages::Message> {
//     Section::default()
//         .title(fl!("about-related"))
//         .descriptions(vec![fl!("about-related", "support").into()])
//         .view::<Page>(move |_binder, _page, section| {
//             settings::section().title(&section.title)
//                 .add(settings::item(&*section.descriptions[0], text::body("TODO")))
//                 .into()
//         })
// }

// fn page(app: &crate::SettingsApp) -> &Page {
//     app.pages
//         .resource::<Page>()
//         .expect("missing system->about page")
// }
