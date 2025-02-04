// Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::devices::virtio::pmem::device::{Pmem, PmemError};

/// Errors associated wit the operations allowed on a pmem device
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum PmemConfigError {
    /// Unable to create the virtio-pmem device
    CreatePmemDevice(#[from] PmemError),
    /// Error accessing underlying file
    File(std::io::Error),
}

/// Use this structure to setup a Pmem device before boothing the kernel.
#[derive(Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PmemDeviceConfig {
    /// Unique identifier of the device.
    pub drive_id: String,
    /// Path of the drive.
    pub path_on_host: String,
    /// If set to true the device will be read-only.
    pub is_read_only: bool,
}

/// Only provided fields will be updated. I.e. if any optional fields
/// are missing, they will not be updated.
#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmemDeviceUpdateConfig {
    /// The drive ID, as provided by the user at creation time.
    pub drive_id: String,

    /// New block file path on the host. Only provided data will be updated.
    pub path_on_host: Option<String>,
}

/// Wrapper for the collection that holds all the Pmem devices.
#[derive(Debug, Default)]
pub struct PmemBuilder {
    /// The list of pmem devices
    pub devices: Vec<Arc<Mutex<Pmem>>>,
}

impl PmemBuilder {
    /// Constructor for Pmem devices collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a device from the config
    pub fn build(&mut self, config: PmemDeviceConfig) -> Result<(), PmemConfigError> {
        let size = std::fs::metadata(&config.path_on_host)
            .map_err(PmemConfigError::File)?
            .len();
        let pmem = Pmem::new(
            size.try_into().unwrap(),
            config.drive_id,
            config.path_on_host,
            config.is_read_only,
        )?;
        self.devices.push(Arc::new(Mutex::new(pmem)));
        Ok(())
    }

    /// Returns a vec with the structures used to configure the devices.
    pub fn configs(&self) -> Vec<PmemDeviceConfig> {
        self.devices
            .iter()
            .map(|b| b.lock().unwrap().config())
            .collect()
    }
}
