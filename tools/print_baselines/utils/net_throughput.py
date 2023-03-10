import matplotlib.pyplot as plt
from collections import defaultdict
import numpy as np

from utils.parser import BaselineParser
from utils.types import KernelVersion, Metric, MachineConfig, GuestOs
from utils.git_worktree import GitWorktree
from utils.plotter import MetricPlotter

TESTS = [
    "tcp-p1024K-ws16k",
    "tcp-p1024K-ws256k",
    "tcp-p1024K-wsDEFAULT",
    "tcp-pDEFAULT-ws16k",
    "tcp-pDEFAULT-ws256k",
    "tcp-pDEFAULT-wsDEFAULT",
]

class NetworkTCPThroughput(BaselineParser):
    """Class representing Network TCP throughput baselines"""

    def __init__(self, baseline_dir, host_kernel: KernelVersion, git_version):
        baselines_file = f"{baseline_dir}/test_network_tcp_throughput_config_{host_kernel}.json"
        super().__init__(baselines_file, host_kernel, git_version)
        super()._parse_data(NetworkTCPThroughput.parser)
        
    @staticmethod
    def parser(data, container):
        # Parse host-to-guest
        h2g = {
            "delta_percentage": [data[type]["delta_percentage"] for type in data.keys() if "h2g" in type],
            "target": [data[type]["target"] for type in data.keys() if "h2g" in type]
        }
        container["host-to-guest"] = h2g

        # Parse guest-to-host
        g2h = {
            "delta_percentage": [data[type]["delta_percentage"] for type in data.keys() if "g2h" in type],
            "target": [data[type]["target"] for type in data.keys() if "g2h" in type]
        }
        container["guest-to-host"] = g2h

        # Parse bidirectional
        bd = {
            "test_type": [type for type in data.keys() if "bd" in type],
            "delta_percentage": [data[type]["delta_percentage"] for type in data.keys() if "bd" in type],
            "target": [data[type]["target"] for type in data.keys() if "bd" in type]
        }
        container["bidirectional"] = bd

    @staticmethod
    def test_names(): 
        return TESTS

    def guest_to_host_target(self, instance_type, metric, guest_kernel, guest_rootfs, machine_config):
        return self.machine_configs(instance_type, metric, guest_kernel, guest_rootfs)[machine_config]["guest-to-host"]["target"]

    def guest_to_host_target_delta(self, instance_type, metric, guest_kernel, guest_rootfs, machine_config):
        return self.machine_configs(instance_type, metric, guest_kernel, guest_rootfs)[machine_config]["guest-to-host"]["delta"]

    def host_to_guest_target(self, instance_type, metric, guest_kernel, guest_rootfs, machine_config):
        return self.machine_configs(instance_type, metric, guest_kernel, guest_rootfs)[machine_config]["host-to-guest"]["target"]

    def host_to_guest_target_delta(self, instance_type, metric, guest_kernel, guest_rootfs, machine_config):
        return self.machine_configs(instance_type, metric, guest_kernel, guest_rootfs)[machine_config]["host-to-guest"]["delta"]

    def bidirectional_target(self, instance_type, metric, guest_kernel, guest_rootfs, machine_config):
        return self.machine_configs(instance_type, metric, guest_kernel, guest_rootfs)[machine_config]["bidirectional"]["target"]

    def bidirectional_target_delta(self, instance_type, metric, guest_kernel, guest_rootfs, machine_config):
        return self.machine_configs(instance_type, metric, guest_kernel, guest_rootfs)[machine_config]["bidirectional"]["delta"]


def _plot_network_tcp_throughput(data_parser, title, git_versions, machine, host_kernel, guest_kernel, outfile):
    print(f"Plotting Git history of guest-to-host throughput for {machine} with host kernel {host_kernel} and guest kernel {guest_kernel}")
    plt.style.use('ggplot')
    _, ax = plt.subplots(dpi=400, figsize=(8,8))
    ax.set_title(f"Network TCP throughput\nMachine: {machine} Host Kernel: {host_kernel} Guest Kernel: {guest_kernel}")
    ax.set_ylabel("MBps")
    ax.grid(True)

    guest_to_host = {
            "metric": defaultdict(lambda: [], {}),
            "io_util": defaultdict(lambda: [], {}),
            "total_util": defaultdict(lambda: [], {}),
    }

    width = 1 / (len(TESTS) + 1)
    found_git_versions = []
    for git_ref in git_versions:
        with GitWorktree(git_ref) as worktree:
            data_file = f"{worktree}/tests/integration_tests/performance/configs/test_network_tcp_throughput_config_{host_kernel}.json"
            parser = NetworkTCPThroughput(data_file, host_kernel, hash)

            if machine not in parser.cpu_models():
                print(f"No data for machine {machine} with git version {git_ref}")
                continue
            
            if guest_kernel not in parser.guest_kernels(machine, Metric.THROUGHPUT):
                print(f"No data for guest kernel {guest_kernel} with git version {git_ref}")
                continue

            found_git_versions.append(git_ref)

            (metric, io_util, total_util) = data_parser(parser)


            for (test, value) in zip(TESTS, metric):
                guest_to_host["metric"][test].append(value)

            for (test,value) in zip(TESTS, io_util):
                guest_to_host["io_util"][test].append(value)

            for (test,value) in zip(TESTS, total_util):
                guest_to_host["total_util"][test].append(value)

    xticks = np.arange(len(found_git_versions)) 
    
    for (metric_index, (metric, values)) in enumerate(guest_to_host['metric'].items()):
        ax.bar(xticks + metric_index * width, values, width=width, label=metric)
    
    ax.legend(loc='upper center', bbox_to_anchor=(0.5, -0.05), ncol=len(TESTS)/2)
    ax.set_xticks(xticks + 2.5 * width, found_git_versions)

    plt.savefig(outfile)

def plot_network_tcp_throughput(git_versions, machine, host_kernel, guest_kernel, outdir):
    parser = lambda parser: (
        parser.guest_to_host_target(machine, Metric.THROUGHPUT, guest_kernel, GuestOs.UBUNTU_18_04, MachineConfig.VCPU_2_MEM_1G),
        parser.guest_to_host_target(machine, Metric.CPU_IO, guest_kernel, GuestOs.UBUNTU_18_04, MachineConfig.VCPU_2_MEM_1G),
        parser.guest_to_host_target(machine, Metric.CPU_TOTAL, guest_kernel, GuestOs.UBUNTU_18_04, MachineConfig.VCPU_2_MEM_1G)
    )

    _plot_network_tcp_throughput(
            parser,
            f"guest-to-host TCP throughput\nMachine: {machine} Host kernel: {host_kernel} Guest kernel: {guest_kernel}",
            git_versions, machine, host_kernel, guest_kernel,
            f"{outdir}/network_tcp_throughput_{machine}_host-{host_kernel}_guest-{guest_kernel}_host-to-guest.png"
    )

class NetworkTCPPlotter(MetricPlotter):
    def __init__(self):
        super().__init__()

    def plot_for_git(self):
        configs = self.git_combinations()

        for (machine, host_kernel, guest_kernel) in configs:
            super()._plot_for_git(NetworkTCPThroughput, machine, host_kernel, guest_kernel)


