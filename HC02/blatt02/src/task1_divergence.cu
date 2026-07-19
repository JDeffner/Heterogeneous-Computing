// Task 1: SIMT throughput scaling (experiment A) and controlled warp
// divergence (experiment B). Compute-bound: one float in, k FMA iterations,
// one float out. CSV to stdout, device info to stderr.

#include "common.cuh"

#include <cstring>
#include <vector>

// ---------------------------------------------------------------------------
// Arithmetic paths. Every path executes exactly one FMA (2 FLOP) per
// iteration, so all paths do identical work and any slowdown under
// divergence is pure serialization. The immediates and the instruction
// order are unique per P, and the bodies are __noinline__, so nvcc cannot
// merge the switch cases back into a single path (spec 3.3 pitfall).
// All maps are contractive with fixed points near 1, so x stays bounded.
// k must be a multiple of 4. Keep in sync with the copy in cpu_baseline.cpp.

template <int P>
__device__ __noinline__ float run_path(float x, int k) {
  const float a0 = 0.99900f - 1e-5f * P;
  const float b0 = 1.0f - a0;              // fixed point of x*a0+b0 is 1
  const float a1 = 0.99850f - 1e-5f * P;
  const float b1 = 1.0f - a1;
  const float m = -(1e-3f + 1e-5f * P);    // x = x*(1+m): mild contraction
  for (int j = 0; j < k; j += 4) {
    if constexpr (P % 2 == 0) {
      x = fmaf(x, a0, b0);
      x = fmaf(x, m, x);
      x = fmaf(x, a1, b1);
      x = fmaf(x, m, x);
    } else {
      x = fmaf(x, m, x);
      x = fmaf(x, a1, b1);
      x = fmaf(x, m, x);
      x = fmaf(x, a0, b0);
    }
  }
  return x;
}

__device__ float dispatch_path(int p, float x, int k) {
  switch (p) {
#define HC_CASE(P) case P: return run_path<P>(x, k);
    HC_CASE(0)  HC_CASE(1)  HC_CASE(2)  HC_CASE(3)
    HC_CASE(4)  HC_CASE(5)  HC_CASE(6)  HC_CASE(7)
    HC_CASE(8)  HC_CASE(9)  HC_CASE(10) HC_CASE(11)
    HC_CASE(12) HC_CASE(13) HC_CASE(14) HC_CASE(15)
    HC_CASE(16) HC_CASE(17) HC_CASE(18) HC_CASE(19)
    HC_CASE(20) HC_CASE(21) HC_CASE(22) HC_CASE(23)
    HC_CASE(24) HC_CASE(25) HC_CASE(26) HC_CASE(27)
    HC_CASE(28) HC_CASE(29) HC_CASE(30) HC_CASE(31)
#undef HC_CASE
  }
  return x;  // unreachable, p is always in [0, 32)
}

// ---------------------------------------------------------------------------
// Kernels

__global__ void kernel_uniform(const float* a, float* b, int n, int k) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i < n) b[i] = run_path<0>(a[i], k);
}

// d distinct paths per warp, selected by lane residue.
__global__ void kernel_branch(const float* a, float* b, int n, int k, int d) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i < n) b[i] = dispatch_path(threadIdx.x % d, a[i], k);
}

// Divergence via data-dependent trip count: lane residue r gets
// k_r = 2k(1+r)/(d+1) iterations; summed over a full warp that is 32k
// (up to rounding), i.e. the same total work as the uniform kernel.
__global__ void kernel_looplen(const float* a, float* b, int n, int k, int d) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i >= n) return;
  int r = threadIdx.x % d;
  int ki = ((2 * k * (1 + r)) / (d + 1)) & ~3;  // multiple of 4 for the unroll
  b[i] = run_path<0>(a[i], ki);
}

// ---------------------------------------------------------------------------

namespace {

void usage(const char* argv0) {
  std::fprintf(stderr,
               "usage: %s [--exp scaling|divergence] [--n N] [--k K] [--block B]\n"
               "  --exp scaling     experiment A: GFLOP/s vs n, no divergence (default)\n"
               "                    sweeps n = 2^10, 2^12, ..., 2^26 unless --n is given\n"
               "  --exp divergence  experiment B: d = 1,2,4,8,16,32 branch/looplen sweep\n"
               "  --n N             single problem size (default: sweep / 2^24)\n"
               "  --k K             inner iterations, multiple of 4 (default 1024)\n"
               "  --block B         threads per block (default 256)\n"
               "CSV on stdout, device info on stderr.\n",
               argv0);
}

struct Args {
  const char* exp = "scaling";
  int n = -1;
  int k = 1024;
  int block = 256;
};

Args parse_args(int argc, char** argv) {
  Args a;
  for (int i = 1; i < argc; ++i) {
    if (!std::strcmp(argv[i], "--help")) { usage(argv[0]); std::exit(0); }
    else if (!std::strcmp(argv[i], "--exp") && i + 1 < argc) a.exp = argv[++i];
    else if (!std::strcmp(argv[i], "--n") && i + 1 < argc) a.n = std::atoi(argv[++i]);
    else if (!std::strcmp(argv[i], "--k") && i + 1 < argc) a.k = std::atoi(argv[++i]);
    else if (!std::strcmp(argv[i], "--block") && i + 1 < argc) a.block = std::atoi(argv[++i]);
    else { usage(argv[0]); std::exit(1); }
  }
  if (a.k <= 0 || a.k % 4 != 0) { std::fprintf(stderr, "--k must be a positive multiple of 4\n"); std::exit(1); }
  if (a.block <= 0 || a.block > 1024 || a.block % 32 != 0) { std::fprintf(stderr, "--block must be a multiple of 32 up to 1024\n"); std::exit(1); }
  return a;
}

double gflops_of(long long n, long long k, float t_ms) {
  return 2.0 * static_cast<double>(n) * static_cast<double>(k) / (t_ms * 1e-3) / 1e9;
}

void run_scaling(const Args& args, const DeviceInfo& info) {
  const int n_max = args.n > 0 ? args.n : (1 << 26);
  const int n_min = args.n > 0 ? args.n : (1 << 10);

  float *da = nullptr, *db = nullptr;
  CUDA_CHECK(cudaMalloc(&da, sizeof(float) * n_max));
  CUDA_CHECK(cudaMalloc(&db, sizeof(float) * n_max));
  std::vector<float> ha(n_max, 1.0f);
  CUDA_CHECK(cudaMemcpy(da, ha.data(), sizeof(float) * n_max, cudaMemcpyHostToDevice));

  print_csv_provenance(info);
  std::printf("device,n,k,block,time_ms,gflops\n");
  for (int n = n_min; n <= n_max; n <<= 2) {
    const int grid = (n + args.block - 1) / args.block;
    float t = time_median_ms([&] {
      kernel_uniform<<<grid, args.block>>>(da, db, n, args.k);
    });
    std::printf("gpu,%d,%d,%d,%.5f,%.2f\n", n, args.k, args.block, t,
                gflops_of(n, args.k, t));
  }
  CUDA_CHECK(cudaFree(da));
  CUDA_CHECK(cudaFree(db));
}

void run_divergence(const Args& args, const DeviceInfo& info) {
  const int n = args.n > 0 ? args.n : (1 << 24);
  const int grid = (n + args.block - 1) / args.block;
  const int ds[] = {1, 2, 4, 8, 16, 32};

  float *da = nullptr, *db = nullptr;
  CUDA_CHECK(cudaMalloc(&da, sizeof(float) * n));
  CUDA_CHECK(cudaMalloc(&db, sizeof(float) * n));
  std::vector<float> ha(n, 1.0f);
  CUDA_CHECK(cudaMemcpy(da, ha.data(), sizeof(float) * n, cudaMemcpyHostToDevice));

  print_csv_provenance(info);
  std::printf("device,mode,d,n,k,time_ms,gflops,slowdown_vs_d1\n");
  for (const char* mode : {"branch", "looplen"}) {
    float t_d1 = 0.0f;
    for (int d : ds) {
      float t = time_median_ms([&] {
        if (!std::strcmp(mode, "branch"))
          kernel_branch<<<grid, args.block>>>(da, db, n, args.k, d);
        else
          kernel_looplen<<<grid, args.block>>>(da, db, n, args.k, d);
      });
      if (d == 1) t_d1 = t;
      std::printf("gpu,%s,%d,%d,%d,%.5f,%.2f,%.3f\n", mode, d, n, args.k, t,
                  gflops_of(n, args.k, t), t / t_d1);
    }
  }
  CUDA_CHECK(cudaFree(da));
  CUDA_CHECK(cudaFree(db));
}

}  // namespace

int main(int argc, char** argv) {
  Args args = parse_args(argc, argv);
  DeviceInfo info = query_device_info();
  print_device_info(info);
  if (!std::strcmp(args.exp, "scaling")) {
    run_scaling(args, info);
  } else if (!std::strcmp(args.exp, "divergence")) {
    run_divergence(args, info);
  } else {
    usage(argv[0]);
    return 1;
  }
  return 0;
}
