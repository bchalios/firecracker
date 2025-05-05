// Copyright 2019 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub(crate) mod cache_info;
mod fdt;
/// Module for the global interrupt controller configuration.
pub mod gic;
/// Architecture specific KVM-related code
pub mod kvm;
/// Layout for this aarch64 system.
pub mod layout;
/// Logic for configuring aarch64 registers.
pub mod regs;
/// Architecture specific vCPU code
pub mod vcpu;
/// Architecture specific VM state code
pub mod vm;

use std::cmp::min;
use std::fmt::Debug;
use std::fs::File;

use linux_loader::loader::pe::PE as Loader;
use linux_loader::loader::{Cmdline, KernelLoader};
use vm_memory::GuestMemoryError;

use crate::arch::{BootProtocol, EntryPoint, arch_memory_regions_with_gap};
use crate::cpu_config::aarch64::{CpuConfiguration, CpuConfigurationError};
use crate::cpu_config::templates::CustomCpuTemplate;
use crate::initrd::InitrdConfig;
use crate::utils::{align_up, u64_to_usize, usize_to_u64};
use crate::vmm_config::machine_config::MachineConfig;
use crate::vstate::memory::{Address, Bytes, GuestAddress, GuestMemory, GuestMemoryMmap};
use crate::vstate::vcpu::KvmVcpuError;
use crate::{Vcpu, VcpuConfig, Vmm, logger};

/// Errors thrown while configuring aarch64 system.
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum ConfigurationError {
    /// Failed to create a Flattened Device Tree for this aarch64 microVM: {0}
    SetupFDT(#[from] fdt::FdtError),
    /// Failed to write to guest memory.
    MemoryError(#[from] GuestMemoryError),
    /// Cannot copy kernel file fd
    KernelFile,
    /// Cannot load kernel due to invalid memory configuration or invalid kernel image: {0}
    KernelLoader(#[from] linux_loader::loader::Error),
    /// Error creating vcpu configuration: {0}
    VcpuConfig(#[from] CpuConfigurationError),
    /// Error configuring the vcpu: {0}
    VcpuConfigure(#[from] KvmVcpuError),
}

/// Returns a Vec of the valid memory addresses for aarch64.
/// See [`layout`](layout) module for a drawing of the specific memory model for this platform.
///
/// The `offset` parameter specified the offset from [`layout::DRAM_MEM_START`].
pub fn arch_memory_regions(offset: usize, size: usize) -> Vec<(GuestAddress, usize)> {
    assert!(size > 0, "Attempt to allocate guest memory of length 0");
    assert!(
        offset.checked_add(size).is_some(),
        "Attempt to allocate guest memory such that the address space would wrap around"
    );
    assert!(
        offset < layout::DRAM_MEM_MAX_SIZE,
        "offset outside allowed DRAM range"
    );

    let dram_size = min(
        size,
        layout::DRAM_MEM_MAX_SIZE - offset - u64_to_usize(layout::MMIO64_MEM_SIZE),
    );

    if dram_size != size {
        logger::warn!(
            "Requested offset/memory size {}/{} exceeds architectural maximum (1022GiB). Size has \
             been truncated to {}",
            offset,
            size,
            dram_size
        );
    }

    let mut regions = vec![];
    if let Some((offset, remaining)) = arch_memory_regions_with_gap(
        &mut regions,
        usize::try_from(layout::DRAM_MEM_START).unwrap() + offset,
        size,
        u64_to_usize(layout::MMIO64_MEM_START),
        u64_to_usize(layout::MMIO64_MEM_SIZE),
    ) {
        regions.push((GuestAddress(offset as u64), remaining));
    }

    regions
}

/// Configures the system for booting Linux.
pub fn configure_system_for_boot(
    vmm: &mut Vmm,
    vcpus: &mut [Vcpu],
    machine_config: &MachineConfig,
    cpu_template: &CustomCpuTemplate,
    entry_point: EntryPoint,
    initrd: &Option<InitrdConfig>,
    boot_cmdline: Cmdline,
) -> Result<(), ConfigurationError> {
    // Construct the base CpuConfiguration to apply CPU template onto.
    let cpu_config = CpuConfiguration::new(cpu_template, vcpus)?;

    // Apply CPU template to the base CpuConfiguration.
    let cpu_config = CpuConfiguration::apply_template(cpu_config, cpu_template);

    let vcpu_config = VcpuConfig {
        vcpu_count: machine_config.vcpu_count,
        smt: machine_config.smt,
        cpu_config,
    };

    let optional_capabilities = vmm.kvm.optional_capabilities();
    // Configure vCPUs with normalizing and setting the generated CPU configuration.
    for vcpu in vcpus.iter_mut() {
        vcpu.kvm_vcpu.configure(
            vmm.vm.guest_memory(),
            entry_point,
            &vcpu_config,
            &optional_capabilities,
        )?;
    }
    let vcpu_mpidr = vcpus
        .iter_mut()
        .map(|cpu| cpu.kvm_vcpu.get_mpidr())
        .collect::<Result<Vec<_>, _>>()
        .map_err(KvmVcpuError::ConfigureRegisters)?;
    let cmdline = boot_cmdline
        .as_cstring()
        .expect("Cannot create cstring from cmdline string");

    let fdt = fdt::create_fdt(
        vmm.vm.guest_memory(),
        vcpu_mpidr,
        cmdline,
        &vmm.device_manager,
        vmm.vm.get_irqchip(),
        initrd,
    )?;

    let fdt_address = GuestAddress(get_fdt_addr(vmm.vm.guest_memory()));
    vmm.vm
        .guest_memory()
        .write_slice(fdt.as_slice(), fdt_address)?;

    Ok(())
}

/// Returns the memory address where the kernel could be loaded.
pub fn get_kernel_start() -> u64 {
    layout::SYSTEM_MEM_START + layout::SYSTEM_MEM_SIZE
}

/// Returns the memory address where the initrd could be loaded.
pub fn initrd_load_addr(guest_mem: &GuestMemoryMmap, initrd_size: usize) -> Option<u64> {
    let rounded_size = align_up(
        usize_to_u64(initrd_size),
        usize_to_u64(super::GUEST_PAGE_SIZE),
    );
    match GuestAddress(get_fdt_addr(guest_mem)).checked_sub(rounded_size) {
        Some(offset) => {
            if guest_mem.address_in_range(offset) {
                Some(offset.raw_value())
            } else {
                None
            }
        }
        None => None,
    }
}

// Auxiliary function to get the address where the device tree blob is loaded.
fn get_fdt_addr(mem: &GuestMemoryMmap) -> u64 {
    // If the memory allocated is smaller than the size allocated for the FDT,
    // we return the start of the DRAM so that
    // we allow the code to try and load the FDT.

    if let Some(addr) = mem.last_addr().checked_sub(layout::FDT_MAX_SIZE as u64 - 1) {
        if mem.address_in_range(addr) {
            return addr.raw_value();
        }
    }

    layout::DRAM_MEM_START
}

/// Load linux kernel into guest memory.
pub fn load_kernel(
    kernel: &File,
    guest_memory: &GuestMemoryMmap,
) -> Result<EntryPoint, ConfigurationError> {
    // Need to clone the File because reading from it
    // mutates it.
    let mut kernel_file = kernel
        .try_clone()
        .map_err(|_| ConfigurationError::KernelFile)?;

    let entry_addr = Loader::load(
        guest_memory,
        Some(GuestAddress(get_kernel_start())),
        &mut kernel_file,
        None,
    )?;

    Ok(EntryPoint {
        entry_addr: entry_addr.kernel_load,
        protocol: BootProtocol::LinuxBoot,
    })
}

#[cfg(kani)]
mod verification {
    use vm_memory::GuestAddress;

    use crate::arch::aarch64::layout;
    use crate::arch::arch_memory_regions;

    #[kani::proof]
    #[kani::unwind(3)]
    fn verify_arch_memory_regions() {
        let offset: u64 = kani::any::<u64>();
        let len: u64 = kani::any::<u64>();

        kani::assume(len > 0);
        kani::assume(offset.checked_add(len).is_some());
        kani::assume(offset < layout::DRAM_MEM_MAX_SIZE as u64);

        let regions = arch_memory_regions(offset as usize, len as usize);

        // On Arm we have one MMIO gap that might fall within addressable ranges,
        // so we can get either 1 or 2 regions.
        assert!(regions.len() >= 1);
        assert!(regions.len() <= 2);

        // The very first address should be offset bytes past DRAM_MEM_START
        assert_eq!(start, layout::DRAM_MEM_START + offset);
        // The total length of all regions cannot exceed DRAM_MEM_MAX_SIZE
        let actual_len = regions.iter().map(|&(_, len)| len).sum::<usize>();
        assert!(actual_len <= layout::DRAM_MEM_MAX_SIZE as u64);
        // The total length is smaller or equal to the length we asked
        assert!(actual_len <= len);
        // If it's smaller, it's because we asked more than the the maximum possible.
        if actual_len < len {
            assert!(offset + len >= layout::DRAM_MEM_MAX_SIZE as u64);
        }

        // No region overlaps the 64-bit MMIO gap
        assert!(
            regions
                .iter()
                .all(|&(start, len)| start.0 >= FIRST_ADDR_PAST_64BITS_MMIO
                    || start.0 + len as u64 <= MMIO64_MEM_START)
        );

        // All regions start after our specified offset
        assert!(regions.iter().all(|&(start, _)| start.0 >= offset as u64));

        // All regions have non-zero length
        assert!(regions.iter().all(|&(_, len)| len > 0));

        // If there's two regions, they perfectly snuggle up the 64bit MMIO gap
        if regions.len() == 2 {
            kani::cover!();

            assert_eq!(regions[0].0.0 + regions[0].1 as u64, MMIO32_MEM_START);
            assert_eq!(regions[1].0.0, FIRST_ADDR_PAST_32BITS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::arch_mem;

    #[test]
    fn test_regions_lt_1024gb() {
        let regions = arch_memory_regions(0, 1usize << 29);
        assert_eq!(1, regions.len());
        assert_eq!(GuestAddress(super::layout::DRAM_MEM_START), regions[0].0);
        assert_eq!(1usize << 29, regions[0].1);
    }

    #[test]
    fn test_regions_gt_1024gb() {
        let regions = arch_memory_regions(0, 1usize << 41);
        assert_eq!(1, regions.len());
        assert_eq!(GuestAddress(super::layout::DRAM_MEM_START), regions[0].0);
        assert_eq!(super::layout::DRAM_MEM_MAX_SIZE, regions[0].1);
    }

    #[test]
    fn test_get_fdt_addr() {
        let mem = arch_mem(layout::FDT_MAX_SIZE - 0x1000);
        assert_eq!(get_fdt_addr(&mem), layout::DRAM_MEM_START);

        let mem = arch_mem(layout::FDT_MAX_SIZE);
        assert_eq!(get_fdt_addr(&mem), layout::DRAM_MEM_START);

        let mem = arch_mem(layout::FDT_MAX_SIZE + 0x1000);
        assert_eq!(get_fdt_addr(&mem), 0x1000 + layout::DRAM_MEM_START);
    }
}
