import json
from typing import List

from utils.types import KernelVersion, Metric, CpuModel, GuestOs, MachineConfig

class BaselineParser:
    """
    Class that allows loading a performance baselines file and
    query various information about it
    """

    def __init__(self, baselines, host_kernel: KernelVersion, git_version):
        with open(baselines, "r", encoding="utf-8") as fp:
            raw = json.load(fp);
            
            if "hosts" not in raw.keys():
                raise Exception("Malformed baseline file")

            self._meta = {x: raw[x] for x in raw.keys() if x != "hosts"}
            self._baselines = raw['hosts']['instances']
            self._instances = {}
            self._host_kernel = host_kernel
            self._git_version = git_version


    def metadata(self):
        return self._meta

    def cpu_models(self):
        return self._instances

    def host_kernel(self):
        return self._host_kernel

    def git_version(self):
        return self._git_version

    def metrics(self, instance_type):
        return self._instances[instance_type]

    def guest_kernels(self, instance_type, metric):
        return self.metrics(instance_type)[metric]

    def guest_rootfs(self, instance_type, metric, guest_kernel):
        return self.guest_kernels(instance_type, metric)[guest_kernel]

    def machine_configs(self, instance_type, metric, guest_kernel, guest_rootfs):
        return self.guest_rootfs(instance_type, metric, guest_kernel)[guest_rootfs]
        
    def _parse_data(self, parser):
        self._instances = {}
        for (instance_type, instance_data) in self._baselines.items():
            for cpu in instance_data['cpus']:
                model = cpu['model']
                baselines = cpu['baselines']

                metrics = {}
                for metric in baselines:
                    guest_kernels = {}
                    for guest_kernel in baselines[metric]:
                        os = {}
                        for userspace in baselines[metric][guest_kernel]:
                            machine_config = {}
                            for config in baselines[metric][guest_kernel][userspace]:
                                metric_type = list(baselines[metric][guest_kernel][userspace][config].keys())
                                assert len(metric_type) == 1
                                metric_type = metric_type[0]
                                data = baselines[metric][guest_kernel][userspace][config][metric_type]

                                config = MachineConfig.from_str(config)
                                machine_config[config] = {}
                                parser(data, machine_config[config])


                            os[GuestOs.from_str(userspace)] = machine_config

                        guest_kernels[KernelVersion.from_str(guest_kernel)] = os

                    metrics[Metric.from_str(metric)] = guest_kernels

                self._instances[CpuModel.from_str(instance_type, model)] = metrics
