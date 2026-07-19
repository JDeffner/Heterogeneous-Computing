# HC02: SIMT, warp divergence and memory bandwidth

Exercise sheet 2 for Heterogeneous Computing. Two CUDA micro-benchmarks and a
CPU baseline:

- `task1_divergence`: compute-bound FMA kernel. Experiment A measures GFLOP/s
  vs problem size (GPU vs CPU), experiment B measures the cost of controlled
  warp divergence (d = 1..32 distinct paths per warp).
- `task2_bandwidth`: memory-bound streaming kernel. Experiment C compares
  coalesced, strided and random-gather access, experiment D measures
  bandwidth vs occupancy (block size sweep and shared-memory throttle).
- `cpu_baseline`: the same FMA workload on the CPU (1 thread and OpenMP).

Results (CSVs and plots) live in `results/`; the report is `REPORT.md`.

## Build

Requires CUDA 12.x, a C++17 host compiler with OpenMP, CMake >= 3.18.

```sh
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build -j
```

Fallback without CMake:

```sh
nvcc -O3 -std=c++17 -arch=native -Xcompiler -Wall,-Wextra src/task1_divergence.cu -o build/task1_divergence
nvcc -O3 -std=c++17 -arch=native -Xcompiler -Wall,-Wextra src/task2_bandwidth.cu -o build/task2_bandwidth
g++  -O3 -std=c++17 -march=native -fopenmp -Wall -Wextra src/cpu_baseline.cpp -o build/cpu_baseline
```

No fast math is used anywhere (honest FLOP counts). Binaries run on any GPU
with compute capability >= 7.0; everything (SM count, peak bandwidth,
occupancy) is queried at runtime.

## Run

The intended way is Google Colab (T4): open `colab/run_colab.ipynb`, set the
repo URL in cell 2, Run all, download `results.zip`.

Locally (any CUDA machine):

```sh
bash scripts/run_all.sh      # all sweeps -> results/*.csv  (well under 10 min on a T4)
python scripts/plot.py       # results/plots/*.png + summary numbers
```

Each binary also runs standalone and documents its flags via `--help`; all
CSVs go to stdout (with a `# gpu=..., cc=..., ...` provenance line), device
info and validation messages go to stderr. Sanity check for the divergence
experiment (spec 3.3): the d=32 slowdown must be roughly an order of
magnitude; if not, inspect with `nvcc --ptxas-options=-v` whether the
compiler merged the branch bodies.
