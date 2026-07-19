# Spec: Heterogeneous Computing, Übungsblatt 2 (HC02)

Implementation spec for exercise sheet 2 of "Heterogeneous Computing" (Prof. Sturm, Uni Trier, Sommer 2026).
This document is self-contained: an agent with no other context must be able to implement everything below.

## 0. Context and hard constraints

- **Deliverable:** a subfolder in a git repository, submitted as a pull request against the course repo `2026S-HC` on GitHub. The subfolder contains all code AND a short results report.
- **Deadline:** 2026-07-12. Presentation of ~10 minutes in the exercise session (slide outline is a deliverable here too, see §6).
- **NO personal information anywhere** in the repo: no name, no matriculation number, no email, no username in file contents. (The GitHub account on the PR is unavoidable; file contents must stay clean. Do not add author fields to source headers, CMake, or the report.)
- **Language:** the language for everything is English.
- **Execution environment:** the development machine has NO NVIDIA GPU (AMD Radeon 860M iGPU only, Windows 11). Target CUDA C++ anyway and execute on **Google Colab (T4 GPU)** via the runner in §5. Code must compile with CUDA 12.x and run on any GPU of compute capability ≥ 7.0 (no hardcoded SM counts, clocks, or sizes; query everything at runtime).
- Keep the code minimal and single-purpose. No frameworks, no header-only libraries, no speculative abstraction. Dependencies: CUDA toolkit, C++17, OpenMP (for the CPU baseline), Python 3 with matplotlib + pandas (plots only).

## 1. Repository layout

Create subfolder `blatt02/` (everything for this sheet lives inside it):

```
blatt02/
  README.md              # what this is, how to build, how to run (3 short sections)
  REPORT.md              # the results report, embeds plots from results/plots/
  CMakeLists.txt         # builds all three binaries; plain nvcc lines in README as fallback
  src/
    common.cuh           # timing helpers (cuda events), CUDA_CHECK macro, device info dump
    task1_divergence.cu  # Task 1 benchmark binary
    task2_bandwidth.cu   # Task 2 benchmark binary
    cpu_baseline.cpp     # CPU reference for Task 1 (and optional Task 2 stride/random)
  scripts/
    run_all.sh           # runs every sweep, writes CSVs into results/
    plot.py              # reads results/*.csv, writes results/plots/*.png
  colab/
    run_colab.ipynb      # clone-build-run-download notebook (see §5)
  results/               # committed CSVs and plots from the actual measurement run
    plots/
```

CSV files are the interface between C++ and Python. Every binary prints CSV to stdout (header line included); `run_all.sh` redirects into `results/`. Every CSV also gets a leading comment line `# gpu=<name>, cc=<major.minor>, driver=..., toolkit=...` for provenance.

## 2. Shared infrastructure (`common.cuh`)

- `CUDA_CHECK(call)` macro: abort with file/line on error.
- Timing: `cudaEvent_t` pair around kernel only (no H2D/D2H in the timed region). Convention for all measurements: **3 warmup launches, 10 timed launches, report the median**.
- `print_device_info()`: name, compute capability, SM count, max threads/SM, warp size, global mem, L2 size, memory clock and bus width if available via `cudaDeviceGetAttribute` (guard: attributes may be unavailable on newer toolkits; print "n/a" then). Also compute theoretical peak bandwidth `2 * memClockHz * busWidthBytes` when the attributes exist. This replaces `deviceQuery`.
- All binaries take CLI flags (trivial hand-rolled parsing, no library): documented via `--help`.

## 3. Task 1: SIMT and warp divergence (compute-bound)

### 3.1 Kernel design

One thread per element, minimal memory traffic: read one `float`, run `k` iterations of pure arithmetic, write one `float`.

Inner load: an iterated FMA chain, e.g. `x = fmaf(x, a, b)` with per-iteration constant variation so the compiler cannot collapse the loop. Count 2 FLOP per FMA. The result must be written to global memory so nothing is dead-code-eliminated. Use `float`, `-use_fast_math` off (keep it honest), `-O3`.

### 3.2 Experiment A: throughput scaling over n (no divergence)

- Sweep `n` ∈ {2^10, 2^12, ..., 2^26} (up to 64M), fixed `k = 1024`, block size 256.
- Metric: GFLOP/s = `2 * n * k / t_median`.
- CPU baseline (`cpu_baseline.cpp`): identical loop, (a) single thread, (b) OpenMP all cores. Same metric.
- CSV: `task1_scaling.csv` with columns `device,n,k,block,time_ms,gflops` (device ∈ gpu, cpu1, cpuN).
- The report must read off: at which `n` the GPU saturates ("warmläuft"), how many threads that is, and how many threads/SM, versus the CPU which needs no such scale.

### 3.3 Experiment B: controlled warp divergence

- Divergence degree `d` ∈ {1, 2, 4, 8, 16, 32}: branch on `threadIdx.x % d`, so a warp executes `d` distinct paths.
- **Critical design rule: every path performs the same amount of work `k`** so any slowdown is pure serialization, not workload imbalance. Ideal expectation: slowdown ≈ d.
- **Pitfall (must handle):** if the `d` branch bodies are structurally identical and differ only in constants, `nvcc` will merge them into one path with a constant lookup and divergence disappears. Make each path operationally distinct (different arithmetic mixes per case, e.g. rotate between `fmaf(x,a,b)`, `x*x+c`, `fmaf(x,x,a)`, division-free variants), generated by a macro or template unrolling so the 32 cases stay maintainable. **Verify** the effect is real: measured slowdown at d=32 must be roughly an order of magnitude or more vs d=1; if it is not, inspect with `--ptxas-options=-v` / disassembly and fix before proceeding.
- Fixed `n = 2^24`, `k = 1024`, block 256.
- CPU comparison: same `%d` branching in the OpenMP baseline; expectation: near-flat over d (branch prediction, independent cores). Measure it, do not just claim it.
- Secondary variant (small, cheap to add): data-dependent loop length `k_i = k * (1 + (i % d))/…` normalized to equal total work, to show divergence via loop trip count. One extra CSV column `mode ∈ {branch, looplen}`.
- CSV: `task1_divergence.csv` with `device,mode,d,n,k,time_ms,gflops,slowdown_vs_d1`.

### 3.4 Plots (plot.py)

1. `task1_scaling.png`: GFLOP/s vs n (log-x), three lines (GPU, CPU 1 thread, CPU all cores). Mark the GPU saturation point.
2. `task1_divergence.png`: relative throughput vs d, GPU and CPU lines, plus the ideal `1/d` reference curve.

## 4. Task 2: memory access, latency hiding, bandwidth (memory-bound)

### 4.1 Kernel design

Streaming kernel with minimal arithmetic intensity: `b[j] = a[j] * c` with three addressing patterns over `float` arrays:

1. **coalesced**: `j = i` (stride 1)
2. **strided**: `j = (i * stride) % n` (stride ∈ {2, 4, 8, 16, 32, 64, 128}); ensure n and stride are coprime-safe via power-of-two n and modulo, and that each element is touched exactly once (use `j = (i % (n/stride)) * stride + i/(n/stride)` or simply restrict to `n` divisible by stride with the standard `(i*stride) % n + offset` trick; correctness check: sum over b equals sum over a * c within tolerance).
3. **random gather**: `b[i] = a[idx[i]] * c` with `idx` a precomputed uniform random permutation (fixed seed 42, generated on host, copied once outside the timed region).

- Metric: effective bandwidth GB/s. Byte convention: **8 bytes per element** (4 read + 4 write) for patterns 1 and 2; for gather report the same 8 B/element as "payload bandwidth" and add a footnote in the report that the 4 B/element index traffic is on top (optionally a second column `bw_incl_idx` with 12 B/element).
- `n = 2^26` (256 MiB per array; check free device memory first and halve if needed).
- Reference point: theoretical peak from §2 if attributes are available, AND a measured practical peak (best coalesced result). Report both; express every measurement as % of practical peak.

### 4.2 Experiment C: pattern comparison

- Sweep the three patterns (strided over its stride set), block 256, full grid.
- CSV: `task2_patterns.csv` with `pattern,stride,n,time_ms,gbps,pct_of_peak`.

### 4.3 Experiment D: latency hiding via occupancy

- Take the coalesced kernel and vary the number of resident warps two ways:
  1. **Block size sweep:** {32, 64, 128, 256, 512, 1024}.
  2. **Occupancy throttle:** fixed block 256, allocate unused dynamic shared memory per block to cap resident blocks/SM, sweeping the cap so theoretical occupancy goes ~12.5% → 100% in steps.
- For every configuration compute **theoretical occupancy** with `cudaOccupancyMaxActiveBlocksPerMultiprocessor`: `occupancy = activeWarpsPerSM / maxWarpsPerSM`. Print it into the CSV. (Measured occupancy via Nsight Compute `sm__warps_active.avg.pct_of_peak_sustained` is usually blocked on Colab due to counter permissions; try `ncu` once, and if it fails note that in the report and rely on the API value.)
- Use a grid-stride loop so total work is identical across configurations.
- CSV: `task2_occupancy.csv` with `knob,block,smem_bytes,active_warps_per_sm,occupancy,time_ms,gbps`.

### 4.4 Plots

3. `task2_patterns.png`: bandwidth (GB/s) per pattern; strided as line over stride (log-x), coalesced and random as reference hlines; peak line annotated.
4. `task2_occupancy.png`: bandwidth vs occupancy (%), both knobs as two series; expected shape: rising then flat once enough warps hide the latency.

## 5. Colab runner (`colab/run_colab.ipynb`)

Notebook cells, in order:

1. `!nvidia-smi` and assert a GPU is present.
2. Clone the repo (parameterized URL/branch at the top of the notebook), `cd blatt02`.
3. Build: `cmake -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build -j` (fallback cell with raw `nvcc -O3 -std=c++17 -arch=native` lines).
4. `bash scripts/run_all.sh` (writes `results/*.csv`).
5. `python scripts/plot.py`.
6. Zip `results/` and offer download (`files.download`).

`run_all.sh` must be idempotent and finish in well under 10 minutes on a T4 (size the repetition counts accordingly; the sweeps above fit comfortably).

## 6. Report (`REPORT.md`) and presentation

Structure (short aim for 2–4 pages equivalent):

1. **Setup**: GPU (from device info dump), toolkit version, measurement methodology (median of 10, events, warmups).
2. **Aufgabe 1**: scaling plot + saturation point (n, thread count, threads/SM), divergence plot + measured slowdowns vs ideal 1/d, CPU comparison numbers. Then the required explanation: why a warp serializes divergent paths (single program counter per warp pre-Volta, active-mask execution; on Volta+ independent thread scheduling still serializes divergent instruction streams), and why the same branch is nearly free on a CPU (per-core independent control flow, branch prediction, speculative execution).
3. **Aufgabe 2**: pattern plot with % of peak, occupancy plot. Explanation: GPU hides latency by warp oversubscription (needs enough resident warps, hence occupancy matters), CPU hides it via cache hierarchy + prefetching (sequential fast, random slow due to misses). Consequence for data layout on GPUs: structure-of-arrays, coalesced 32/128-byte transactions, avoid indirection/gather where possible, pad/align to transaction boundaries.
4. **Limitationen**: theoretical occupancy instead of profiled (Colab counter permissions), single GPU model, no ECC/clock-boost control.

All numeric claims in the report must come from the committed CSVs. Embed plots with relative paths (`results/plots/...`).

Presentation: add `SLIDES.md` (outline only, ~8 slides: motivation, SIMT model, exp A result, exp B result, why serialization, memory patterns result, occupancy result, takeaways). 10 minutes.

## 7. Acceptance checklist

- [ ] `cmake --build` succeeds with CUDA 12.x, no warnings from `-Wall -Wextra` on host code.
- [ ] All three binaries run with `--help` and with defaults.
- [ ] Divergence effect verified: d=32 slowdown ≥ ~8x vs d=1 on GPU (else investigate compiler merging, §3.3).
- [ ] CPU divergence measured, near-flat over d.
- [ ] Coalesced bandwidth ≥ ~70% of theoretical peak on T4; random gather dramatically lower.
- [ ] Occupancy sweep shows the rise-then-plateau shape.
- [ ] Strided/gather kernels validated numerically (checksum vs reference) once per run.
- [ ] `results/` contains the real CSVs + PNGs from the Colab run; REPORT.md references only those numbers.
- [ ] grep of repo for personal data (name/matrikel/email) is clean.
- [ ] Everything lives under `blatt02/`; nothing outside it is touched in the PR.

## 8. Out of scope

- No shared-memory tiling, no streams, no multi-GPU, no half precision, no CUB/Thrust.
- No CI, no unit test framework (the checksum validation above suffices).
- The PR itself (fork, branch, submit) is done manually by the author, not by the agent, but write the README so the folder is PR-ready.
