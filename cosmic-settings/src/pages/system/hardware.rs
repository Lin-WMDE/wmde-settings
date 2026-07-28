// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: the Hardware page.
//!
//! Everything it shows comes from the `wmde-hardware` crate, which reads sysfs, procfs,
//! DMI and the system hwdata database and needs no privileges. Rows that the kernel keeps
//! at mode 0400 are rendered as "requires root" rather than hidden, so it is clear they
//! were not simply missed.
//!
//! The page is read-only on purpose: the only message it has is the collected data
//! arriving. Anything actionable about hardware belongs on the page that owns the action
//! (Displays, Network, Sound), not here.

use cosmic::Task;
use cosmic::iced::Alignment;
use cosmic::widget::{settings, text};
use cosmic_settings_page::{self as page, Section, section};
use slotmap::SlotMap;
use wmde_hardware::util::{fmt_bytes_dec, fmt_khz};
use wmde_hardware::{FirmwareType, SecureBoot, SysInfo, Value};

#[derive(Clone, Debug)]
pub enum Message {
    Info(Box<SysInfo>),
}

impl From<Message> for crate::app::Message {
    fn from(message: Message) -> Self {
        crate::pages::Message::Hardware(message).into()
    }
}

impl From<Message> for crate::pages::Message {
    fn from(message: Message) -> Self {
        crate::pages::Message::Hardware(message)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    entity: page::Entity,
    info: SysInfo,
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
        Some(vec![
            sections.insert(firmware()),
            sections.insert(processor()),
            sections.insert(storage()),
            sections.insert(graphics()),
            sections.insert(network()),
            sections.insert(usb()),
            sections.insert(pci()),
        ])
    }

    fn info(&self) -> page::Info {
        page::Info::new("hardware", "computer-chip-symbolic")
            .title(fl!("hardware"))
            .description(fl!("hardware", "desc"))
    }

    fn on_enter(&mut self) -> Task<crate::pages::Message> {
        // Blocking sysfs work, and this page is also entered from search results, so it
        // goes to the blocking pool rather than the UI thread.
        let (task, handle) = Task::future(async move {
            let info = tokio::task::spawn_blocking(SysInfo::collect)
                .await
                .unwrap_or_default();
            crate::pages::Message::Hardware(Message::Info(Box::new(info)))
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
            Message::Info(info) => self.info = *info,
        }

        Task::none()
    }
}

// ------------------------------------------------------------------ firmware

fn firmware() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        vendor = fl!("hardware-firmware", "vendor");
        version = fl!("hardware-firmware", "version");
        date = fl!("hardware-firmware", "date");
        board = fl!("hardware-firmware", "board");
        chassis = fl!("hardware-firmware", "chassis");
        firmware_type = fl!("hardware-firmware", "type");
        secure_boot = fl!("hardware-firmware", "secure-boot");
        tpm = fl!("hardware-firmware", "tpm");
        microcode = fl!("hardware-firmware", "microcode");
    });

    Section::default()
        .title(fl!("hardware-firmware"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let desc = &section.descriptions;
            let fw = &page.info.firmware;

            settings::section()
                .title(&section.title)
                .add(row(&desc[vendor], value(&fw.bios_vendor)))
                .add(row(&desc[version], value(&fw.bios_version)))
                .add(row(&desc[date], value(&fw.bios_date)))
                .add(row(&desc[board], board_label(fw)))
                .add(row(&desc[chassis], chassis_label(fw)))
                .add(row(&desc[firmware_type], firmware_type_label(fw)))
                .add(row(&desc[secure_boot], secure_boot_label(fw.secure_boot)))
                .add(row(&desc[tpm], tpm_label(fw)))
                .add(row(&desc[microcode], value(&fw.cpu_microcode)))
                .into()
        })
}

// Suffixed `_label`: `crate::slab!` above declares locals named after every description
// key, so a helper called `board` or `chassis` would be shadowed by a usize.
fn board_label(fw: &wmde_hardware::Firmware) -> String {
    match (fw.board_vendor.as_str(), fw.board_name.as_str()) {
        (Some(vendor), Some(name)) => format!("{vendor} {name}"),
        (None, Some(name)) => name.to_owned(),
        (Some(vendor), None) => vendor.to_owned(),
        (None, None) => unknown(),
    }
}

fn chassis_label(fw: &wmde_hardware::Firmware) -> String {
    match (fw.chassis_type_name, fw.chassis_type) {
        (Some(name), Some(code)) => format!("{name} ({code})"),
        (None, Some(code)) => code.to_string(),
        _ => unknown(),
    }
}

fn firmware_type_label(fw: &wmde_hardware::Firmware) -> String {
    match (fw.firmware_type, fw.efi_bits) {
        (FirmwareType::Uefi, Some(bits)) => format!("UEFI ({bits}-bit)"),
        (FirmwareType::Uefi, None) => "UEFI".to_owned(),
        (FirmwareType::LegacyBios, _) => fl!("hardware-firmware", "legacy"),
        (FirmwareType::Unknown, _) => unknown(),
    }
}

fn secure_boot_label(state: SecureBoot) -> String {
    match state {
        SecureBoot::Enabled => fl!("hardware-state", "enabled"),
        SecureBoot::Disabled => fl!("hardware-state", "disabled"),
        SecureBoot::SetupMode => fl!("hardware-firmware", "setup-mode"),
        SecureBoot::NotUefi => fl!("hardware-state", "not-applicable"),
        SecureBoot::Unknown => unknown(),
    }
}

fn tpm_label(fw: &wmde_hardware::Firmware) -> String {
    match &fw.tpm {
        Some(tpm) => match &tpm.version_major {
            Some(version) => format!("{} {version}", fl!("hardware-state", "present")),
            None => fl!("hardware-state", "present"),
        },
        None => fl!("hardware-state", "absent"),
    }
}

// ------------------------------------------------------------------ processor

fn processor() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        model = fl!("hardware-processor", "model");
        cores = fl!("hardware-processor", "cores");
        frequency = fl!("hardware-processor", "frequency");
    });

    Section::default()
        .title(fl!("hardware-processor"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let desc = &section.descriptions;
            let cpu = &page.info.summary.cpu;

            let counts = match (cpu.physical_cores, cpu.logical_threads, cpu.sockets) {
                (Some(cores), Some(threads), Some(sockets)) if sockets > 1 => {
                    format!("{cores} / {threads} ({sockets})")
                }
                (Some(cores), Some(threads), _) => format!("{cores} / {threads}"),
                _ => unknown(),
            };

            settings::section()
                .title(&section.title)
                .add(row(&desc[model], value(&cpu.model)))
                .add(row(&desc[cores], counts))
                .add(row(
                    &desc[frequency],
                    cpu.max_freq_khz.map_or_else(unknown, fmt_khz),
                ))
                .into()
        })
}

// ------------------------------------------------------------------ storage

fn storage() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        empty = fl!("hardware-none");
    });

    Section::default()
        .title(fl!("hardware-storage"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let mut view = settings::section().title(&section.title);

            if page.info.disks.is_empty() {
                return view
                    .add(row(&section.descriptions[empty], String::new()))
                    .into();
            }

            for disk in &page.info.disks {
                let mut detail = vec![
                    fmt_bytes_dec(disk.size_bytes),
                    disk.kind.as_str().to_owned(),
                ];
                if let Some(firmware) = &disk.firmware {
                    detail.push(firmware.clone());
                }
                view = view.add(row(&disk.title(), detail.join(", ")));

                // One line per mounted partition: the usage figure is the reason anybody
                // opens this section.
                for partition in &disk.partitions {
                    if partition.mountpoints.is_empty() {
                        continue;
                    }
                    let mounts = partition
                        .mountpoints
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut detail = vec![mounts];
                    if let Some(fstype) = &partition.fstype {
                        detail.push(fstype.clone());
                    }
                    if let Some(usage) = partition.usage {
                        detail.push(fl!(
                            "hardware-usage",
                            used = fmt_bytes_dec(usage.used),
                            total = fmt_bytes_dec(usage.total),
                            percent = (usage.percent() as usize)
                        ));
                    }
                    view = view.add(row(&partition.name, detail.join(", ")));
                }
            }

            view.into()
        })
}

// ------------------------------------------------------------------ graphics

fn graphics() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        empty = fl!("hardware-none");
    });

    Section::default()
        .title(fl!("hardware-graphics"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let mut view = settings::section().title(&section.title);
            let graphics = &page.info.graphics;

            if graphics.gpus.is_empty() && graphics.displays.is_empty() {
                return view
                    .add(row(&section.descriptions[empty], String::new()))
                    .into();
            }

            for gpu in &graphics.gpus {
                let driver = match (&gpu.driver, &gpu.driver_version) {
                    (Some(driver), Some(version)) => format!("{driver} {version}"),
                    (Some(driver), None) => driver.clone(),
                    _ => unknown(),
                };
                view = view.add(row(&gpu.title(), driver));
            }

            for display in &graphics.displays {
                if !display.connected {
                    continue;
                }
                let mut detail = Vec::new();
                if let Some(make) = &display.make {
                    detail.push(make.clone());
                }
                if let Some(model) = &display.model {
                    detail.push(model.clone());
                }
                if let Some(mode) = &display.preferred_mode {
                    detail.push(mode.clone());
                }
                if detail.is_empty() {
                    detail.push(fl!("hardware-state", "connected"));
                }
                view = view.add(row(&display.connector, detail.join(", ")));
            }

            view.into()
        })
}

// ------------------------------------------------------------------ network

fn network() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        empty = fl!("hardware-none");
        hidden = fl!("hardware-network", "virtual-hidden");
    });

    Section::default()
        .title(fl!("hardware-network"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let desc = &section.descriptions;
            let mut view = settings::section().title(&section.title);

            let physical: Vec<_> = page
                .info
                .network
                .iter()
                .filter(|iface| !iface.is_virtual())
                .collect();
            let hidden_count = page.info.network.len() - physical.len();

            if physical.is_empty() {
                view = view.add(row(&desc[empty], String::new()));
            }

            for iface in physical {
                let mut detail = vec![iface.state.as_str().to_owned()];
                if let Some(mac) = &iface.mac {
                    detail.push(mac.clone());
                }
                if let Some(speed) = iface.speed_mbps {
                    detail.push(format!("{speed} Mbps"));
                }
                if let Some(driver) = &iface.driver {
                    detail.push(driver.clone());
                }
                for address in &iface.ipv4 {
                    detail.push(address.to_string());
                }
                view = view.add(row(&iface.name, detail.join(", ")));
            }

            // Counted rather than listed: a developer machine routinely has forty veth
            // pairs, and they would bury everything else.
            if hidden_count > 0 {
                view = view.add(row(&desc[hidden], hidden_count.to_string()));
            }

            view.into()
        })
}

// ------------------------------------------------------------------ usb

fn usb() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        empty = fl!("hardware-none");
    });

    Section::default()
        .title(fl!("hardware-usb"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let mut view = settings::section().title(&section.title);

            if page.info.usb.is_empty() {
                return view
                    .add(row(&section.descriptions[empty], String::new()))
                    .into();
            }

            for device in &page.info.usb {
                let mut detail = vec![device.id_string()];
                if let Some(speed) = device.speed_label() {
                    detail.push(speed);
                }
                view = view.add(row(&device.title(), detail.join(", ")));
            }

            view.into()
        })
}

// ------------------------------------------------------------------ pci

fn pci() -> Section<crate::pages::Message> {
    crate::slab!(descriptions {
        empty = fl!("hardware-none");
    });

    Section::default()
        .title(fl!("hardware-pci"))
        .descriptions(descriptions)
        .view::<Page>(move |_binder, page, section| {
            let mut view = settings::section().title(&section.title);

            if page.info.pci.is_empty() {
                return view
                    .add(row(&section.descriptions[empty], String::new()))
                    .into();
            }

            for device in &page.info.pci {
                let mut detail = vec![device.title()];
                if let Some(class) = &device.class_name {
                    detail.push(class.clone());
                }
                if let Some(driver) = &device.raw.driver {
                    detail.push(driver.clone());
                }
                view = view.add(row(&device.raw.slot, detail.join(", ")));
            }

            view.into()
        })
}

// ------------------------------------------------------------------ helpers

/// One label/value row, laid out like every other settings row.
fn row(label: &str, value: String) -> cosmic::widget::FlexRow<'static, crate::pages::Message> {
    settings::flex_item(label.to_owned(), text::body(value)).align_items(Alignment::Center)
}

fn value(value: &Value) -> String {
    match value {
        Value::Known(text) => text.clone(),
        Value::RequiresRoot => fl!("hardware-state", "requires-root"),
        Value::Unavailable => unknown(),
    }
}

fn unknown() -> String {
    fl!("unknown")
}
