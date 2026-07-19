# HC02, explained line by line

This document explains everything in `blatt02/`: what each file does, what each
non-obvious line means, and why it was written that way. It assumes you know
the course material (SIMT, warps, occupancy, coalescing) and general
programming, but **no Python, no CMake, no bash** and only basic CUDA.

This file is intentionally *outside* `blatt02/` so it does not end up in the PR.

---

## 0. The big picture

The project is a measurement pipeline. Five kinds of files, four languages,
one direction of data flow:

```
 C++/CUDA binaries          bash               Python              Markdown
┌──────────────────┐   ┌─────────────┐   ┌───────────────┐   ┌──────────────┐
│ task1_divergence │   │             │   │               │   │              │
│ task2_bandwidth  ├──►│ run_all.sh  ├──►│   plot.py     ├──►│  REPORT.md   │
│ cpu_baseline     │   │ (runs them, │   │ (reads CSVs,  │   │ (embeds the  │
│ (print CSV to    │   │  saves CSVs)│   │  writes PNGs) │   │  PNGs, cites │
│  the terminal)   │   │             │   │               │   │  the numbers)│
└──────────────────┘   └─────────────┘   └───────────────┘   └──────────────┘
```

Two design decisions shape everything:

1. **CSV is the interface.** The C++ programs do not draw plots and Python
   does not measure anything. Each binary prints a comma-separated table to
   its standard output; the shell script redirects that text into files under
   `results/`; Python reads those files. The benefit: every intermediate
   result is a plain text file you can open, inspect, commit to git, and cite
   in the report. Nothing is hidden in program memory.

2. **stdout vs stderr.** Every program has two output channels. *Standard
   output* (stdout) is for the machine-readable CSV. *Standard error*
   (stderr) is for everything meant for humans: the device info dump,
   validation messages, error messages. This matters because `run_all.sh`
   redirects stdout into the CSV file; if the device dump went to stdout too,
   it would corrupt the CSV. With the split, you still see the human messages
   in the terminal while the clean CSV lands in the file.

Your laptop has an AMD GPU, so the CUDA parts cannot run locally. The
pipeline is therefore designed to run unattended on Google Colab (free NVIDIA
T4): the notebook `colab/run_colab.ipynb` clones the repo, builds, runs
`run_all.sh`, runs `plot.py`, and hands you a zip of `results/`.

---

## 1. `src/common.cuh`: shared plumbing

A `.cuh` file is a CUDA header: code that is textually pasted into every
`.cu` file that writes `#include "common.cuh"`. The first line, `#pragma
once`, tells the compiler "if this file gets included twice, only paste it
once", which prevents duplicate-definition errors.

### 1.1 `CUDA_CHECK`: never ignore an error

Almost every CUDA API function returns an error code instead of throwing an
exception. If you ignore those codes, a failed memory allocation silently
produces garbage numbers ten lines later. So every single CUDA call in this
project is wrapped:

```c
CUDA_CHECK(cudaMalloc(&da, sizeof(float) * n));
```

`CUDA_CHECK` is a *macro*: a text substitution performed before compilation.
It expands to "run the call, store the returned code, and if it is not
`cudaSuccess`, print the file name and line number and kill the program".
`__FILE__` and `__LINE__` are built-in placeholders the compiler replaces
with the location of the macro use, so an error message points at the exact
offending line.

The odd-looking `do { ... } while (0)` wrapper is a standard C idiom: it
turns the multi-statement macro body into a single statement, so the macro
behaves correctly inside an `if`/`else` without braces.

### 1.2 Timing with CUDA events

Timing GPU code has a trap: a kernel launch is *asynchronous*. The line
`kernel<<<grid, block>>>(...)` returns immediately, while the GPU is still
working. If you timed it with a normal CPU clock, you would measure the cost
of *enqueueing* the launch (microseconds), not of running it.

CUDA events solve this. An event is a timestamp marker inserted into the
GPU's own command stream:

```
cudaEventRecord(start);   // marker before the kernel
kernel<<<...>>>(...);
cudaEventRecord(stop);    // marker after the kernel
cudaEventSynchronize(stop);  // CPU waits until the GPU reached 'stop'
cudaEventElapsedTime(&ms, start, stop);  // GPU-side time between markers
```

Because both markers live on the GPU timeline, the measured span contains
exactly the kernel and nothing else. In particular, host-to-device and
device-to-host copies are *outside* the timed region, as the spec requires.

`time_median_ms()` wraps this in the measurement convention used everywhere:

- **3 warmup launches** first. The very first launch of a kernel pays
  one-time costs (code upload to the GPU, cache population, clock ramp-up).
  Warmups absorb that so it does not pollute the measurement.
- **10 timed launches, report the median.** A cloud GPU is a noisy
  environment. The *median* (sort the 10 values, take the middle) is robust
  against outliers, unlike the mean, where one hiccup shifts the result.
  With an even count there are two middle values, so the code averages the
  5th and 6th.

`time_median_ms` takes the code to run as a parameter, written at the call
site as a *lambda*, an unnamed inline function:

```c
float t = time_median_ms([&] { kernel_uniform<<<grid, block>>>(da, db, n, k); });
```

`[&]` means "the body may use the surrounding variables (`grid`, `da`, ...)".
This way one timing function serves every kernel in the project.

### 1.3 Device info and provenance

`query_device_info()` asks the driver for the GPU's properties: name, compute
capability, number of SMs, and so on. This replaces the `deviceQuery` sample
the sheet mentions. Two attributes need special care: memory clock and bus
width were *removed from newer CUDA toolkits*. The code therefore checks the
return value of `cudaDeviceGetAttribute` instead of using `CUDA_CHECK`, and
prints `n/a` when the attribute is gone. When both exist, the theoretical
peak bandwidth is computed as

```
peak = 2 × memory clock [Hz] × bus width [bytes]
```

The factor 2 is DDR memory: it transfers data on both edges of the clock.

`print_csv_provenance()` prints one comment line at the top of each CSV, e.g.
`# gpu=Tesla T4, cc=7.5, driver=12.4, toolkit=12.4`. Months later you can
still tell which hardware produced a given results file. The leading `#`
marks it as a comment, and `plot.py` tells pandas to skip such lines.

---

## 2. `src/task1_divergence.cu`: the compute benchmark

### 2.1 Thirty-second CUDA refresher

A `__global__` function is a *kernel*: code that runs on the GPU, once per
thread. The launch `kernel<<<grid, block>>>(args)` starts `grid × block`
threads, organized in blocks of `block` threads. Inside the kernel each
thread computes its own global index:

```c
int i = blockIdx.x * blockDim.x + threadIdx.x;
if (i < n) ...   // guard: the last block may be partly out of range
```

Hardware-wise, each group of 32 consecutive threads of a block is a *warp*
and executes in lockstep. That is the whole reason Task 1 exists.

### 2.2 The workload: an FMA chain

The spec wants a *compute-bound* kernel: minimal memory traffic, maximal
arithmetic. Each thread reads one `float`, runs `k = 1024` iterations of
arithmetic on it, writes one `float`. The arithmetic unit is
`fmaf(x, a, b)`, the fused multiply-add `x*a + b`: one instruction, counted
as 2 FLOP, which makes the GFLOP/s bookkeeping trivial
(`FLOPs = 2 · n · k`).

Three traps had to be avoided, and they explain the strange-looking details:

- **Dead-code elimination.** If the result were never used, the compiler
  would delete the entire loop and you would measure an empty kernel. Hence
  every thread writes its result to global memory (`b[i] = ...`).
- **Loop collapsing.** If the loop were mathematically foldable, the
  compiler could replace 1024 iterations with a closed formula. It is not,
  for two reasons: floating-point math is not associative, so without
  `-use_fast_math` (deliberately off) the compiler must not reorder it; and
  each iteration feeds its result into the next (`x = fmaf(x, ...)`), a
  serial dependence chain with varying constants.
- **Numeric blow-up.** 1024 repeated multiplications explode to infinity or
  decay to zero unless the map is chosen carefully. Every operation used is
  a *contraction toward 1*: `x*a + (1-a)` with `a` slightly below 1 has
  fixed point exactly 1, and `x*(1+m)` with tiny negative `m` shrinks x
  slightly. Values stay in a small band around 1 forever. That keeps the
  numbers honest (no special-case timing for infinities or denormals).

### 2.3 Experiment A: throughput scaling

`run_scaling()` sweeps n over 2^10, 2^12, ..., 2^26 with everything else
fixed, timing the *uniform* kernel (every thread runs the identical path 0).
The point: a GPU only reaches its rated GFLOP/s when there are vastly more
threads than cores, because it hides instruction latency by switching
between resident warps. Small n = few warps = idle SMs. The CSV lets the
report read off exactly where saturation happens.

Implementation notes: the arrays are allocated once at the maximum size and
the sweep only varies how many elements the kernel touches, which avoids
re-allocating per step. `n <<= 2` is a bit-shift: multiply n by 4 each
iteration, producing the 2^10, 2^12, ... progression.

### 2.4 Experiment B: controlled divergence, and the compiler war

Goal: force a warp to execute `d` *different* code paths and measure the
slowdown. The kernel branches on `threadIdx.x % d` (the thread index within
the block, modulo d). Since d divides 32, each warp contains all d residues,
so each warp must run all d paths. SIMT hardware serializes them, so the
ideal expectation is a slowdown of exactly d.

The hard part is not the branching, it is *keeping* the branching. The spec
warns explicitly: if the d branch bodies are structurally identical and
differ only in constants, the compiler will merge them into one path with a
constant lookup, and your "divergence benchmark" measures nothing. Three
defenses are layered here:

1. **Distinct constants per path.** Path P uses constants derived from P
   (`0.999 - 0.00001·P` etc.), so no two paths are byte-identical.
2. **Distinct instruction order per path.** Even paths run the four FMAs in
   order A,B,A',B'; odd paths in order B,A',B',A. Different dataflow means
   the "merge identical code" optimization cannot fire.
3. **`__noinline__`.** Each path is compiled as a genuinely separate
   function that the compiler must call, not paste inline. Separate
   functions cannot be merged into one branch body at all.

At the same time, **every path does exactly the same amount of work**: k
iterations, one FMA each. This is the critical design rule from the spec.
If path 3 were more expensive than path 0, a slowdown could be blamed on
workload imbalance; with equal work, any slowdown is pure serialization.

How the 32 paths are generated without writing 32 functions by hand:
`run_path` is a *template*, `template <int P> float run_path(float x, int k)`.
A template is a recipe the compiler stamps out once per value of P you use.
`run_path<7>` and `run_path<12>` are two distinct compiled functions with
their own constants baked in. The `if constexpr (P % 2 == 0)` inside is a
*compile-time* if: for a given P only one of the two orderings survives into
the compiled code, with zero runtime cost. Because P must be known at
compile time but `threadIdx.x % d` is a runtime value, `dispatch_path()`
bridges the two with a `switch` over all 32 cases, generated by a small
macro (`HC_CASE(P)` expands to `case P: return run_path<P>(x, k);`).

Sanity check to run on Colab if the measured d=32 slowdown is suspiciously
small: `nvcc --ptxas-options=-v` and check the generated code, per spec 3.3.

### 2.5 The `looplen` variant

Second way to create divergence: all threads run the *same* code, but with
data-dependent trip counts. Lane residue r gets `k_r = 2k(1+r)/(d+1)`
iterations. That formula is chosen so the *sum* over a warp is again 32·k,
i.e. exactly the same total work as the uniform kernel (you can verify: each
residue occurs 32/d times, and the arithmetic series sums to 32k).

Expected result, and why it differs from the branch variant: a warp is only
finished when its *slowest* lane is finished, so the warp time follows
`max(k_r) ≈ 2k`, a saturating slowdown of about 2 instead of d. Same
mechanism (lanes waiting for each other), different signature. The `& ~3`
rounds the trip count down to a multiple of 4 because the loop is unrolled
in groups of four FMAs.

### 2.6 The host code (main, flags, sweeps)

`parse_args` is a deliberately primitive command-line parser (the spec bans
libraries): it walks the argument array, `strcmp`s each entry against the
known flags, and converts the following string to a number with `atoi`. All
three binaries follow this pattern and print a usage text on `--help`.

`gflops_of` computes `2·n·k / seconds / 10^9`. The `slowdown_vs_d1` column
is filled by remembering the d=1 time of the current mode and dividing.

---

## 3. `src/cpu_baseline.cpp`: the same workload on the CPU

This file exists so the report can *measure*, not merely claim, that a CPU
behaves differently: flat performance over n (Experiment A) and near-zero
divergence penalty (Experiment B). It contains a byte-for-byte copy of the
32-path code from the CUDA file (with `__attribute__((noinline))`, the GCC
spelling of noinline). The duplication is deliberate: the spec fixes the
file list, and a shared header between CUDA and plain C++ would have needed
an extra file. A comment in both files says "keep in sync".

New concepts relative to the CUDA file:

- **OpenMP** parallelizes the element loop across CPU cores with a single
  line: `#pragma omp parallel for num_threads(threads)`. The compiler
  splits the iterations of the following `for` loop among threads. Running
  with `threads = 1` gives the single-core baseline (`cpu1`), with all
  cores the multicore baseline (`cpuN`).
- **`std::chrono::steady_clock`** replaces CUDA events; CPU code is
  synchronous, so an ordinary monotonic wall clock is correct. Same
  convention: 3 warmups, median of 10.
- **The `consume()` trick.** `volatile float sink = b[n/2];` reads one
  result through a `volatile` variable, which the compiler must not
  optimize away. This anchors the whole computation against dead-code
  elimination, the CPU equivalent of "write the result to global memory".

Two pragmatic deviations from the GPU sweeps, both documented in the report:

- The n sweep stops at 2^20 (GPU: 2^26) and the divergence run uses 2^18
  (GPU: 2^24). Reason: at 2^26 a single Colab CPU point would take minutes
  (13 runs × ~70 s), and CPU GFLOP/s is flat over n anyway, which is the
  very thing being demonstrated. The CSV contains an `n` column, so the
  difference is transparent, and GFLOP/s and slowdown are size-independent
  metrics.
- The scaling CSV has a `block` column (from the GPU schema); CPU rows
  store the thread count there instead of leaving a hole.

The `--no-header` flag exists purely for `run_all.sh`: when CPU rows are
*appended* to the GPU CSV, a second header line in the middle of the file
would confuse pandas. The provenance comment still gets printed because `#`
lines are skipped anyway.

---

## 4. `src/task2_bandwidth.cu`: the memory benchmark

Task 2 flips the regime: minimal arithmetic, maximal memory traffic. The
kernel is `b[j] = a[j] * c`, one multiply per two 4-byte memory operations.
The metric is effective bandwidth: `8 bytes × n / time` (4 read + 4
written per element).

### 4.1 The three access patterns (Experiment C)

**Coalesced** (`j = i`): thread i touches element i. The 32 threads of a
warp touch 32 *consecutive* floats = one contiguous 128-byte region, which
the hardware fetches as a single transaction. This is the best case and its
measured result is used as the "practical peak" that all other numbers are
expressed against (in `pct_of_peak`).

**Strided**: thread i touches `j = (i mod chunk) · stride + i / chunk`,
with `chunk = n/stride`. Where does this formula come from? The naive
`j = i · stride` runs out of the array. The naive `j = (i · stride) mod n`
stays inside but visits elements *multiple times* while skipping others
(and overflows 32-bit arithmetic for large i·stride). The formula used is
the spec's suggested alternative and is a clean *permutation*: think of the
array as a matrix with `stride` columns stored row-by-row; the formula
makes consecutive threads walk down a column. Consecutive threads are
`stride` elements apart in memory (the divergence you want to measure), yet
every element is touched exactly once. Example with n = 8, stride = 4,
chunk = 2: threads 0..7 touch j = 0, 4, 1, 5, 2, 6, 3, 7. Exactly once
each. A warp now touches 32 elements spread over `stride`-sized gaps, so
the hardware needs up to 32 separate memory transactions where the
coalesced kernel needed one; most of each fetched cache line is thrown
away, and effective bandwidth collapses.

**Random gather** (`b[i] = a[idx[i]] * c`): the read address comes from an
index array `idx`, which holds a random *permutation* of 0..n-1 (so again
every element exactly once). The permutation is built on the host with a
fixed seed (42, so runs are reproducible), shuffled by `std::shuffle`, and
copied to the GPU once, *outside* the timed region. Byte-accounting
footnote (also in the report): the CSV counts 8 B/element of payload; the
4 B/element read of `idx` itself is physically on top, so the true traffic
is 1.5× the reported number.

**Validation.** A wrong-but-fast kernel is worthless, so every
configuration is checksummed: since all three patterns compute
`b = a·c` element-wise, just permuted, the *sum* of b must equal
`sum(a) · c` no matter the order. The host sums b in double precision and
aborts on a relative error above 10^-5. The input values are multiples of
1/1024 in [0,1), chosen so `a[i] · 1.5` is exactly representable in float
and the checksum is tight, not fuzzy.

The helper `fit_n()` asks the driver how much free memory the GPU has
(`cudaMemGetInfo`) and halves n until the three arrays fit into 80% of it,
per the spec's "check free memory and halve if needed".

### 4.2 Experiment D: occupancy and latency hiding

Question: how many resident warps does an SM need before memory latency is
fully hidden? Two knobs vary the number of resident warps; the y-axis is
bandwidth of the *same* coalesced workload.

The kernel here is a **grid-stride loop**:

```c
for (int i = blockIdx.x * blockDim.x + threadIdx.x; i < n;
     i += gridDim.x * blockDim.x)
  b[i] = a[i] * c;
```

Instead of "one thread = one element", each thread processes every
(grid·block)-th element. Why: the experiment launches *deliberately small*
grids (exactly the blocks that fit on the SMs at the chosen occupancy), and
the loop guarantees the total work is always the same n elements regardless
of how many threads exist. Otherwise configurations would not be comparable.

**Knob 1, block size** {32...1024}: the runtime asks
`cudaOccupancyMaxActiveBlocksPerMultiprocessor` how many blocks of this
size fit on one SM, launches exactly `that × number of SMs` blocks, and
reports the resulting occupancy `= active warps / max warps per SM`.

**Knob 2, shared-memory throttle** at fixed block 256: a block that
*requests* dynamic shared memory occupies that much of the SM's shared
memory budget, even if the kernel never touches it. Requesting
`smem_per_SM / m` bytes per block therefore caps residency at m blocks per
SM. Sweeping m = 1, 2, 3, ... walks occupancy from low to 100% without
changing the code at all; the third launch parameter
(`<<<grid, block, smem>>>`) is the requested byte count. Two wrinkles:
requesting more than 48 KB requires an explicit opt-in
(`cudaFuncSetAttribute(..., MaxDynamicSharedMemorySize, ...)`), and the
actual achieved residency is always re-queried from the occupancy API and
deduplicated, rather than trusting the arithmetic, so the code works on any
GPU (spec rule: query everything at runtime, hardcode nothing).

On a T4 with block 256 the throttle can only produce 25/50/75/100%
occupancy (a block is 8 warps, the SM holds at most 32), which is why the
spec's "~12.5%" endpoint is not reachable with this block size; the
block-size series adds finer points. The report names this granularity.

Why *theoretical* occupancy: the profiler metric would need Nsight Compute
counter permissions, which Colab normally denies. The notebook contains an
optional `ncu` cell to try anyway; the report's Limitations section covers
the fallback.

---

## 5. `CMakeLists.txt`: the build recipe

CMake is a build-system generator: you declare *what* to build and it works
out the compiler commands. Reading it top to bottom:

- `cmake_minimum_required(VERSION 3.18)`: oldest CMake version that
  understands this file.
- The `CMAKE_CUDA_ARCHITECTURES` block answers "for which GPU generation
  should nvcc generate machine code?". It must be decided *before*
  `project()` enables the CUDA language. If CMake is new enough (≥ 3.24),
  `native` means "whatever GPU is in this machine", perfect for Colab. On
  older CMake, it falls back to compiling for every major architecture ≥
  7.0 (a "fat binary" that runs anywhere the spec targets). Note this is
  about *instruction sets*, not the forbidden kind of hardcoding (SM
  counts, sizes), which are all queried at runtime.
- `project(hc02 LANGUAGES CXX CUDA)`: this project contains C++ and CUDA.
- The `CMAKE_..._STANDARD` lines pin C++17 (needed for `if constexpr`).
- Three `add_executable` calls declare the three binaries and their source
  files. The `$<$<COMPILE_LANGUAGE:CUDA>:-Xcompiler=-Wall,-Wextra>`
  generator expression means: when compiling CUDA files, pass
  `-Wall -Wextra` (all warnings) through nvcc to the host compiler, which
  satisfies the acceptance-checklist item "no warnings on host code".
- `find_package(OpenMP REQUIRED)` locates OpenMP and
  `target_link_libraries(... OpenMP::OpenMP_CXX)` attaches the right flags
  to `cpu_baseline`. `-march=native` lets GCC use the host CPU's FMA
  instructions, so the CPU baseline is not unfairly slow.
- `CMAKE_BUILD_TYPE Release` turns on `-O3` optimization by default.

The README carries raw `nvcc`/`g++` command lines as a fallback in case
CMake misbehaves on Colab.

---

## 6. `scripts/run_all.sh`: the orchestration script

A bash script is a list of terminal commands executed top to bottom. The
non-obvious lines:

- `#!/usr/bin/env bash` (the "shebang"): tells the OS which interpreter
  runs this file.
- `set -euo pipefail`: safety switches. `-e` aborts the script on the first
  failing command (otherwise a crashed benchmark would silently produce an
  empty CSV and the script would carry on), `-u` treats using an undefined
  variable as an error, `pipefail` makes failures inside pipelines visible.
- `cd "$(dirname "$0")/.."`: `$0` is the path of the script itself,
  `dirname` strips the file name, so this means "go to the folder above
  `scripts/`", i.e. `blatt02/`. Result: the script works no matter which
  directory you call it from.
- `BIN="${BIN:-build}"`: use the environment variable `BIN` if set,
  otherwise default to `build`. Lets you point at a differently named build
  folder without editing the script.
- `>` redirects a program's stdout into a file (creating/overwriting it);
  `>>` appends. Hence: GPU binary `>` creates the CSV with the header, CPU
  binary `--no-header >>` appends its rows below.

The whole thing is idempotent: run it twice and it simply overwrites
`results/` with fresh numbers.

---

## 7. `scripts/plot.py`: Python from zero

Since you have never used Python, this section is more granular. The big
differences from C++ first: Python is not compiled, it executes top to
bottom like a script; variables have no declared types; **indentation is
the syntax** for blocks (there are no braces; the code inside a `for` is
whatever is indented under it); and libraries are pulled in with `import`.

### 7.1 The imports and setup

```python
import os, sys
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import pandas as pd
```

- `os` and `sys` are standard built-ins (file paths, command-line access).
- **matplotlib** is *the* Python plotting library. `matplotlib.use("Agg")`
  selects the "just render to image files" backend before anything else, so
  the script works on Colab where no display window exists.
- `import matplotlib.pyplot as plt` imports the plotting interface under
  the short alias `plt`; `import pandas as pd` likewise. These two aliases
  are universal Python convention.
- **pandas** is a table-manipulation library. Its central object is the
  `DataFrame`: a table with named columns, like a spreadsheet in a variable.

```python
RES = sys.argv[1] if len(sys.argv) > 1 else os.path.join(...)
```

`sys.argv` is the list of command-line arguments (`argv[0]` is the script
name). This line reads: "if the user passed an argument, use it as the
results folder, otherwise default to `../results` relative to the script".
That one-liner form (`A if condition else B`) is Python's ternary
expression. The default path is built with `os.path.join`, which glues path
segments with the correct separator, and
`os.path.dirname(os.path.abspath(__file__))`, which is "the folder this
script lives in", the same trick as the `dirname "$0"` in bash.
`os.makedirs(PLOTS, exist_ok=True)` creates the plots folder;
`exist_ok=True` means "no error if it already exists".

### 7.2 The helper functions

```python
def load(name):
    return pd.read_csv(os.path.join(RES, name), comment="#")
```

`def` defines a function. `pd.read_csv` reads a CSV file into a DataFrame,
using the first line as column names. `comment="#"` tells it to ignore
lines starting with `#`, which is exactly why the provenance lines (and the
CPU comment line in the middle of the appended file) are harmless.

`provenance(name, key)` opens a CSV, reads only its first line, and
extracts a `key=value` pair from it (used to pull
`peak_theoretical_gbps=320.1` out of the comment). Notable syntax:
`line.split(", ")` chops a string into a list at each ", ";
`part.startswith(key + "=")` is a string test; `try/except` is Python's
exception handling, here used to return `None` (Python's "nothing" value)
when the value is not a number, e.g. "n/a".

`save(fig, name)` finishes every plot the same way: `tight_layout()`
auto-adjusts margins so labels are not cut off, `savefig(..., dpi=150)`
writes the PNG, `plt.close(fig)` frees the figure's memory, and `print`
logs the file name to the terminal. (`print(f"wrote plots/{name}")` is an
*f-string*: inside a string prefixed with `f`, any `{expression}` is
replaced by its value. Python's printf.)

### 7.3 One plot block in slow motion

All four plots follow the same shape; here is the first one dissected:

```python
df = load("task1_scaling.csv")
fig, ax = plt.subplots(figsize=(7, 4.5))
```

`df` (conventional name for a DataFrame) now holds the CSV as a table.
`plt.subplots` creates one figure (`fig`, the canvas/file) containing one
axes (`ax`, the actual coordinate system you draw into); `figsize` is in
inches. The `fig, ax = ...` form unpacks the two returned values into two
variables in one line (Python functions can return multiple values).

```python
for dev, label, style in [("gpu", "GPU", "o-"), ("cpu1", "CPU, 1 thread", "s-"), ...]:
```

This iterates over a list of 3-element *tuples*, unpacking each into three
loop variables. So the loop body runs three times, once per curve, with
`dev`, `label`, `style` set accordingly. The style strings are matplotlib
shorthand: `"o-"` means circle markers joined by a solid line, `"s-"`
square markers, `"^-"` triangles; in other plots `"--"` is a dashed line
and `":"` dotted.

```python
sub = df[df.device == dev].sort_values("n")
```

The single most Python-ish line in the file. `df.device == dev` compares
the whole `device` column against one value at once and yields a column of
True/False. Indexing the DataFrame with that boolean column, `df[...]`,
keeps only the True rows. So this reads: "the sub-table of rows whose
device equals `dev`, sorted by n". This filter-by-mask pattern is used for
every curve in the script. `if not sub.empty:` skips curves with no data
instead of crashing.

```python
ax.plot(sub.n, sub.gflops, style, label=label)
```

Draws one curve: x-values from column `n`, y-values from column `gflops`.
The `label` is what later shows up in the legend.

```python
gpu_peak = gpu.gflops.max()
sat_n = int(gpu[gpu.gflops >= 0.95 * gpu_peak].n.min())
```

Finding the saturation point: take the GPU rows, keep those within 95% of
the best GFLOP/s value, and take the *smallest* n among them. That is "the
first n at which the GPU is essentially warmed up". `ax.axvline` then draws
a vertical marker line there and `ax.annotate` places a text label next to
it (`sat_n.bit_length() - 1` computes the exponent, i.e. log2 of n, for the
"n = 2^22" label text).

```python
ax.set_xscale("log", base=2)
ax.set_yscale("log")
```

Log axes: the x sweep is 2^10...2^26, so a linear axis would squash
everything except the last point; base 2 makes the ticks land on the
powers of two. The y-axis is log because GPU and CPU differ by a factor of
about 1000 and both should be readable. The remaining lines set axis
labels, title, a light background grid, and the legend, then call `save`.

The other three blocks repeat this pattern with small twists: plot 2 draws
`1.0 / sub.slowdown_vs_d1` (converting slowdown into relative throughput)
plus the ideal 1/d reference as a dotted line built with a *list
comprehension*, `[1.0 / d for d in ds]`, which is Python's inline "build a
list by transforming another list". Plot 3 uses `ax.axhline` (horizontal
reference lines) for the coalesced, random, and theoretical-peak levels.
Plot 4 annotates each measured point with its block size or shared-memory
amount, alternating the label offsets (`i % 3`) so labels at the same
occupancy do not overprint each other.

### 7.4 The summary block

The final block prints exactly the numbers the report template needs
(saturation point, peaks, d=32 slowdowns, % of peak, occupancy range), so
that filling `REPORT.md` is a copy-paste job instead of CSV archaeology.
`float(row.slowdown_vs_d1.iloc[0])` shows one more pandas idiom: filtering
can return a one-row table, `.iloc[0]` takes the first row's value by
position.

---

## 8. `colab/run_colab.ipynb`: the notebook

A Jupyter/Colab notebook is a sequence of *cells*, each either text
(Markdown) or code, executed one by one; the file itself is JSON. Cells run
Python by default, but two escape hatches matter here:

- A line starting with `!` runs a *shell command* instead
  (`!nvidia-smi`, `!cmake ...`). Python variables can be spliced in with
  `{braces}`: `!git clone --branch {BRANCH} {REPO_URL}`.
- A line starting with `%` is a notebook "magic". `%cd` changes the working
  directory *persistently* across cells (a plain `!cd` would only affect
  its own subprocess and be forgotten immediately).

The cells in order: (1) run `nvidia-smi` and `assert` a GPU is present, so
you fail fast if you forgot to select the T4 runtime; (2) parameterized
clone (edit `REPO_URL`/`BRANCH` at the top) and `%cd` into `blatt02`,
skipping the clone if the folder already exists so re-running the notebook
is harmless; (3) the CMake build; (3b) the raw-nvcc fallback, wrapped in
`if False:` so it is skipped unless you flip it to `True`; (4)
`run_all.sh`; (5) `plot.py`; (6) zip `results/` and trigger a browser
download via Colab's `files.download`. The last Markdown cell holds the two
optional verification commands (ptxas info, Nsight Compute attempt).

---

## 9. Cross-cutting decisions, in one place

- **Median of 10 with 3 warmups, everywhere.** One convention across CPU
  and GPU makes every number in the report comparable and is stated once in
  the methodology section.
- **No fast math.** `-use_fast_math` lets the compiler fuse, reorder, and
  approximate, which would silently change how many FLOPs actually execute.
  Honest counting beats bigger numbers.
- **Query, never hardcode.** SM count, warps per SM, shared memory sizes,
  peak bandwidth, free memory: all read from the driver at runtime, so the
  binaries run unchanged on any ≥ cc 7.0 GPU (spec requirement).
- **Equal work under divergence.** The one design rule that makes
  Experiment B meaningful; see 2.4.
- **Validation built in.** Task 2 checksums every pattern against a
  permutation-invariant reference; Task 1 anchors results against
  dead-code elimination. A benchmark that might be computing nothing
  measures nothing.
- **Reproducibility.** Fixed RNG seed (42), provenance comment in every
  CSV, all committed artifacts regenerable by two commands.
- **What is still open.** `results/` is empty until the Colab run; the
  `[TODO]` markers in `REPORT.md` map one-to-one onto the summary lines
  `plot.py` prints. The `.cu` files have never been compiled (no NVIDIA GPU
  here), so treat the first Colab build as a smoke test.
