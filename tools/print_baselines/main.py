import os
import argparse

from utils.types import CpuModel, KernelVersion, Metric, GuestOs, MachineConfig
from utils.net_throughput import plot_network_tcp_throughput
from utils.defs import TESTS, KERNELS, CODENAME2DICT 

def plot_guest_kernel_metrics(args, machine, host_kernel, guest_kernel):
    for test in args.tests:
        if test == "network_tcp_throughput":
            plot_network_tcp_throughput(args.git_versions, machine, host_kernel, guest_kernel, args.out_dir)

def plot_host_kernel_metrics(args, machine, host_kernel):
    for guest_kernel in args.guest_kernels:
        guest_kernel = KernelVersion.from_str(guest_kernel)
        plot_guest_kernel_metrics(args, machine, host_kernel, guest_kernel)

def plot_machine_metrics(args, machine):
    for host_kernel in args.host_kernels:
        host_kernel = KernelVersion.from_str(host_kernel)
        plot_host_kernel_metrics(args, machine, host_kernel)

def cmd_git(args):
    """Plot performance baselines between various commit hashes"""
    for machine in args.cpu_models:
        machine = CpuModel.from_codename(machine)
        plot_machine_metrics(args, machine)


def main():
    parser = argparse.ArgumentParser(description="Plot Firecracker perofrmance metrics")

    shared_parser = argparse.ArgumentParser(add_help=False)
    shared_parser.add_argument(
        "-t",
        "--tests",
        help="List of test types",
        nargs="+",
        action="store",
        choices=TESTS,
        default=TESTS,
    ) 
    shared_parser.add_argument(
        "--host-kernels",
        help="List of host kernel versions",
        nargs="+",
        action="store",
        choices=KERNELS,
        default=KERNELS,
    ) 
    shared_parser.add_argument(
        "--guest-kernels",
        help="List of guest kernel versions",
        nargs="+",
        action="store",
        choices=KERNELS,
        default=KERNELS,
    ) 
    shared_parser.add_argument(
        "--cpu-models",
        help="List of CPU codenames",
        action="store",
        nargs="+",
        choices=list(CODENAME2DICT.keys()),
        default=list(CODENAME2DICT.keys()),
    )
    shared_parser.add_argument(
        "-o",
        "--out-dir",
        help="Directory to store plots",
        action="store",
        default=os.getcwd(),
    )

    subparsers = parser.add_subparsers(title="modes")

    # Subcommand for plotting multiple git versions
    git_versions = subparsers.add_parser(
        "commit", parents=[shared_parser], help="Plot for various git hashes"
    )
    git_versions.set_defaults(handler=cmd_git)
    git_versions.add_argument(
        "-g",
        "--git-versions",
        help="List of git commits hashes, branches, or tags to plot",
        action="store",
        nargs="+",
        required=True,
    )


    # Parse arguments and call corresponding handler
    args = parser.parse_args()
    if hasattr(args, "handler"):
        args.handler(args)
    else:
        parser.print_help()


    """
    shared_parser = NetworkTCPThroughput(sys.argv[1], KernelVersion.KERNEL_5_10, "main")

    test_names = parser.guest_to_host_test_type(CpuModel.CASCADE_LAKE, Metric.THROUGHPUT, KernelVersion.KERNEL_5_10, GuestOs.UBUNTU_18_04, MachineConfig.VCPU_1_MEM_1G)
    target_5_10 = parser.guest_to_host_target(CpuModel.CASCADE_LAKE, Metric.THROUGHPUT, KernelVersion.KERNEL_5_10, GuestOs.UBUNTU_18_04, MachineConfig.VCPU_1_MEM_1G)
    target_4_14 = parser.guest_to_host_target(CpuModel.CASCADE_LAKE, Metric.THROUGHPUT, KernelVersion.KERNEL_4_14, GuestOs.UBUNTU_18_04, MachineConfig.VCPU_1_MEM_1G)
    """

if __name__ == "__main__":

    main()
