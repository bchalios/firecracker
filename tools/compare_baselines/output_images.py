# Copyright 2022 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Output images of CPU performance comparison results"""


import os
import json
import argparse
import numpy as np
import matplotlib.pyplot as plt

from utils.defs import MODEL2SHORT, DEFAULT_RESULT_FILEPATH


def output_images(result, dpath, key, base):
    """Output images of comparison results"""
    for raw in result.values():
        stats = raw["stats"]

        for metric, value in stats.items():
            cpus = value[key]

            labels = []
            x_vals = []
            y_vals = []
            yerrs = []
            for i, cpu in enumerate(cpus):
                labels.append(f"{MODEL2SHORT[cpu['model']]}")
                x_vals.append(i)
                y_vals.append(cpu["value"]["mean"] + base)
                yerrs.append(cpu["value"]["stdev"])

            plt.rcParams["font.size"] = 16
            plt.title(f"{metric} - {key}", fontsize=20)
            plt.ylabel(f"% on basis of {MODEL2SHORT[cpus[0]['model']]}")
            plt.bar(
                np.array(x_vals),
                np.array(y_vals),
                tick_label=labels,
                yerr=yerrs,
                capsize=10,
                align="center",
            )
            plt.tight_layout()
            plt.savefig(
                os.path.join(
                    dpath,
                    f"{raw['test']}_{raw['kernel']}_{metric}_{key}.png",
                )
            )
            plt.clf()


def main():
    """Main function"""
    parser = argparse.ArgumentParser(
        description="Output images for CPU performance comparision results."
    )
    parser.add_argument(
        "-i",
        "--input",
        help="Path of performance comparison results file.",
        action="store",
        default=DEFAULT_RESULT_FILEPATH,
    )
    parser.add_argument(
        "-o",
        "--output",
        help="Path of output directory.",
        action="store",
        default="comparison_results/",
    )
    args = parser.parse_args()

    os.makedirs(args.output, exist_ok=True)

    with open(args.input, "r", encoding="utf-8") as fp:
        result = json.load(fp)

    for (key, base) in [
        ("target_diff_percentage", 100),
        ("delta_percentage_diff", 0),
    ]:
        output_images(result, args.output, key, base)


if __name__ == "__main__":
    main()
