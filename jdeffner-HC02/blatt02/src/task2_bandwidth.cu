// Task 2: memory access patterns (experiment C) and latency hiding via
// occupancy (experiment D). Memory-bound streaming kernel b[j] = a[j] * c.
// Byte convention: 8 bytes per element (4 read + 4 write); the random
// gather's 4 B/element index traffic is NOT included (see REPORT.md).

#include "common.cuh"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <numeric>
#include <random>
#include <vector>

__global__ void kernel_coalesced(const float* a, float* b, int n, float c) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i < n) b[i] = a[i] * c;
}

// j enumerates a permutation of [0, n) when n = chunk * stride, so every
// element is read and written exactly once.
__global__ void kernel_strided(const float* a, float* b, int n, int stride,
                               int chunk, float c) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i >= n) return;
  int j = (i % chunk) * stride + i / chunk;
  b[j] = a[j] * c;
}

__global__ void kernel_gather(const float* a, const int* idx, float* b, int n,
                              float c) {
  int i = blockIdx.x * blockDim.x + threadIdx.x;
  if (i < n) b[i] = a[idx[i]] * c;
}

// Grid-stride version for the occupancy experiment: total work is n
// elements regardless of grid/block configuration.
__global__ void kernel_gridstride(const float* a, float* b, int n, float c) {
  for (int i = blockIdx.x * blockDim.x + threadIdx.x; i < n;
       i += gridDim.x * blockDim.x)
    b[i] = a[i] * c;
}

namespace {

constexpr float kScale = 1.5f;
constexpr double kBytesPerElem = 8.0;  // 4 B read + 4 B write

void usage(const char* argv0) {
  std::fprintf(stderr,
               "usage: %s [--exp patterns|occupancy] [--n N] [--block B]\n"
               "  --exp patterns   experiment C: coalesced / strided / random gather (default)\n"
               "  --exp occupancy  experiment D: block size sweep + shared memory throttle\n"
               "  --n N            elements per array, power of two (default 2^26, halved\n"
               "                   automatically if device memory is short)\n"
               "  --block B        threads per block for experiment C / the smem throttle\n"
               "                   (default 256)\n"
               "CSV on stdout, device info on stderr.\n",
               argv0);
}

struct Args {
  const char* exp = "patterns";
  int n = 1 << 26;
  int block = 256;
};

Args parse_args(int argc, char** argv) {
  Args a;
  for (int i = 1; i < argc; ++i) {
    if (!std::strcmp(argv[i], "--help")) { usage(argv[0]); std::exit(0); }
    else if (!std::strcmp(argv[i], "--exp") && i + 1 < argc) a.exp = argv[++i];
    else if (!std::strcmp(argv[i], "--n") && i + 1 < argc) a.n = std::atoi(argv[++i]);
    else if (!std::strcmp(argv[i], "--block") && i + 1 < argc) a.block = std::atoi(argv[++i]);
    else { usage(argv[0]); std::exit(1); }
  }
  if (a.n <= 0 || (a.n & (a.n - 1)) != 0) { std::fprintf(stderr, "--n must be a power of two\n"); std::exit(1); }
  if (a.block <= 0 || a.block > 1024 || a.block % 32 != 0) { std::fprintf(stderr, "--block must be a multiple of 32 up to 1024\n"); std::exit(1); }
  return a;
}

double gbps_of(int n, float t_ms) {
  return kBytesPerElem * n / (t_ms * 1e-3) / 1e9;
}

// Checksum validation: sum(b) must equal sum(a) * c (permutation-invariant).
void validate(const float* db, int n, double expected, const char* what) {
  static std::vector<float> hb;
  hb.resize(n);
  CUDA_CHECK(cudaMemcpy(hb.data(), db, sizeof(float) * n, cudaMemcpyDeviceToHost));
  double sum = std::accumulate(hb.begin(), hb.end(), 0.0);
  double rel = std::abs(sum - expected) / std::abs(expected);
  if (rel > 1e-5) {
    std::fprintf(stderr, "VALIDATION FAILED (%s): sum=%.9g expected=%.9g rel=%.3g\n",
                 what, sum, expected, rel);
    std::exit(EXIT_FAILURE);
  }
  std::fprintf(stderr, "validated %-10s rel_err=%.2e\n", what, rel);
}

// Shrink n until a, b and idx fit comfortably into free device memory.
int fit_n(int n) {
  size_t free_b = 0, total_b = 0;
  CUDA_CHECK(cudaMemGetInfo(&free_b, &total_b));
  while (n > (1 << 20) && 12.0 * n > 0.8 * static_cast<double>(free_b)) n >>= 1;
  return n;
}

struct Buffers {
  float* da = nullptr;
  float* db = nullptr;
  double sum_a = 0.0;  // host-side reference sum
};

Buffers make_buffers(int n) {
  Buffers buf;
  CUDA_CHECK(cudaMalloc(&buf.da, sizeof(float) * n));
  CUDA_CHECK(cudaMalloc(&buf.db, sizeof(float) * n));
  std::vector<float> ha(n);
  for (int i = 0; i < n; ++i) {
    ha[i] = static_cast<float>(i & 1023) * 9.765625e-4f;  // in [0, 1)
    buf.sum_a += ha[i];
  }
  CUDA_CHECK(cudaMemcpy(buf.da, ha.data(), sizeof(float) * n, cudaMemcpyHostToDevice));
  return buf;
}

void run_patterns(const Args& args, const DeviceInfo& info) {
  const int n = fit_n(args.n);
  const int grid = (n + args.block - 1) / args.block;
  Buffers buf = make_buffers(n);
  const double expected = buf.sum_a * kScale;

  char extra[64];
  if (info.peak_gbps > 0)
    std::snprintf(extra, sizeof extra, ", peak_theoretical_gbps=%.1f", info.peak_gbps);
  else
    std::snprintf(extra, sizeof extra, ", peak_theoretical_gbps=n/a");
  print_csv_provenance(info, extra);
  std::printf("pattern,stride,n,time_ms,gbps,pct_of_peak\n");

  // Coalesced first: its result is the measured practical peak.
  float t = time_median_ms([&] {
    kernel_coalesced<<<grid, args.block>>>(buf.da, buf.db, n, kScale);
  });
  validate(buf.db, n, expected, "coalesced");
  const double peak_practical = gbps_of(n, t);
  std::printf("coalesced,1,%d,%.5f,%.2f,%.1f\n", n, t, peak_practical, 100.0);

  for (int stride : {2, 4, 8, 16, 32, 64, 128}) {
    const int chunk = n / stride;
    t = time_median_ms([&] {
      kernel_strided<<<grid, args.block>>>(buf.da, buf.db, n, stride, chunk, kScale);
    });
    validate(buf.db, n, expected, "strided");
    double g = gbps_of(n, t);
    std::printf("strided,%d,%d,%.5f,%.2f,%.1f\n", stride, n, t, g,
                100.0 * g / peak_practical);
  }

  // Random gather: uniform permutation, fixed seed, built on the host and
  // copied once outside the timed region.
  std::vector<int> hidx(n);
  std::iota(hidx.begin(), hidx.end(), 0);
  std::mt19937 rng(42);
  std::shuffle(hidx.begin(), hidx.end(), rng);
  int* didx = nullptr;
  CUDA_CHECK(cudaMalloc(&didx, sizeof(int) * n));
  CUDA_CHECK(cudaMemcpy(didx, hidx.data(), sizeof(int) * n, cudaMemcpyHostToDevice));
  t = time_median_ms([&] {
    kernel_gather<<<grid, args.block>>>(buf.da, didx, buf.db, n, kScale);
  });
  validate(buf.db, n, expected, "random");
  double g = gbps_of(n, t);
  std::printf("random,0,%d,%.5f,%.2f,%.1f\n", n, t, g, 100.0 * g / peak_practical);

  CUDA_CHECK(cudaFree(didx));
  CUDA_CHECK(cudaFree(buf.da));
  CUDA_CHECK(cudaFree(buf.db));
}

void occupancy_row(const char* knob, int block, size_t smem, int active_blocks,
                   int max_warps_per_sm, int n, float t) {
  const int warps = active_blocks * block / 32;
  std::printf("%s,%d,%zu,%d,%.3f,%.5f,%.2f\n", knob, block, smem, warps,
              static_cast<double>(warps) / max_warps_per_sm, t, gbps_of(n, t));
}

void run_occupancy(const Args& args, const DeviceInfo& info) {
  const int n = fit_n(args.n);
  const int sms = info.prop.multiProcessorCount;
  const int max_warps_per_sm = info.prop.maxThreadsPerMultiProcessor / 32;
  Buffers buf = make_buffers(n);

  print_csv_provenance(info);
  std::printf("knob,block,smem_bytes,active_warps_per_sm,occupancy,time_ms,gbps\n");

  // Knob 1: block size sweep, no shared memory. Grid covers exactly the
  // resident blocks; the grid-stride loop keeps total work identical.
  for (int block : {32, 64, 128, 256, 512, 1024}) {
    int active = 0;
    CUDA_CHECK(cudaOccupancyMaxActiveBlocksPerMultiprocessor(
        &active, kernel_gridstride, block, 0));
    float t = time_median_ms([&] {
      kernel_gridstride<<<active * sms, block>>>(buf.da, buf.db, n, kScale);
    });
    occupancy_row("block", block, 0, active, max_warps_per_sm, n, t);
  }

  // Knob 2: fixed block size, unused dynamic shared memory per block caps
  // the number of resident blocks per SM. Run at half the block size too:
  // smaller blocks reach lower occupancy floors (block 128 gets to 12.5%
  // on a T4, where block 256 bottoms out at 25%).
  int dev = 0;
  CUDA_CHECK(cudaGetDevice(&dev));
  int smem_per_sm = 0, smem_optin = 0;
  CUDA_CHECK(cudaDeviceGetAttribute(&smem_per_sm,
                                    cudaDevAttrMaxSharedMemoryPerMultiprocessor, dev));
  CUDA_CHECK(cudaDeviceGetAttribute(&smem_optin,
                                    cudaDevAttrMaxSharedMemoryPerBlockOptin, dev));
  CUDA_CHECK(cudaFuncSetAttribute(kernel_gridstride,
                                  cudaFuncAttributeMaxDynamicSharedMemorySize,
                                  smem_optin));
  for (int tblock : {args.block / 2, args.block}) {
    int max_blocks = 0;
    CUDA_CHECK(cudaOccupancyMaxActiveBlocksPerMultiprocessor(
        &max_blocks, kernel_gridstride, tblock, 0));
    int prev_active = -1;
    for (int m = 1; m <= max_blocks; ++m) {
      size_t smem = static_cast<size_t>(smem_per_sm) / m;
      if (smem > static_cast<size_t>(smem_optin)) smem = smem_optin;
      if (m == max_blocks) smem = 0;  // unthrottled reference point
      int active = 0;
      CUDA_CHECK(cudaOccupancyMaxActiveBlocksPerMultiprocessor(
          &active, kernel_gridstride, tblock, smem));
      if (active == prev_active) continue;
      prev_active = active;
      float t = time_median_ms([&] {
        kernel_gridstride<<<active * sms, tblock, smem>>>(buf.da, buf.db, n, kScale);
      });
      occupancy_row("smem", tblock, smem, active, max_warps_per_sm, n, t);
    }
  }

  // One checksum per run for the streaming kernel used here.
  validate(buf.db, n, buf.sum_a * kScale, "gridstride");
  CUDA_CHECK(cudaFree(buf.da));
  CUDA_CHECK(cudaFree(buf.db));
}

}  // namespace

int main(int argc, char** argv) {
  Args args = parse_args(argc, argv);
  DeviceInfo info = query_device_info();
  print_device_info(info);
  if (!std::strcmp(args.exp, "patterns")) {
    run_patterns(args, info);
  } else if (!std::strcmp(args.exp, "occupancy")) {
    run_occupancy(args, info);
  } else {
    usage(argv[0]);
    return 1;
  }
  return 0;
}
