#!/usr/bin/env python3
"""
Generate benchmark plots from CSV files with schema:
iteration,rtt_ms,status,proc_ram_mb,proc_vsz_mb,proc_cpu_pct,sys_ram_pct,sys_cpu_pct

Features:
- Per-file dashboard (3 panels): RTT distribution, process CPU usage, process RAM usage
- Optional comparison dashboard combining two files
- Stats overlays: mean, median, P95, min/max, N

Usage examples:
python3 scripts/plot_benchmark_metrics.py --input scenario_a.csv --label scenario_a
python3 scripts/plot_benchmark_metrics.py --input scenario_a.csv --compare scenario_b.csv --label same_host --compare-label multi_host
"""

from __future__ import annotations

import argparse
import math
import os
from dataclasses import dataclass
from typing import Dict, Tuple

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd


REQUIRED_COLUMNS = [
    "iteration",
    "rtt_ms",
    "status",
    "proc_ram_mb",
    "proc_vsz_mb",
    "proc_cpu_pct",
    "sys_ram_pct",
    "sys_cpu_pct",
]


@dataclass
class SeriesStats:
    n: int
    min_v: float
    max_v: float
    mean_v: float
    median_v: float
    p95_v: float


def compute_stats(values: pd.Series) -> SeriesStats:
    clean = pd.to_numeric(values, errors="coerce").dropna()
    if clean.empty:
        return SeriesStats(0, math.nan, math.nan, math.nan, math.nan, math.nan)
    return SeriesStats(
        n=int(clean.shape[0]),
        min_v=float(clean.min()),
        max_v=float(clean.max()),
        mean_v=float(clean.mean()),
        median_v=float(clean.median()),
        p95_v=float(np.percentile(clean, 95)),
    )


def fmt_stats_box(name: str, st: SeriesStats, unit: str = "") -> str:
    if st.n == 0:
        return f"{name} Stats\nNo valid samples"
    return (
        f"{name} Stats\n"
        f"N = {st.n}\n"
        f"Min = {st.min_v:.3f}{unit}\n"
        f"Max = {st.max_v:.3f}{unit}\n"
        f"Mean = {st.mean_v:.3f}{unit}\n"
        f"Median = {st.median_v:.3f}{unit}\n"
        f"P95 = {st.p95_v:.3f}{unit}"
    )


def validate_columns(df: pd.DataFrame, source: str) -> None:
    missing = [c for c in REQUIRED_COLUMNS if c not in df.columns]
    if missing:
        raise ValueError(
            f"File '{source}' missing required columns: {', '.join(missing)}"
        )


def load_csv(path: str) -> pd.DataFrame:
    df = pd.read_csv(path)
    validate_columns(df, path)

    numeric_cols = [
        "iteration",
        "rtt_ms",
        "proc_ram_mb",
        "proc_vsz_mb",
        "proc_cpu_pct",
        "sys_ram_pct",
        "sys_cpu_pct",
    ]
    for col in numeric_cols:
        df[col] = pd.to_numeric(df[col], errors="coerce")

    return df


def split_ok_ko(df: pd.DataFrame) -> pd.DataFrame:
    status_clean = df["status"].astype(str).str.strip().str.lower()
    ok_df = df[status_clean == "ok"].copy()
    return ok_df


def get_total_ram_mb() -> float:
    page_size = os.sysconf("SC_PAGE_SIZE")
    pages = os.sysconf("SC_PHYS_PAGES")
    return (page_size * pages) / (1024 * 1024)


def normalize_cpu_to_system(
    proc_cpu_pct: pd.Series,
    cpu_cores: int,
    cpu_mode: str,
    sys_cpu_pct: pd.Series | None = None,
) -> Tuple[pd.Series, str]:
    clean = pd.to_numeric(proc_cpu_pct, errors="coerce")
    observed_max = float(clean.dropna().max()) if not clean.dropna().empty else 0.0

    if cpu_mode == "system":
        return clean, "system"
    if cpu_mode == "core_equivalent":
        return clean / float(cpu_cores), "core_equivalent"

    # Auto mode: values over 100 cannot be a direct system-wide percentage.
    if observed_max > 100.0:
        return clean / float(cpu_cores), "core_equivalent"

    # Auto mode heuristic using system CPU column.
    # A single process cannot consistently exceed total system CPU usage.
    if sys_cpu_pct is not None:
        sys_clean = pd.to_numeric(sys_cpu_pct, errors="coerce").dropna()
        proc_clean = clean.dropna()
        if not proc_clean.empty and not sys_clean.empty:
            proc_median = float(proc_clean.median())
            sys_p95 = float(np.percentile(sys_clean, 95))
            if proc_median > (sys_p95 + 1.0):
                return clean / float(cpu_cores), "core_equivalent"

    return clean, "system"


def plot_single_dashboard(
    df: pd.DataFrame,
    label: str,
    outpath: str,
    suptitle: str,
    cpu_cores: int,
    cpu_mode: str,
    total_ram_mb: float,
) -> None:
    ok_df = split_ok_ko(df)

    rtt_stats = compute_stats(ok_df["rtt_ms"])
    cpu_sys_series, cpu_mode_used = normalize_cpu_to_system(
        ok_df["proc_cpu_pct"], cpu_cores, cpu_mode, ok_df["sys_cpu_pct"]
    )
    cpu_core_stats = compute_stats(ok_df["proc_cpu_pct"])
    cpu_sys_stats = compute_stats(cpu_sys_series)

    if cpu_mode_used == "core_equivalent":
        cpu_core_mean = cpu_core_stats.mean_v
        cpu_core_median = cpu_core_stats.median_v
    else:
        cpu_core_mean = cpu_sys_stats.mean_v * cpu_cores
        cpu_core_median = cpu_sys_stats.median_v * cpu_cores

    ram_stats = compute_stats(ok_df["proc_ram_mb"])
    ram_pct_stats = compute_stats((ok_df["proc_ram_mb"] / total_ram_mb) * 100.0)

    fig, axes = plt.subplots(1, 3, figsize=(18, 5.8))
    fig.suptitle(suptitle, fontsize=14, fontweight="bold")

    ax0 = axes[0]
    rtt_vals = ok_df["rtt_ms"].dropna()
    if not rtt_vals.empty:
        ax0.hist(rtt_vals, bins=40, color="#4c72b0", alpha=0.75, edgecolor="black", linewidth=0.3)
        ax0.axvline(rtt_stats.mean_v, color="red", linestyle="--", linewidth=1.3, label=f"Mean = {rtt_stats.mean_v:.2f} ms")
        ax0.axvline(rtt_stats.median_v, color="green", linestyle="--", linewidth=1.3, label=f"Median = {rtt_stats.median_v:.2f} ms")
        ax0.axvline(rtt_stats.p95_v, color="purple", linestyle="--", linewidth=1.3, label=f"P95 = {rtt_stats.p95_v:.2f} ms")
    ax0.set_title("RTT Distribution")
    ax0.set_xlabel("RTT (ms)")
    ax0.set_ylabel("Frequency")
    ax0.grid(True, alpha=0.25)
    ax0.text(
        0.98,
        0.95,
        fmt_stats_box("RTT", rtt_stats, " ms"),
        transform=ax0.transAxes,
        va="top",
        ha="right",
        fontsize=7.5,
        bbox={"facecolor": "white", "edgecolor": "gray", "alpha": 0.95, "boxstyle": "round,pad=0.35"},
    )
    if ax0.get_legend_handles_labels()[0]:
        ax0.legend(
            loc="upper right",
            bbox_to_anchor=(1.0, 0.68),
            fontsize=8,
            framealpha=0.95,
        )

    ax1 = axes[1]
    cpu_df = ok_df.dropna(subset=["iteration", "proc_cpu_pct"])
    if not cpu_df.empty:
        cpu_sys_vals, _ = normalize_cpu_to_system(
            cpu_df["proc_cpu_pct"], cpu_cores, cpu_mode, cpu_df["sys_cpu_pct"]
        )
        ax1.plot(cpu_df["iteration"], cpu_sys_vals, color="#dd8452", linewidth=1.1, alpha=0.85, label="CPU Usage")
        ax1.axhline(cpu_sys_stats.mean_v, color="red", linestyle="--", linewidth=1.0, label=f"Mean = {cpu_sys_stats.mean_v:.2f}%")
        ax1.axhline(cpu_sys_stats.median_v, color="green", linestyle="--", linewidth=1.0, label=f"Median = {cpu_sys_stats.median_v:.2f}%")
    ax1.set_title("Process CPU Usage")
    ax1.set_xlabel("Iteration")
    ax1.set_ylabel("CPU usage (% of system total)")
    ax1.grid(True, alpha=0.25)
    if ax1.get_legend_handles_labels()[0]:
        ax1.legend(loc="lower right", fontsize=8)
    ax1.text(
        0.98,
        0.20,
        (
            f"CPU Metrics\n"
            f"Mean: {cpu_sys_stats.mean_v:.2f}% of system\n"
            f"({cpu_core_mean:.1f}% core-equivalent)\n"
            f"Median: {cpu_sys_stats.median_v:.2f}% of system\n"
            f"({cpu_core_median:.1f}% core-equivalent)"
        ),
        transform=ax1.transAxes,
        va="bottom",
        ha="right",
        fontsize=7.5,
        bbox={"facecolor": "white", "edgecolor": "gray", "alpha": 0.95, "boxstyle": "round,pad=0.35"},
    )

    ax2 = axes[2]
    ram_df = ok_df.dropna(subset=["iteration", "proc_ram_mb"])
    if not ram_df.empty:
        ax2.plot(ram_df["iteration"], ram_df["proc_ram_mb"], color="#55a868", linewidth=1.1, alpha=0.85, label="RAM Usage")
        ax2.axhline(ram_stats.mean_v, color="red", linestyle="--", linewidth=1.0, label=f"Mean = {ram_stats.mean_v:.2f} MB")
        ax2.axhline(ram_stats.median_v, color="green", linestyle="--", linewidth=1.0, label=f"Median = {ram_stats.median_v:.2f} MB")
    ax2.set_title("Process RAM Usage")
    ax2.set_xlabel("Iteration")
    ax2.set_ylabel("RAM usage (MB)")
    ax2.grid(True, alpha=0.25)
    if ax2.get_legend_handles_labels()[0]:
        ax2.legend(loc="lower right", fontsize=8)
    ax2.text(
        0.98,
        0.20,
        (
            f"RAM Metrics\n"
            f"Mean: {ram_pct_stats.mean_v:.2f}% of system\n"
            f"({ram_stats.mean_v:.2f} MB)\n"
            f"Median: {ram_pct_stats.median_v:.2f}% of system\n"
            f"({ram_stats.median_v:.2f} MB)"
        ),
        transform=ax2.transAxes,
        va="bottom",
        ha="right",
        fontsize=7.5,
        bbox={"facecolor": "white", "edgecolor": "gray", "alpha": 0.95, "boxstyle": "round,pad=0.35"},
    )

    fig.tight_layout(rect=(0, 0, 1, 0.90), w_pad=2.0)
    fig.savefig(outpath, dpi=160)
    plt.close(fig)


def plot_comparison(
    datasets: Dict[str, pd.DataFrame],
    outpath: str,
    suptitle: str,
    cpu_cores: int,
    cpu_mode: str,
) -> None:
    fig, axes = plt.subplots(1, 3, figsize=(18, 5.8))
    fig.suptitle(suptitle, fontsize=14, fontweight="bold")

    for label, df in datasets.items():
        ok_df = split_ok_ko(df)

        rtt = ok_df["rtt_ms"].dropna()
        if not rtt.empty:
            axes[0].hist(rtt, bins=36, alpha=0.35, label=label)

        cpu_df = ok_df.dropna(subset=["iteration", "proc_cpu_pct"])
        if not cpu_df.empty:
            cpu_sys_vals, _ = normalize_cpu_to_system(
                cpu_df["proc_cpu_pct"], cpu_cores, cpu_mode, cpu_df["sys_cpu_pct"]
            )
            axes[1].plot(cpu_df["iteration"], cpu_sys_vals, linewidth=1.1, alpha=0.9, label=label)

        ram_df = ok_df.dropna(subset=["iteration", "proc_ram_mb"])
        if not ram_df.empty:
            axes[2].plot(ram_df["iteration"], ram_df["proc_ram_mb"], linewidth=1.1, alpha=0.9, label=label)

    axes[0].set_title("RTT Distribution")
    axes[0].set_xlabel("RTT (ms)")
    axes[0].set_ylabel("Frequency")
    axes[0].grid(True, alpha=0.25)
    axes[0].legend(fontsize=8)

    axes[1].set_title("Process CPU Usage")
    axes[1].set_xlabel("Iteration")
    axes[1].set_ylabel("CPU usage (% of system total)")
    axes[1].grid(True, alpha=0.25)
    axes[1].legend(fontsize=8)

    axes[2].set_title("Process RAM Usage")
    axes[2].set_xlabel("Iteration")
    axes[2].set_ylabel("RAM usage (MB)")
    axes[2].grid(True, alpha=0.25)
    axes[2].legend(fontsize=8)

    fig.tight_layout(rect=(0, 0, 1, 0.95))
    fig.savefig(outpath, dpi=160)
    plt.close(fig)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Create benchmark dashboards from CSV metrics")
    parser.add_argument(
        "--input",
        required=True,
        metavar="CSV_A",
        help="Primary CSV file to process",
    )
    parser.add_argument(
        "--label",
        default="run_a",
        metavar="LABEL_A",
        help="Label for primary CSV",
    )
    parser.add_argument(
        "--compare",
        default=None,
        metavar="CSV_B",
        help="Optional second CSV for comparison",
    )
    parser.add_argument(
        "--compare-label",
        default="run_b",
        metavar="LABEL_B",
        help="Label for comparison CSV",
    )
    parser.add_argument(
        "--outdir",
        default="plots",
        help="Output folder for generated images (default: plots)",
    )
    parser.add_argument(
        "--title",
        default="Intra-container Communication (Same Host)",
        help="Title prefix for figures",
    )
    parser.add_argument(
        "--cpu-cores",
        type=int,
        default=None,
        help="Number of CPU cores on the machine where CSV was collected (default: local machine)",
    )
    parser.add_argument(
        "--cpu-mode",
        choices=["auto", "system", "core_equivalent"],
        default="auto",
        help="Interpretation of proc_cpu_pct: auto detect, already system percent, or core-equivalent percent",
    )
    parser.add_argument(
        "--system-ram-mb",
        type=float,
        default=None,
        help="Total system RAM (MB) of machine where CSV was collected (default: local machine)",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    os.makedirs(args.outdir, exist_ok=True)

    input_a = args.input
    label_a = args.label
    cpu_cores = args.cpu_cores or (os.cpu_count() or 1)
    total_ram_mb = args.system_ram_mb or get_total_ram_mb()

    df_a = load_csv(input_a)

    dashboard_a = os.path.join(args.outdir, f"dashboard_{label_a}.png")
    plot_single_dashboard(
        df_a,
        label_a,
        dashboard_a,
        args.title,
        cpu_cores,
        args.cpu_mode,
        total_ram_mb,
    )

    print("Generated files:")
    print(f"- {dashboard_a}")
    print(f"CPU cores used for conversion: {cpu_cores}")
    print(f"System RAM used for conversion: {total_ram_mb:.2f} MB")
    if args.system_ram_mb is None:
        print(
            "NOTE: RAM % of system uses local machine RAM. "
            "Pass --system-ram-mb from the machine where CSV was collected for exact values."
        )

    if args.compare:
        label_b = args.compare_label
        df_b = load_csv(args.compare)
        dashboard_b = os.path.join(args.outdir, f"dashboard_{label_b}.png")
        comparison = os.path.join(args.outdir, "dashboard_comparison.png")

        plot_single_dashboard(
            df_b,
            label_b,
            dashboard_b,
            args.title,
            cpu_cores,
            args.cpu_mode,
            total_ram_mb,
        )
        plot_comparison(
            {label_a: df_a, label_b: df_b},
            comparison,
            args.title,
            cpu_cores,
            args.cpu_mode,
        )

        print(f"- {dashboard_b}")
        print(f"- {comparison}")


if __name__ == "__main__":
    main()
