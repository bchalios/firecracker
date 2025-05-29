// Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};

use event_manager::MutEventSubscriber;
use kvm_ioctls::{IoEventAddress, NoDatamatch, VmFd};
use log::debug;
use pci::{PciBarRegionType, PciDevice, PciDeviceError, PciRootError};
use serde::{Deserialize, Serialize};
use vm_device::BusError;
use vm_device::interrupt::{InterruptManager, MsiIrqGroupConfig};
use vmm_sys_util::errno;

use crate::device_manager::interrupt::MsiInterruptManager;
use crate::device_manager::resources::ResourceAllocator;
use crate::devices::pci::PciSegment;
use crate::devices::virtio;
use crate::devices::virtio::device::VirtioDevice;
use crate::devices::virtio::transport::pci::device::{VirtioPciDevice, VirtioPciDeviceError};
use crate::vstate::memory::GuestMemoryMmap;

pub struct PciDevices {
    /// Interrupt manager for MSIx
    msix_interrupt_manager: Arc<dyn InterruptManager<GroupConfig = MsiIrqGroupConfig>>,
    /// PCIe segment of the VMM, if PCI is enabled. We currently support a single PCIe segment.
    pub pci_segment: Option<PciSegment>,
}

impl std::fmt::Debug for PciDevices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PciDevices")
            .field("pci_segment", &self.pci_segment)
            .finish()
    }
}

#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum PciManagerError {
    /// Resource allocation error: {0}
    ResourceAllocation(#[from] vm_allocator::Error),
    /// Bus error: {0}
    Bus(#[from] BusError),
    /// PCI Root bus error: {0}
    Root(#[from] PciRootError),
    /// Virtio PCI device error: {0}
    VirtioDevice(#[from] VirtioPciDeviceError),
    /// PCI device error: {0}
    PciDevice(#[from] PciDeviceError),
    /// Kvm error: {0}
    Kvm(#[from] errno::Error),
}

impl PciDevices {
    pub fn new(resource_allocator: &Arc<ResourceAllocator>, vm_fd: &Arc<VmFd>) -> Self {
        let msix_interrupt_manager = Arc::new(MsiInterruptManager::new(
            resource_allocator.clone(),
            vm_fd.clone(),
        ));

        Self {
            msix_interrupt_manager,
            pci_segment: None,
        }
    }

    pub fn attach_pci_segment(
        &mut self,
        resource_allocator: &Arc<ResourceAllocator>,
    ) -> Result<(), PciManagerError> {
        // We only support a single PCIe segment. Calling this function twice is a Firecracker
        // internal error.
        assert!(self.pci_segment.is_none());

        // Currently we don't assign any IRQs to PCI devices. We will be using MSI-X interrupts
        // only.
        let pci_segment = PciSegment::new(0, resource_allocator, &[0u8; 32])?;
        self.pci_segment = Some(pci_segment);

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    /// Attaches a VirtioDevice with MMIO transport
    pub(crate) fn attach_pci_virtio_device<
        T: 'static + VirtioDevice + MutEventSubscriber + Debug,
    >(
        &mut self,
        mem: &GuestMemoryMmap,
        vm_fd: &VmFd,
        id: String,
        device: Arc<Mutex<T>>,
        resource_allocator: &ResourceAllocator,
    ) -> Result<(), PciManagerError> {
        // We should only be reaching this point if PCI is enabled
        let pci_segment = self.pci_segment.as_ref().unwrap();
        let pci_device_bdf = pci_segment.next_device_bdf()?;
        debug!("Allocating BDF: {pci_device_bdf:?} for device");

        let (msix_num, device_type) = {
            let locked_device = device.lock().expect("Poisoned lock");
            // Allows support for one MSI-X vector per device queue, plus one vector for notifying the
            // device about configuration changes.
            let msix_num: u16 = (locked_device.queues().len() + 1).try_into().unwrap();
            debug!("Creating {msix_num} MSI-X vectors for device");
            let device_type = locked_device.device_type();
            (msix_num, device_type)
        };

        // Create the transport
        let mut virtio_device = VirtioPciDevice::new(
            id.clone(),
            mem.clone(),
            device,
            msix_num,
            &self.msix_interrupt_manager,
            pci_device_bdf.into(),
            true,
            None,
        )?;

        // Allocate bars
        let mut mmio32_allocator = resource_allocator
            .mmio32_memory
            .lock()
            .expect("Poisoned lock");
        let mut mmio64_allocator = resource_allocator
            .mmio64_memory
            .lock()
            .expect("Poisoned lock");

        let bars =
            virtio_device.allocate_bars(&mut mmio32_allocator, &mut mmio64_allocator, None)?;

        let virtio_device = Arc::new(Mutex::new(virtio_device));
        pci_segment
            .pci_bus
            .lock()
            .expect("Poisoned lock")
            .add_device(pci_device_bdf.device() as u32, virtio_device.clone())?;

        for bar in &bars {
            match bar.region_type() {
                PciBarRegionType::IoRegion => {
                    #[cfg(target_arch = "x86_64")]
                    resource_allocator.pio_bus.insert(
                        virtio_device.clone(),
                        bar.addr(),
                        bar.size(),
                    )?;
                    #[cfg(target_arch = "aarch64")]
                    log::error!("pci: We do not support I/O region allocation")
                }
                PciBarRegionType::Memory32BitRegion | PciBarRegionType::Memory64BitRegion => {
                    resource_allocator.mmio_bus.insert(
                        virtio_device.clone(),
                        bar.addr(),
                        bar.size(),
                    )?;
                }
            }
        }

        let locked_device = virtio_device.lock().expect("Poisoned lock");

        let bar_addr = locked_device.config_bar_addr();
        for (i, queue_evt) in locked_device
            .virtio_device()
            .lock()
            .expect("Poisoned lock")
            .queue_events()
            .iter()
            .enumerate()
        {
            const NOTIFICATION_BAR_OFFSET: u64 = 0x6000;
            const NOTIFY_OFF_MULTIPLIER: u64 = 4;
            let notify_base = bar_addr + NOTIFICATION_BAR_OFFSET;
            let io_addr = IoEventAddress::Mmio(notify_base + i as u64 * NOTIFY_OFF_MULTIPLIER);
            vm_fd.register_ioevent(queue_evt, &io_addr, NoDatamatch)?;
        }

        Ok(())
    }

    pub fn save(&self) -> PciDevicesState {
        PciDevicesState {
            pci_enabled: self.pci_segment.is_some(),
        }
    }

    pub fn restore(
        &mut self,
        state: &PciDevicesState,
        resource_allocator: &Arc<ResourceAllocator>,
    ) -> Result<(), PciManagerError> {
        if state.pci_enabled {
            self.attach_pci_segment(resource_allocator)?;
        }

        Ok(())
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct PciDevicesState {
    pci_enabled: bool,
}
