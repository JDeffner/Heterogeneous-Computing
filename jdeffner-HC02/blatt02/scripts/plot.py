#!/usr/bin/env python3
"""Reads results/*.csv, writes results/plots/*.png and prints the key numbers
used in REPORT.md. Optional argument: path to the results directory."""

import os
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import pandas as pd

RES = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "results")
PLOTS = os.path.join(RES, "plots")
os.makedirs(PLOTS, exist_ok=True)


def load(name):
    return pd.read_csv(os.path.join(RES, name), comment="#")


def provenance(name, key):
    """Extract key=value from the leading '# ...' comment line, or None."""
    with open(os.path.join(RES, name)) as f:
        line = f.readline()
    for part in line.lstrip("# ").strip().split(", "):
        if part.startswith(key + "="):
            value = part.split("=", 1)[1]
            try:
                return float(value)
            except ValueError:
                return None
    return None


def save(fig, name):
    fig.tight_layout()
    fig.savefig(os.path.join(PLOTS, name), dpi=150)
    plt.close(fig)
    print(f"wrote plots/{name}")


# --- 1) Task 1, experiment A: GFLOP/s vs n ---------------------------------
df = load("task1_scaling.csv")
fig, ax = plt.subplots(figsize=(7, 4.5))
for dev, label, style in [("gpu", "GPU", "o-"),
                          ("cpu1", "CPU, 1 thread", "s-"),
                          ("cpuN", "CPU, all cores", "^-")]:
    sub = df[df.device == dev].sort_values("n")
    if not sub.empty:
        ax.plot(sub.n, sub.gflops, style, label=label)

gpu = df[df.device == "gpu"].sort_values("n")
gpu_peak = gpu.gflops.max()
sat_n = int(gpu[gpu.gflops >= 0.95 * gpu_peak].n.min())
ax.axvline(sat_n, color="gray", ls=":", lw=1)
ax.annotate(f"saturation\nn = 2^{sat_n.bit_length() - 1}", (sat_n, gpu_peak),
            textcoords="offset points", xytext=(8, -30), fontsize=9)
ax.set_xscale("log", base=2)
ax.set_yscale("log")
ax.set_xlabel("n (elements)")
ax.set_ylabel("GFLOP/s")
ax.set_title("Task 1A: FMA throughput vs problem size (k = 1024)")
ax.grid(True, which="both", alpha=0.3)
ax.legend()
save(fig, "task1_scaling.png")

# --- 2) Task 1, experiment B: relative throughput vs divergence degree -----
df = load("task1_divergence.csv")
fig, ax = plt.subplots(figsize=(7, 4.5))
for dev, mode, label, style in [("gpu", "branch", "GPU, branch", "o-"),
                                ("gpu", "looplen", "GPU, loop length", "o--"),
                                ("cpuN", "branch", "CPU, branch", "^-"),
                                ("cpuN", "looplen", "CPU, loop length", "^--")]:
    sub = df[(df.device == dev) & (df["mode"] == mode)].sort_values("d")
    if not sub.empty:
        ax.plot(sub.d, 1.0 / sub.slowdown_vs_d1, style, label=label)
ds = sorted(df.d.unique())
ax.plot(ds, [1.0 / d for d in ds], "k:", label="ideal 1/d")
ax.set_xscale("log", base=2)
ax.set_yscale("log", base=2)
ax.set_xticks(ds, [str(d) for d in ds])
ax.set_xlabel("divergence degree d (paths per warp)")
ax.set_ylabel("throughput relative to d = 1")
ax.set_title("Task 1B: warp divergence")
ax.grid(True, which="both", alpha=0.3)
ax.legend()
save(fig, "task1_divergence.png")

# --- 3) Task 2, experiment C: bandwidth per access pattern ------------------
df = load("task2_patterns.csv")
peak_theo = provenance("task2_patterns.csv", "peak_theoretical_gbps")
fig, ax = plt.subplots(figsize=(7, 4.5))
strided = df[df.pattern == "strided"].sort_values("stride")
coalesced = float(df[df.pattern == "coalesced"].gbps.iloc[0])
random_bw = float(df[df.pattern == "random"].gbps.iloc[0])
ax.plot(strided.stride, strided.gbps, "o-", label="strided")
ax.axhline(coalesced, color="tab:green", ls="-", lw=1.2,
           label=f"coalesced (practical peak, {coalesced:.0f} GB/s)")
ax.axhline(random_bw, color="tab:red", ls="--", lw=1.2,
           label=f"random gather ({random_bw:.0f} GB/s)")
if peak_theo:
    ax.axhline(peak_theo, color="gray", ls=":",
               label=f"theoretical peak ({peak_theo:.0f} GB/s)")
ax.set_xscale("log", base=2)
ax.set_xticks(strided.stride, [str(s) for s in strided.stride])
ax.set_xlabel("stride (elements)")
ax.set_ylabel("effective bandwidth (GB/s)")
ax.set_title("Task 2C: bandwidth by access pattern (8 B/element)")
ax.grid(True, which="both", alpha=0.3)
ax.legend()
save(fig, "task2_patterns.png")

# --- 4) Task 2, experiment D: bandwidth vs occupancy ------------------------
df = load("task2_occupancy.csv")
fig, ax = plt.subplots(figsize=(7, 4.5))
for knob, label, style in [("block", "block size sweep", "o-"),
                           ("smem", "smem throttle (block 256)", "s--")]:
    sub = df[df.knob == knob].sort_values("occupancy").reset_index(drop=True)
    ax.plot(100 * sub.occupancy, sub.gbps, style, label=label)
    for i, r in sub.iterrows():
        tag = f"b{int(r.block)}" if knob == "block" else f"{int(r.smem_bytes) // 1024}K"
        dy = 5 + 9 * (i % 3) if knob == "block" else -11 - 9 * (i % 3)
        ax.annotate(tag, (100 * r.occupancy, r.gbps), fontsize=7,
                    textcoords="offset points", xytext=(3, dy))
ax.set_xlabel("theoretical occupancy (%)")
ax.set_ylabel("effective bandwidth (GB/s)")
ax.set_title("Task 2D: latency hiding, bandwidth vs occupancy")
ax.grid(True, alpha=0.3)
ax.legend()
save(fig, "task2_occupancy.png")

# --- Summary numbers for REPORT.md ------------------------------------------
print("\n--- summary for REPORT.md ---")
print(f"GPU peak: {gpu_peak:.0f} GFLOP/s, saturation at n = {sat_n} (2^{sat_n.bit_length() - 1})")
sc = load("task1_scaling.csv")
for dev in ("cpu1", "cpuN"):
    sub = sc[sc.device == dev]
    if not sub.empty:
        print(f"{dev}: {sub.gflops.max():.1f} GFLOP/s (max over n)")
dv = load("task1_divergence.csv")
for dev, mode in [("gpu", "branch"), ("gpu", "looplen"), ("cpuN", "branch")]:
    row = dv[(dv.device == dev) & (dv["mode"] == mode) & (dv.d == 32)]
    if not row.empty:
        print(f"slowdown d=32, {dev}/{mode}: {float(row.slowdown_vs_d1.iloc[0]):.2f}x")
pt = load("task2_patterns.csv")
print(f"coalesced: {coalesced:.0f} GB/s"
      + (f" = {100 * coalesced / peak_theo:.0f}% of theoretical peak" if peak_theo else ""))
print(f"random gather: {random_bw:.0f} GB/s = {100 * random_bw / coalesced:.0f}% of practical peak")
worst = pt[pt.pattern == "strided"].gbps.min()
print(f"worst stride: {worst:.0f} GB/s = {100 * worst / coalesced:.0f}% of practical peak")
oc = load("task2_occupancy.csv")
print(f"occupancy sweep: {oc.gbps.min():.0f} -> {oc.gbps.max():.0f} GB/s "
      f"over {100 * oc.occupancy.min():.0f}% -> {100 * oc.occupancy.max():.0f}% occupancy")
