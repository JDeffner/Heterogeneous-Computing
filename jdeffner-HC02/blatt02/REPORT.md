# Results report: exercise sheet 2 (SIMT, divergence, memory bandwidth)

All numbers in this report come from the committed CSVs in `results/`
(the summary block printed by `scripts/plot.py` reproduces them).

## 1. Setup

- **GPU:** Tesla T4 (Google Colab), compute capability 7.5, 40 SMs,
  max. 1024 threads/SM, theoretical peak bandwidth 320.1 GB/s
  (`2 * memory clock * bus width` = 2 x 5001 MHz x 32 B, queried at runtime).
- **Toolkit/driver:** CUDA 12.8, driver 13.0 (provenance line of the CSVs).
- **CPU reference:** Colab host VM, 2 threads (OpenMP `omp_max_threads=2`).
- **Methodology:** per configuration 3 warmup runs, then 10 timed runs,
  the median is reported. GPU times via `cudaEvent` pairs around the kernel
  only (no H2D/D2H in the timed window). No fast math, `-O3`, FLOP
  convention: 2 FLOP per FMA. All metrics come from `results/*.csv`.

## 2. Task 1: SIMT and warp divergence (compute-bound)

Kernel: one thread per element, read one `float`, run `k = 1024` FMA
iterations, write one `float`. Metric: GFLOP/s = `2 n k / t`.

### 2.1 Experiment A: scaling over n

![Scaling](results/plots/task1_scaling.png)

- The GPU reaches its full throughput only from n ≈ 2^20 (1,048,576
  threads; with 40 SMs that is ~26,000 scheduled threads per SM, i.e. ~26
  full waves of the 40,960 threads that can be resident at once). Below
  that there are too few warps to hide instruction latency and to fill all
  SMs; at very small n the launch overhead dominates.
- GPU plateau: ~4,365 GFLOP/s. The largest point (n = 2^26) measured
  6,853 GFLOP/s; we attribute the jump to the T4 boost clock ramping up
  during the run (no clock control on Colab, see §4), and use the plateau
  as the honest throughput figure.
- CPU: 1.5 GFLOP/s (1 thread) and 2.9 GFLOP/s (both cores), essentially
  flat over n from the smallest size on: a CPU needs no mass parallelism
  to reach its (much lower) peak.
- Ratio GPU to CPU (both cores): ~1,500x. Two caveats: the Colab VM has
  only 2 CPU cores, and the workload is a serially dependent FMA chain
  behind a `noinline` call, so the CPU runs scalar code far below its own
  vectorized FP32 peak. The ratio compares this specific workload, not
  architecture peaks.

### 2.2 Experiment B: controlled warp divergence

Divergence degree d ∈ {1, 2, 4, 8, 16, 32}: branch on `threadIdx.x % d`,
one warp executes d distinct paths. Every path does exactly the same work
(k iterations, 1 FMA each), but the paths are operationally distinct (own
constants and instruction order, `__noinline__`) so the compiler cannot
merge them. Expectation under full serialization: slowdown ≈ d.

![Divergence](results/plots/task1_divergence.png)

| d | GPU slowdown (branch) | ideal | CPU slowdown (branch) |
|---|---|---|---|
| 1 | 1.00 | 1 | 1.00 |
| 2 | 1.46 | 2 | 1.00 |
| 4 | 2.80 | 4 | 1.75 |
| 8 | 5.37 | 8 | 1.03 |
| 16 | 10.66 | 16 | 1.03 |
| 32 | 21.32 | 32 | 1.02 |

- GPU: the slowdown at d = 32 is 21.3x, i.e. more than an order of
  magnitude. The effect is real and not a compiler artifact (the spec's
  verification threshold of ~8x is met with margin).
- The slowdown stays at roughly 2/3 of the ideal d across the sweep. This
  is consistent with the d = 1 baseline being latency-limited: each thread
  is one dependent FMA chain, so a single path cannot keep the FMA pipes
  full by itself. Divergent paths are independent instruction streams, and
  the Volta+ scheduler (independent thread scheduling) can interleave
  them, recovering part of the serialization cost. The paths still execute
  serially per cycle, hence the near-proportional growth.
- CPU (OpenMP, same `%d` branching): flat over d within noise (all points
  ≤ 1.03x except a single 1.75x outlier at d = 4 on the shared 2-core VM).
  Measured, not just claimed.
- Secondary variant `looplen` (data-dependent trip count, normalized to
  equal total work): the slowdown saturates at 1.92x. Expected: the warp
  time is the maximum of the trip counts, and `max(k_i) ≈ 2k` independent
  of d, while total work stays constant.

**Why does the GPU serialize divergent paths?** A warp (32 threads) is the
execution unit of the SIMT model: before Volta there is a single program
counter per warp. On a branch that lanes take differently, the hardware
executes the paths one after another, masking the inactive lanes (active
mask). With d paths, a fraction of the lanes is active d times in
sequence, so execution time grows by roughly a factor of d. From Volta on
(independent thread scheduling) every thread has its own PC, but a warp
scheduler still issues one instruction per cycle for a group of convergent
lanes: divergent instruction streams remain serialized in time, only the
interleaving is more flexible, which is exactly the 2/3-of-ideal effect
measured above.

**Why is the same branch nearly free on a CPU?** Every core has its own
independent control flow; there is no lock-step group that must wait for
both paths. In addition, the pattern `i % d` is perfectly periodic and is
predicted essentially without misses by the branch predictor; speculative
out-of-order execution hides the remaining cost.

## 3. Task 2: memory access, latency hiding, bandwidth (memory-bound)

Streaming kernel `b[j] = a[j] * c`, n = 2^26 elements (256 MiB per array),
metric: effective bandwidth at 8 B/element (4 B read + 4 B write).

### 3.1 Experiment C: access patterns

![Access patterns](results/plots/task2_patterns.png)

- **Coalesced** (stride 1): 247.1 GB/s = 77% of the theoretical peak
  bandwidth (320.1 GB/s). This measurement serves as the practical peak
  (100%) that all percentages below refer to.
- **Strided:** falls steeply with growing stride: 87.3 GB/s (35%) at
  stride 2, 20.5 GB/s (8%) at stride 8, down to 6.4 GB/s (2.6%) at stride
  128. From stride ≈ 16 on it is at or below the random-gather level.
  Reason: instead of one contiguous 128-B transaction per warp, up to 32
  separate memory segments are touched; of every 32-B sector fetched from
  DRAM only 4 B are used.
- **Random gather:** 14.4 GB/s = 5.8% of the practical peak. Footnote on
  the byte convention: this is the payload bandwidth at 8 B/element; the
  4 B/element for reading the index array are physically on top (counting
  12 B/element the figure would be 21.6 GB/s).
- Validation: the checksums of all variants match the reference
  (relative error 0.0, see `results/run.log` of the measurement run).

### 3.2 Experiment D: latency hiding via occupancy

Grid-stride kernel (identical total work), number of resident warps varied
via (1) block size 32 to 1024 and (2) unused dynamic shared memory per
block as an occupancy throttle at block 256. Occupancy is the theoretical
value from `cudaOccupancyMaxActiveBlocksPerMultiprocessor`.

![Occupancy](results/plots/task2_occupancy.png)

- Shape as expected: bandwidth rises from 158 GB/s at 25% occupancy
  (8 warps/SM) to ~187 GB/s at 50% and is flat from there on (183–187
  GB/s at 50–100%, differences within run-to-run noise): from ~16 resident
  warps per SM the memory latency is fully hidden and DRAM bandwidth
  becomes the limit. Notably, even 25% occupancy already reaches ~85% of
  the plateau for this purely streaming kernel.
- Granularity note: with block 256 on the T4 (max. 32 warps/SM, 4 blocks
  of 8 warps) only the steps 25/50/100% materialize; the intended 75%
  step collapses into the 50% one because shared-memory allocation
  granularity rounds the 21.3 KiB throttle up. The block-size series
  bottoms out at 50% (at block 32 the 16-blocks/SM limit caps residency
  at 16 warps).
- The absolute level (~185 GB/s) is below the coalesced peak of experiment
  C (247 GB/s): the grid covers only the resident blocks, so each thread
  loops over many elements, which adds loop overhead and limits the number
  of independent loads in flight per thread.

**Interpretation:** the GPU hides memory latency by oversubscription with
warps: as long as enough warps are resident, the scheduler switches to a
runnable warp on every memory stall, hence the occupancy dependence. The
CPU hides latency with its cache hierarchy and hardware prefetching
instead: sequential accesses are fast, random accesses cause cache and TLB
misses. Consequence for data layout on GPUs: structure-of-arrays instead
of array-of-structures, arrange accesses so a warp generates contiguous
32/128-B transactions, avoid indirection/gather, align/pad data to
transaction boundaries.

## 4. Limitations

- **Theoretical instead of profiled occupancy:** Nsight Compute needs
  hardware counter permissions that the Colab runtime does not grant, so
  the automated run relies on the API value; `ncu` was not run.
- **A single GPU model** (Colab T4); no claim about other architectures,
  but all sizes and limits are queried at runtime.
- **No control over ECC and clock boost** on the cloud GPU. The median of
  10 launches damps outliers within a configuration, but not clock drift
  between configurations; visible as the 6,853 GFLOP/s outlier at
  n = 2^26 in experiment A and as level differences between runs (the
  d = 1 divergence baseline measured 5,395 GFLOP/s vs. 4,373 GFLOP/s for
  the same configuration in the scaling run).
- **Weak CPU reference:** 2 cores only on the Colab VM, and the workload
  (dependent scalar FMA chain behind `noinline`) prevents vectorization,
  so the CPU numbers are far below the host CPU's peak; see the caveat in
  §2.1.
- CPU sweeps use smaller n than the GPU sweeps (runtime budget); since CPU
  throughput is flat over n, this does not distort the comparison.
