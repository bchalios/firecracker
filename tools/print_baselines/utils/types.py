from enum import Enum

class CpuModel(Enum):
    SKY_LAKE = 1,
    CASCADE_LAKE = 2,
    ICE_LAKE = 3,
    MILAN = 4,
    GRAVITON2 = 5,
    GRAVITON3 = 6,
    GRAVITON4 = 7,

    def __str__(self):
        if self == CpuModel.SKY_LAKE:
            return "SkyLake"
        elif self == CpuModel.CASCADE_LAKE:
            return "CascadeLake"
        elif self == CpuModel.ICE_LAKE:
            return "IceLake"
        elif self == CpuModel.MILAN:
            return "Milan"
        elif self == CpuModel.GRAVITON2:
            return "Graviton2"
        elif self == CpuModel.GRAVITON3:
            return "Graviton3"
        elif self == CpuModel.GRAVITON4:
            return "Graviton4"

    @staticmethod
    def from_str(instance_type, model):
        MODELCODE = {
            ("m5d.metal", "Intel(R) Xeon(R) Platinum 8175M CPU @ 2.50GHz"): CpuModel.SKY_LAKE,
            ("m5d.metal", "Intel(R) Xeon(R) Platinum 8259CL CPU @ 2.50GHz"): CpuModel.CASCADE_LAKE,
            ("m6i.metal", "Intel(R) Xeon(R) Platinum 8375C CPU @ 2.90GHz"): CpuModel.ICE_LAKE,
            ("m6a.metal", "AMD EPYC 7R13 48-Core Processor"): CpuModel.MILAN,
            ("m6g.metal", "ARM_NEOVERSE_N1"): CpuModel.GRAVITON2,
            ("m6g.metal", "ARM_NEOVERSE_V1"): CpuModel.GRAVITON3,
            ("c7g.metal", "ARM_NEOVERSE_V1"): CpuModel.GRAVITON4,
        }

        return MODELCODE[(instance_type, model)]

    @staticmethod
    def from_codename(name):
        name = name.lower()
        if name == "skylake":
            return CpuModel.SKY_LAKE
        elif name == "cascadelake":
            return CpuModel.CASCADE_LAKE
        elif name == "icelake":
            return CpuModel.ICE_LAKE
        elif name == "milan":
            return CpuModel.MILAN
        elif name == "graviton2":
            return CpuModel.GRAVITON2
        elif name == "graviton3":
            return CpuModel.GRAVITON3
        elif name == "graviton4":
            return CpuModel.GRAVITON4
        else:
            raise Exception(f"Unknown CPU model: {name}")


class GuestOs(Enum):
    UBUNTU_18_04 = 1

    def __str__(self):
        if self == GuestOs.UBUNTU_18_04:
            return "Ubuntu 18.04"

    @staticmethod
    def from_str(raw_str):
        if raw_str == "ubuntu-18.04.ext4":
            return GuestOs.UBUNTU_18_04
        else:
            raise Exception(f"Unknown guest os: {raw_str}")

class MachineConfig(Enum):
    VCPU_1_MEM_1G = 1,
    VCPU_2_MEM_1G = 2,

    def __str__(self):
        if self == MachineConfig.VCPU_1_MEM_1G:
            return "1vcpu_1024mb"
        elif self == MachineConfig.VCPU_2_MEM_1G:
            return "2vcpu_1024mb"

    @staticmethod
    def from_str(raw_str):
        if raw_str == "1vcpu_1024mb.json":
            return MachineConfig.VCPU_1_MEM_1G
        elif raw_str == "2vcpu_1024mb.json":
            return MachineConfig.VCPU_2_MEM_1G
        else:
            raise Exception(f"Unknown machine configuration: {raw_str}")

class Metric(Enum):
    CPU_TOTAL = 1,
    CPU_IO = 2,
    THROUGHPUT = 3,

    def __str__(self):
        if self == Metric.CPU_TOTAL:
            return "cpu_utilization_vcpus_total"
        elif self == Metric.CPU_IO:
            return "cpu_utilization_vmm"
        elif self == Metric.THROUGHPUT:
            return "throughput"

    @staticmethod
    def from_str(raw_str):
        if raw_str == "cpu_utilization_vcpus_total":
            return Metric.CPU_TOTAL
        elif raw_str == "cpu_utilization_vmm":
            return Metric.CPU_IO
        elif raw_str == "throughput":
            return Metric.THROUGHPUT
        else:
            raise Exception(f"Unknown metric {raw_str}")


class KernelVersion(Enum):
    KERNEL_5_10 = 1,
    KERNEL_4_14 = 2,

    def __str__(self):
        if self == KernelVersion.KERNEL_5_10:
            return "5.10"
        elif self == KernelVersion.KERNEL_4_14:
            return "4.14"

    @staticmethod
    def from_str(raw_str):
        if raw_str in ["5.10", "vmlinux-5.10.bin"]:
            return KernelVersion.KERNEL_5_10
        elif raw_str in ["4.14", "vmlinux-4.14.bin"]:
            return KernelVersion.KERNEL_4_14
        else:
            raise Exception(f"Unknown kernel version: {raw_str}")


