// Copyright © 2019 Intel Corporation
// Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
//
// SPDX-License-Identifier: Apache-2.0 AND BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use kvm_bindings::{
    KVM_IRQ_ROUTING_IRQCHIP, KVM_IRQ_ROUTING_MSI, KVM_IRQCHIP_IOAPIC, KVM_MSI_VALID_DEVID,
    kvm_irq_routing, kvm_irq_routing_entry,
};
use kvm_ioctls::VmFd;
use log::debug;
use vm_device::interrupt::{
    InterruptIndex, InterruptManager, InterruptSourceConfig, InterruptSourceGroup,
    MsiIrqGroupConfig,
};
use vmm_sys_util::{errno, eventfd::EventFd};

use super::resources::ResourceAllocator;

#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum InterruptError {
    /// Error allocating resources: {0}
    Allocator(#[from] vm_allocator::Error),
    /// EventFd error: {0}
    EventFd(std::io::Error),
}

#[derive(Debug)]
pub struct InterruptRoute {
    gsi: u32,
    irq_fd: EventFd,
    registered: AtomicBool,
}

impl InterruptRoute {
    pub fn new(allocator: &ResourceAllocator) -> Result<Self, InterruptError> {
        let irq_fd = EventFd::new(libc::EFD_NONBLOCK).map_err(InterruptError::EventFd)?;
        let gsi = allocator.allocate_gsi(1)?[0];
        debug!("Allocated GSI {gsi} for interrupt route");

        Ok(InterruptRoute {
            gsi,
            irq_fd,
            registered: AtomicBool::new(false),
        })
    }

    pub fn enable(&self, vm: &VmFd) -> Result<(), errno::Error> {
        if !self.registered.load(Ordering::Acquire) {
            vm.register_irqfd(&self.irq_fd, self.gsi)?;
            self.registered.store(true, Ordering::Release);
        }

        Ok(())
    }

    pub fn disable(&self, vm: &VmFd) -> Result<(), errno::Error> {
        if self.registered.load(Ordering::Acquire) {
            vm.unregister_irqfd(&self.irq_fd, self.gsi)?;
            self.registered.store(false, Ordering::Release);
        }

        Ok(())
    }

    pub fn trigger(&self) -> Result<(), std::io::Error> {
        self.irq_fd.write(1)?;
        Ok(())
    }

    pub fn notifier(&self) -> &EventFd {
        &self.irq_fd
    }
}

pub struct RoutingEntry {
    route: kvm_irq_routing_entry,
    masked: bool,
}

pub struct MsiInterruptGroup {
    vm: Arc<VmFd>,
    gsi_msi_routes: Arc<Mutex<HashMap<u32, RoutingEntry>>>,
    irq_routes: HashMap<InterruptIndex, InterruptRoute>,
}

fn vec_with_size_in_bytes<T: Default>(size_in_bytes: usize) -> Vec<T> {
    let rounded_size = size_in_bytes.div_ceil(size_of::<T>());
    let mut v = Vec::with_capacity(rounded_size);
    v.resize_with(rounded_size, T::default);
    v
}

fn vec_with_array_field<T: Default, F>(count: usize) -> Vec<T> {
    let element_space = count * size_of::<F>();
    let vec_size_bytes = std::mem::size_of::<T>() + element_space;
    vec_with_size_in_bytes(vec_size_bytes)
}

impl MsiInterruptGroup {
    pub fn new(
        vm: Arc<VmFd>,
        gsi_msi_routes: Arc<Mutex<HashMap<u32, RoutingEntry>>>,
        irq_routes: HashMap<InterruptIndex, InterruptRoute>,
    ) -> Self {
        Self {
            vm,
            gsi_msi_routes,
            irq_routes,
        }
    }

    pub fn set_gsi_routes(
        &self,
        routes: &HashMap<u32, RoutingEntry>,
    ) -> Result<(), std::io::Error> {
        let mut entries = Vec::new();

        for i in 0..24 {
            let mut kvm_route = kvm_irq_routing_entry {
                gsi: i,
                type_: KVM_IRQ_ROUTING_IRQCHIP,
                ..Default::default()
            };

            kvm_route.u.irqchip.irqchip = KVM_IRQCHIP_IOAPIC;
            kvm_route.u.irqchip.pin = i;

            entries.push(kvm_route);
        }

        for (_, entry) in routes.iter() {
            if entry.masked {
                continue;
            }

            entries.push(entry.route)
        }

        let mut irq_routing =
            vec_with_array_field::<kvm_irq_routing, kvm_irq_routing_entry>(entries.len());
        irq_routing[0].nr = entries.len().try_into().unwrap();
        irq_routing[0].flags = 0;

        // SAFETY: irq_routing is initialized with `entries.len()` and now it is being turned into
        // entries_slice with entries.len() again. It is guaranteed to be large enough to hold
        // everything from entries.
        unsafe {
            let entries_slice: &mut [kvm_irq_routing_entry] =
                irq_routing[0].entries.as_mut_slice(entries.len());
            entries_slice.copy_from_slice(&entries);
        }

        self.vm.set_gsi_routing(&irq_routing[0])?;

        Ok(())
    }
}

impl InterruptSourceGroup for MsiInterruptGroup {
    fn enable(&self) -> vm_device::interrupt::Result<()> {
        for (_, route) in self.irq_routes.iter() {
            route.enable(&self.vm)?;
        }

        Ok(())
    }

    fn disable(&self) -> vm_device::interrupt::Result<()> {
        for (_, route) in self.irq_routes.iter() {
            route.disable(&self.vm)?;
        }

        Ok(())
    }

    fn trigger(&self, index: InterruptIndex) -> vm_device::interrupt::Result<()> {
        if let Some(route) = self.irq_routes.get(&index) {
            return route.trigger();
        }

        Err(std::io::Error::other(format!(
            "trigger: Invalid interrupt index {index}"
        )))
    }

    fn notifier(&self, index: InterruptIndex) -> Option<&EventFd> {
        if let Some(route) = self.irq_routes.get(&index) {
            return Some(route.notifier());
        }

        None
    }

    fn update(
        &self,
        index: InterruptIndex,
        config: InterruptSourceConfig,
        masked: bool,
        set_gsi: bool,
    ) -> vm_device::interrupt::Result<()> {
        if let Some(route) = self.irq_routes.get(&index) {
            let kvm_route = match &config {
                InterruptSourceConfig::MsiIrq(cfg) => {
                    let mut kvm_route = kvm_irq_routing_entry {
                        gsi: route.gsi,
                        type_: KVM_IRQ_ROUTING_MSI,
                        ..Default::default()
                    };

                    kvm_route.u.msi.address_lo = cfg.low_addr;
                    kvm_route.u.msi.address_hi = cfg.high_addr;
                    kvm_route.u.msi.data = cfg.data;

                    if self.vm.check_extension(kvm_ioctls::Cap::MsiDevid) {
                        // On AArch64, there is limitation on the range of the 'devid',
                        // it cannot be greater than 65536 (the max of u16).
                        //
                        // BDF cannot be used directly, because 'segment' is in high
                        // 16 bits. The layout of the u32 BDF is:
                        // |---- 16 bits ----|-- 8 bits --|-- 5 bits --|-- 3 bits --|
                        // |      segment    |     bus    |   device   |  function  |
                        //
                        // Now that we support 1 bus only in a segment, we can build a
                        // 'devid' by replacing the 'bus' bits with the low 8 bits of
                        // 'segment' data.
                        // This way we can resolve the range checking problem and give
                        // different `devid` to all the devices. Limitation is that at
                        // most 256 segments can be supported.
                        //
                        let modified_devid = ((cfg.devid & 0x00ff_0000) >> 8) | cfg.devid & 0xff;

                        kvm_route.flags = KVM_MSI_VALID_DEVID;
                        kvm_route.u.msi.__bindgen_anon_1.devid = modified_devid;
                    }
                    kvm_route
                }
                InterruptSourceConfig::LegacyIrq(cfg) => {
                    let mut kvm_route = kvm_irq_routing_entry {
                        gsi: route.gsi,
                        type_: KVM_IRQ_ROUTING_IRQCHIP,
                        ..Default::default()
                    };
                    kvm_route.u.irqchip.irqchip = cfg.irqchip;
                    kvm_route.u.irqchip.pin = cfg.pin;

                    kvm_route
                }
            };

            let entry = RoutingEntry {
                route: kvm_route,
                masked,
            };

            if masked {
                route.disable(&self.vm)?;
            }

            let mut routes = self.gsi_msi_routes.lock().unwrap();
            routes.insert(route.gsi, entry);
            if set_gsi {
                self.set_gsi_routes(&routes)?;
            }

            if !masked {
                route.enable(&self.vm)?;
            }

            return Ok(());
        }

        Err(std::io::Error::other(format!(
            "update: Invalid interrupt index {index}"
        )))
    }

    fn set_gsi(&self) -> vm_device::interrupt::Result<()> {
        let routes = self.gsi_msi_routes.lock().expect("Poisoned lock");
        self.set_gsi_routes(&routes)
    }
}

pub struct MsiInterruptManager {
    allocator: Arc<ResourceAllocator>,
    vm_fd: Arc<VmFd>,
    gsi_msi_routes: Arc<Mutex<HashMap<u32, RoutingEntry>>>,
}

impl MsiInterruptManager {
    pub fn new(allocator: Arc<ResourceAllocator>, vm_fd: Arc<VmFd>) -> Self {
        let gsi_msi_routes = Arc::new(Mutex::new(HashMap::new()));
        MsiInterruptManager {
            allocator,
            vm_fd,
            gsi_msi_routes,
        }
    }
}

impl InterruptManager for MsiInterruptManager {
    type GroupConfig = MsiIrqGroupConfig;

    fn create_group(
        &self,
        config: Self::GroupConfig,
    ) -> vm_device::interrupt::Result<Arc<dyn InterruptSourceGroup>> {
        let mut irq_routes: HashMap<InterruptIndex, InterruptRoute> =
            HashMap::with_capacity(config.count as usize);
        for i in config.base..config.base + config.count {
            irq_routes.insert(i, InterruptRoute::new(&self.allocator).unwrap());
        }

        Ok(Arc::new(MsiInterruptGroup::new(
            self.vm_fd.clone(),
            self.gsi_msi_routes.clone(),
            irq_routes,
        )))
    }

    fn destroy_group(
        &self,
        _group: Arc<dyn InterruptSourceGroup>,
    ) -> vm_device::interrupt::Result<()> {
        Ok(())
    }
}
