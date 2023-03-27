# Copyright 2023 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Tests for the virtio-rng device"""


def test_rng_not_present(test_microvm_with_rng, network_config):
    """
    Test a guest microVM *without* an entropy device and ensure that
    we cannot get data from /dev/hwrng
    """

    vm = test_microvm_with_rng
    vm.spawn()
    vm.basic_config()
    _ = vm.ssh_network_config(network_config, "1")
    vm.start()

    cmd = "test -e /dev/hwrng"
    ecode, _, _ = vm.ssh.execute_command(cmd)
    assert ecode == 0

    cmd = "dd if=/dev/hwrng of=/dev/null bs=10 count=1"
    ecode, _, _ = vm.ssh.execute_command(cmd)
    assert ecode == 1


def test_rng_present(test_microvm_with_rng, network_config):
    """
    Test a guest microVM with an entropy defined configured and ensure
    that we can access `/dev/hwrng`
    """

    vm = test_microvm_with_rng
    vm.spawn()
    vm.basic_config()
    vm.entropy.put()
    _ = vm.ssh_network_config(network_config, "1")
    vm.start()

    cmd = "test -e /dev/hwrng"
    ecode, _, _ = vm.ssh.execute_command(cmd)
    assert ecode == 0

    cmd = "dd if=/dev/hwrng of=/dev/null bs=10 count=1"
    assert ecode == 0
