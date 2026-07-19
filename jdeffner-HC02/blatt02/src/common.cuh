#pragma once
// Shared helpers for the HC02 CUDA binaries: error checking, event-based
// kernel timing (3 warmup + 10 timed launches, median), device info dump.
// Convention: CSV goes to stdout, everything human-readable to stderr.

#include <cuda_runtime.h>

#include <algorithm>
#include <cstdio>
#include <cstdlib>

#define CUDA_CHECK(call)                                                     \
  do {                                                                       \
    cudaError_t err_ = (call);                                               \
    if (err_ != cudaSuccess) {                                               \
      std::fprintf(stderr, "CUDA error at %s:%d: %s (%s)\n", __FILE__,       \
                   __LINE__, cudaGetErrorString(err_), cudaGetErrorName(err_)); \
      std::exit(EXIT_FAILURE);                                               \
    }                                                                        \
  } while (0)

constexpr int kWarmupLaunches = 3;
constexpr int kTimedLaunches = 10;

// fn() must enqueue the kernel on the default stream. Only the kernel is
// inside the timed region (no H2D/D2H). Returns the median time in ms.
template <typename F>
float time_median_ms(F&& fn) {
  cudaEvent_t start, stop;
  CUDA_CHECK(cudaEventCreate(&start));
  CUDA_CHECK(cudaEventCreate(&stop));
  for (int i = 0; i < kWarmupLaunches; ++i) fn();
  CUDA_CHECK(cudaGetLastError());
  CUDA_CHECK(cudaDeviceSynchronize());
  float t[kTimedLaunches];
  for (int i = 0; i < kTimedLaunches; ++i) {
    CUDA_CHECK(cudaEventRecord(start));
    fn();
    CUDA_CHECK(cudaEventRecord(stop));
    CUDA_CHECK(cudaEventSynchronize(stop));
    CUDA_CHECK(cudaEventElapsedTime(&t[i], start, stop));
  }
  CUDA_CHECK(cudaEventDestroy(start));
  CUDA_CHECK(cudaEventDestroy(stop));
  std::sort(t, t + kTimedLaunches);
  return 0.5f * (t[kTimedLaunches / 2 - 1] + t[kTimedLaunches / 2]);
}

struct DeviceInfo {
  cudaDeviceProp prop{};
  int driver = 0;
  int runtime = 0;
  int mem_clock_khz = -1;   // -1 means: attribute not available
  int bus_width_bits = -1;
  double peak_gbps = -1.0;  // theoretical: 2 * mem clock * bus width bytes
};

inline DeviceInfo query_device_info() {
  DeviceInfo d;
  int dev = 0;
  CUDA_CHECK(cudaGetDevice(&dev));
  CUDA_CHECK(cudaGetDeviceProperties(&d.prop, dev));
  CUDA_CHECK(cudaDriverGetVersion(&d.driver));
  CUDA_CHECK(cudaRuntimeGetVersion(&d.runtime));
  // These attributes may be unavailable on newer toolkits; report "n/a"
  // instead of aborting.
  int v = 0;
  if (cudaDeviceGetAttribute(&v, cudaDevAttrMemoryClockRate, dev) == cudaSuccess)
    d.mem_clock_khz = v;
  if (cudaDeviceGetAttribute(&v, cudaDevAttrMemoryBusWidth, dev) == cudaSuccess)
    d.bus_width_bits = v;
  cudaGetLastError();  // clear sticky error from unavailable attributes
  if (d.mem_clock_khz > 0 && d.bus_width_bits > 0)
    d.peak_gbps = 2.0 * (d.mem_clock_khz * 1e3) * (d.bus_width_bits / 8.0) / 1e9;
  return d;
}

// Human-readable device dump to stderr. Replaces deviceQuery.
inline void print_device_info(const DeviceInfo& d) {
  const cudaDeviceProp& p = d.prop;
  std::fprintf(stderr, "device            : %s (cc %d.%d)\n", p.name, p.major, p.minor);
  std::fprintf(stderr, "SMs               : %d\n", p.multiProcessorCount);
  std::fprintf(stderr, "max threads/SM    : %d\n", p.maxThreadsPerMultiProcessor);
  std::fprintf(stderr, "warp size         : %d\n", p.warpSize);
  std::fprintf(stderr, "global memory     : %.1f GiB\n",
               static_cast<double>(p.totalGlobalMem) / (1024.0 * 1024.0 * 1024.0));
  std::fprintf(stderr, "L2 cache          : %d KiB\n", p.l2CacheSize / 1024);
  if (d.mem_clock_khz > 0)
    std::fprintf(stderr, "memory clock      : %.0f MHz\n", d.mem_clock_khz / 1e3);
  else
    std::fprintf(stderr, "memory clock      : n/a\n");
  if (d.bus_width_bits > 0)
    std::fprintf(stderr, "memory bus width  : %d bit\n", d.bus_width_bits);
  else
    std::fprintf(stderr, "memory bus width  : n/a\n");
  if (d.peak_gbps > 0)
    std::fprintf(stderr, "theoretical peak  : %.1f GB/s\n", d.peak_gbps);
  else
    std::fprintf(stderr, "theoretical peak  : n/a\n");
}

// Leading provenance comment for every CSV, e.g.
// # gpu=Tesla T4, cc=7.5, driver=12.4, toolkit=12.4
inline void print_csv_provenance(const DeviceInfo& d, const char* extra = "") {
  std::printf("# gpu=%s, cc=%d.%d, driver=%d.%d, toolkit=%d.%d%s\n",
              d.prop.name, d.prop.major, d.prop.minor,
              d.driver / 1000, (d.driver % 1000) / 10,
              d.runtime / 1000, (d.runtime % 1000) / 10, extra);
}
