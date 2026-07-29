// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

//! The About page's view model.
//!
//! WMDE: this file used to collect the facts itself - two `lspci -nn` subprocesses, the
//! `sysinfo` crate, and a DMI read that reported `board_name` where `product_name` was
//! meant. All of that moved to the `wmde-hardware` crate, which reads sysfs directly, is
//! unit tested, and is shared with the Hardware page. What is left here is the mapping
//! into the strings About renders, so the page itself barely had to change.

use wmde_hardware::util::{fmt_bytes_bin, fmt_bytes_dec};
use wmde_hardware::{SysInfo, Value};

#[must_use]
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub desktop_environment: String,
    pub device_name: String,
    pub disk_capacity: String,
    pub graphics: Vec<String>,
    pub hardware_model: String,
    pub memory: String,
    pub operating_system: String,
    pub os_architecture: String,
    pub kernel_version: String,
    pub processor: String,
    pub windowing_system: String,
    /// WMDE: the board behind the machine name. On a self-built desktop the DMI product
    /// name is OEM junk and gets filtered out, so this is often the only real identifier.
    pub motherboard: String,
    /// WMDE: swap total, shown next to memory.
    pub swap: String,
}

impl Info {
    pub async fn load() -> Info {
        // Collection is blocking sysfs/procfs I/O, and the About page is also entered
        // from search results, so it must not run on the UI thread.
        let collected = tokio::task::spawn_blocking(SysInfo::collect)
            .await
            .unwrap_or_default();
        Info::from(&collected)
    }
}

impl From<&SysInfo> for Info {
    fn from(info: &SysInfo) -> Self {
        let summary = &info.summary;
        Info {
            // The WMDE stack exposes no runtime version anywhere, so this app's own
            // version stands in for the desktop's. Deliberately not asking pacman.
            //
            // Read HERE and not in the collection layer: `env!` expands in the crate it is
            // written in, so once this moved into `wmde-hardware` it started reporting
            // that crate's 0.1.0 instead of the app's version.
            desktop_environment: format!("WMDE {}", env!("CARGO_PKG_VERSION")),
            device_name: value(&summary.hostname),
            disk_capacity: if summary.disk_total_bytes > 0 {
                fmt_bytes_dec(summary.disk_total_bytes)
            } else {
                unknown()
            },
            graphics: summary.gpu_names.clone(),
            hardware_model: summary.machine().unwrap_or_else(unknown),
            memory: summary.mem_total_bytes.map_or_else(unknown, fmt_bytes_bin),
            operating_system: value(&summary.os_name),
            os_architecture: value(&summary.architecture),
            kernel_version: value(&summary.kernel),
            processor: processor(info),
            windowing_system: summary.display_server.as_str().to_owned(),
            motherboard: value(&summary.board),
            swap: summary.swap_total_bytes.map_or_else(unknown, fmt_bytes_bin),
        }
    }
}

/// Processor model plus the core and thread count, which is what makes the row useful on
/// a hybrid CPU where the model name alone says nothing about the split.
fn processor(info: &SysInfo) -> String {
    let cpu = &info.summary.cpu;
    let Some(model) = cpu.model.as_str() else {
        return unknown();
    };
    match (cpu.physical_cores, cpu.logical_threads) {
        (Some(cores), Some(threads)) => format!("{model} ({cores} / {threads})"),
        _ => model.to_owned(),
    }
}

fn value(value: &Value) -> String {
    value.as_str().map_or_else(unknown, ToOwned::to_owned)
}

fn unknown() -> String {
    fl!("unknown")
}
