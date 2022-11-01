# Copyright 2022 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Compare gathered baselines"""

import os
import argparse
import subprocess

from utils.comparator import DirectoryComparator, CpuComparator
from utils.defs import DEFAULT_BASELINE_DIRECTORY, CODENAME2DICT, TESTS, KERNELS, DEFAULT_RESULT_FILEPATH


def cmd_cpu(args):
    """Compare baselines between CPUs"""
    comp = CpuComparator(
        args.directory,
        args.tests,
        args.kernels,
        args.codenames,
    )
    comp.compare()
    comp.dump_json(args.output)


def cmd_directory(args):
    """Comparre baselines between two directories"""
    comp = DirectoryComparator(
        args.source,
        args.target,
        args.tests,
        args.kernels,
        args.codenames,
    )
    comp.compare(args.auxiliary)
    comp.dump_json(args.output)


def cmd_commit(args):
    """Compare baselines between two commit hashes"""
    if args.target is None:
        args.target = subprocess.check_output(
            ["git", "show", "--format='%H'", "--no-patch"]
        )[:-1].decode().strip("'")

    subprocess.run(["git", "worktree", "add", args.source, args.source], check=True)
    subprocess.run(["git", "worktree", "add", args.target, args.target], check=True)

    comp = DirectoryComparator(
        os.path.join(args.source, DEFAULT_BASELINE_DIRECTORY),
        os.path.join(args.target, DEFAULT_BASELINE_DIRECTORY),
        args.tests,
        args.kernels,
        args.codenames,
    )

    subprocess.run(["git", "worktree", "remove", args.source], check=True)
    subprocess.run(["git", "worktree", "remove", args.target], check=True)

    comp.compare(args.auxiliary)
    comp.dump_json(args.output)


def cmd_latest(args):
    """Compare baselines with the latest commit"""
    latest_hash = subprocess.check_output(
        ["git", "show", "--format='%H'", "--no-patch"]
    )[:-1].decode().strip("'")

    subprocess.run(["git", "worktree", "add", latest_hash, latest_hash], check=True)

    comp = DirectoryComparator(
        os.path.join(latest_hash, DEFAULT_BASELINE_DIRECTORY),
        os.path.join(DEFAULT_BASELINE_DIRECTORY),
        args.tests,
        args.kernels,
        args.codenames,
    )

    subprocess.run(["git", "worktree", "remove", latest_hash], check=True)

    comp.compare(args.auxiliary)
    comp.dump_json(args.output)


def main():
    """Main function"""
    parser = argparse.ArgumentParser(
        description="Compare gathered performance baselines."
    )
    subparsers = parser.add_subparsers()

    """
    Subcommand for comparing baselines between CPUs
    """
    parser_cpu = subparsers.add_parser("cpu")
    parser_cpu.set_defaults(handler=cmd_cpu)
    parser_cpu.add_argument(
        "-d",
        "--directory",
        help="Path of directory containing JSON files of baselines.",
        action="store",
        default=DEFAULT_BASELINE_DIRECTORY,
    )
    parser_cpu.add_argument(
        "--tests",
        help="List of test types.",
        nargs="+",
        action="store",
        choices=TESTS,
        default=TESTS,
    )
    parser_cpu.add_argument(
        "--kernels",
        help="List of host kernel versions.",
        nargs="+",
        action="store",
        choices=KERNELS,
        default=KERNELS,
    )
    parser_cpu.add_argument(
        "--codenames",
        help="List of CPU codenames. The first one is used as basis.",
        action="store",
        nargs="*",
        choices=list(CODENAME2DICT.keys()),
        default=list(CODENAME2DICT.keys()),
    )
    parser_cpu.add_argument(
        "-o",
        "--output",
        help="Path of output file.",
        action="store",
        default=DEFAULT_RESULT_FILEPATH,
    )
    parser_cpu.add_argument(
        "-a",
        "--auxiliary",
        help="Include auxiliary information.",
        action="store_true",
    )

    """
    Subcommand for comparing baselines between directories
    """
    parser_dir = subparsers.add_parser("directory")
    parser_dir.set_defaults(handler=cmd_directory)
    parser_dir.add_argument(
        "-s",
        "--source",
        help="Path of source directory containing JSON files of baselines.",
        action="store",
        required=True,
    )
    parser_dir.add_argument(
        "-t",
        "--target",
        help="Path of target directory containing JSON files of baselines.",
        action="store",
        required=True,
    )
    parser_dir.add_argument(
        "--tests",
        help="List of test types.",
        nargs="+",
        action="store",
        choices=TESTS,
        default=TESTS,
    )
    parser_dir.add_argument(
        "--kernels",
        help="List of host kernel versions.",
        nargs="+",
        action="store",
        choices=KERNELS,
        default=KERNELS,
    )
    parser_dir.add_argument(
        "--codenames",
        help="List of CPU codenames.",
        action="store",
        nargs="*",
        choices=list(CODENAME2DICT.keys()),
        default=list(CODENAME2DICT.keys()),
    )
    parser_dir.add_argument(
        "-o",
        "--output",
        help="Path of output file.",
        action="store",
        default=DEFAULT_RESULT_FILEPATH,
    )
    parser_dir.add_argument(
        "-a",
        "--auxiliary",
        help="Include auxiliary information.",
        action="store_true",
    )

    """
    Subcommand for comparing baselines between commit hashes
    """
    parser_commit = subparsers.add_parser("commit")
    parser_commit.set_defaults(handler=cmd_commit)
    parser_commit.add_argument(
        "-d",
        "--directory",
        help="Path of directory containing JSON files of baselines.",
        action="store",
        default=DEFAULT_BASELINE_DIRECTORY,
    )
    parser_commit.add_argument(
        "-s",
        "--source",
        help="Source commit hash.",
        action="store",
        required=True,
    )
    parser_commit.add_argument(
        "-t",
        "--target",
        help="Target commit hash.",
        action="store",
    )
    parser_commit.add_argument(
        "--tests",
        help="List of test types.",
        nargs="+",
        action="store",
        choices=TESTS,
        default=TESTS,
    )
    parser_commit.add_argument(
        "--kernels",
        help="List of host kernel versions.",
        nargs="+",
        action="store",
        choices=KERNELS,
        default=KERNELS,
    )
    parser_commit.add_argument(
        "--codenames",
        help="List of CPU codenames.",
        action="store",
        nargs="*",
        choices=list(CODENAME2DICT.keys()),
        default=list(CODENAME2DICT.keys()),
    )
    parser_commit.add_argument(
        "-o",
        "--output",
        help="Path of output file.",
        action="store",
        default=DEFAULT_RESULT_FILEPATH,
    )
    parser_commit.add_argument(
        "-a",
        "--auxiliary",
        help="Include auxiliary information.",
        action="store_true",
    )

    """
    Subcommand for comparing baselines with the latest commit
    """
    parser_latest = subparsers.add_parser("latest")
    parser_latest.set_defaults(handler=cmd_latest)
    parser_latest.add_argument(
        "-d",
        "--directory",
        help="Path of directory containing JSON files of baselines.",
        action="store",
        default=DEFAULT_BASELINE_DIRECTORY,
    )
    parser_latest.add_argument(
        "--tests",
        help="List of test types.",
        nargs="+",
        action="store",
        choices=TESTS,
        default=TESTS,
    )
    parser_latest.add_argument(
        "--kernels",
        help="List of host kernel versions.",
        nargs="+",
        action="store",
        choices=KERNELS,
        default=KERNELS,
    )
    parser_latest.add_argument(
        "--codenames",
        help="List of CPU codenames.",
        action="store",
        nargs="*",
        choices=CODENAME2DICT.keys(),
        default=CODENAME2DICT.keys(),
    )
    parser_latest.add_argument(
        "-o",
        "--output",
        help="Path of output file.",
        action="store",
        default=DEFAULT_RESULT_FILEPATH,
    )
    parser_latest.add_argument(
        "-a",
        "--auxiliary",
        help="Include auxiliary information.",
        action="store_true",
    )

    # Parse arguments
    args = parser.parse_args()
    if hasattr(args, "handler"):
        args.handler(args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
