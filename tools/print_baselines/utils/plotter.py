import matplotlib.pyplot as plt
from collections import defaultdict
import numpy as np
import itertools

from utils.git_worktree import GitWorktree

class MetricPlotter(object):
    def __init__(self):
        self._style = "ggplot"
        self._title = None
        self._dpi = 400,
        self._figsize = (8, 8)
        self._ylabel = None
        self._grid = True
        self._machines = []
        self._git_versions = []
        self._host_kernels = []
        self._guest_kernels = []
        self._metrics = []
        self._data = {
            "metric": defaultdict(lambda: [], {}),
            "io_util": defaultdict(lambda: [], {}),
            "total_util": defaultdict(lambda: [], {}),
        }
        self._outfile = "firecracker_perf_data.png"

    @property
    def plot_style(self):
        return self._style

    @property
    def title(self):
        return self._title

    @property
    def dpi(self):
        return self._dpi

    @property
    def figure_size(self):
        return self._figsize

    @property
    def ylabel(self):
        return self._ylabel

    @property
    def grid(self):
        return self._grid

    @property
    def git_versions(self):
        return self._git_versions

    @property
    def host_kernels(self):
        return self._host_kernels

    @property
    def guest_kernels(self):
        return self._guest_kernels

    @property
    def machines(self):
        return self._machines

    @property
    def outfile(self):
        return self._outfile

    @property
    def metrics(self):
        return self._metrics

    def git_combinations(self):
        return itertools.product([self.machines, self.host_kernels, self.guest_kernels])

    def _plot_for_git(self, parser_type, machine, host_kernel, guest_kernel):
        plt.style.use(self.plot_style)

        
        fig = plt.figure(dpi=self.dpi, figsize=self.figure_size)
        ax = fig.add_axes([0, 0, 1, 1])
        ax.set_title(self.title)
        ax.set_ylabel(self.ylabel)
        ax.grid(self.grid)

        if not self.metrics:
            raise Exception("Trying to plot without setting metrics")

        bar_width = 1 / (len(self._metrics) + 1)
        found_git_versions = []

        for git_ref in self.git_versions:
            with GitWorktree(git_ref) as worktree:
                parser = parser_type(f"{worktree}/tests/integration_tests/performance/configs/", host_kernel, git_ref) 


