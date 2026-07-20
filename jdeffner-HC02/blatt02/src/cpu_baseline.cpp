// CPU reference for Task 1: identical FMA workload, single thread (cpu1)
// and OpenMP over all cores (cpuN). Same timing convention as the GPU
// binaries: 3 warmup runs, 10 timed runs, median. CSV to stdout.
//
// Sweep sizes are capped below the GPU sizes: GFLOP/s on the CPU is flat
// over n (that is the point of the comparison), and the full 2^26 sweep
// would take minutes per point on two Colab cores.

#include <omp.h>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

// ---------------------------------------------------------------------------
// Arithmetic paths: identical to task1_divergence.cu (keep in sync).
// One FMA (2 FLOP) per iteration in every path; noinline keeps the 32
// bodies as distinct code so the branch in dispatch_path stays real.

template <int P>
__attribute__((noinline)) float run_path(float x, int k) {
  const float a0 = 0.99900f - 1e-5f * P;
  const float b0 = 1.0f - a0;
  const float a1 = 0.99850f - 1e-5f * P;
  const float b1 = 1.0f - a1;
  const float m = -(1e-3f + 1e-5f * P);
  for (int j = 0; j < k; j += 4) {
    if constexpr (P % 2 == 0) {
      x = std::fmaf(x, a0, b0);
      x = std::fmaf(x, m, x);
      x = std::fmaf(x, a1, b1);
      x = std::fmaf(x, m, x);
    } else {
      x = std::fmaf(x, m, x);
      x = std::fmaf(x, a1, b1);
      x = std::fmaf(x, m, x);
      x = std::fmaf(x, a0, b0);
    }
  }
  return x;
}

float dispatch_path(int p, float x, int k) {
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
  return x;  // unreachable
}

// ---------------------------------------------------------------------------

namespace {

constexpr int kWarmupRuns = 3;
constexpr int kTimedRuns = 10;

template <typename F>
double time_median_ms(F&& fn) {
  using clock = std::chrono::steady_clock;
  for (int i = 0; i < kWarmupRuns; ++i) fn();
  double t[kTimedRuns];
  for (int i = 0; i < kTimedRuns; ++i) {
    auto t0 = clock::now();
    fn();
    t[i] = std::chrono::duration<double, std::milli>(clock::now() - t0).count();
  }
  std::sort(t, t + kTimedRuns);
  return 0.5 * (t[kTimedRuns / 2 - 1] + t[kTimedRuns / 2]);
}

void run_uniform(const float* a, float* b, int n, int k, int threads) {
#pragma omp parallel for num_threads(threads) schedule(static)
  for (int i = 0; i < n; ++i) b[i] = run_path<0>(a[i], k);
}

// Same %d branching as the GPU divergence kernel.
void run_branch(const float* a, float* b, int n, int k, int d, int threads) {
#pragma omp parallel for num_threads(threads) schedule(static)
  for (int i = 0; i < n; ++i) b[i] = dispatch_path(i % d, a[i], k);
}

void run_looplen(const float* a, float* b, int n, int k, int d, int threads) {
#pragma omp parallel for num_threads(threads) schedule(static)
  for (int i = 0; i < n; ++i) {
    int r = i % d;
    int ki = ((2 * k * (1 + r)) / (d + 1)) & ~3;
    b[i] = run_path<0>(a[i], ki);
  }
}

void usage(const char* argv0) {
  std::fprintf(stderr,
               "usage: %s [--exp scaling|divergence] [--n N] [--k K] [--no-header]\n"
               "  --exp scaling     GFLOP/s vs n for cpu1 and cpuN (default),\n"
               "                    sweeps n = 2^2, 2^4, ..., 2^20 unless --n is given\n"
               "  --exp divergence  d = 1,2,4,8,16,32 branch/looplen sweep (cpuN),\n"
               "                    default n = 2^18\n"
               "  --k K             inner iterations, multiple of 4 (default 1024)\n"
               "  --no-header       suppress the CSV header line (for appending)\n",
               argv0);
}

struct Args {
  const char* exp = "scaling";
  int n = -1;
  int k = 1024;
  bool header = true;
};

Args parse_args(int argc, char** argv) {
  Args a;
  for (int i = 1; i < argc; ++i) {
    if (!std::strcmp(argv[i], "--help")) { usage(argv[0]); std::exit(0); }
    else if (!std::strcmp(argv[i], "--exp") && i + 1 < argc) a.exp = argv[++i];
    else if (!std::strcmp(argv[i], "--n") && i + 1 < argc) a.n = std::atoi(argv[++i]);
    else if (!std::strcmp(argv[i], "--k") && i + 1 < argc) a.k = std::atoi(argv[++i]);
    else if (!std::strcmp(argv[i], "--no-header")) a.header = false;
    else { usage(argv[0]); std::exit(1); }
  }
  if (a.k <= 0 || a.k % 4 != 0) { std::fprintf(stderr, "--k must be a positive multiple of 4\n"); std::exit(1); }
  return a;
}

double gflops_of(long long n, long long k, double t_ms) {
  return 2.0 * static_cast<double>(n) * static_cast<double>(k) / (t_ms * 1e-3) / 1e9;
}

// Read one result so the b[] stores cannot be dead-code-eliminated.
void consume(const float* b, int n) {
  volatile float sink = b[n / 2];
  (void)sink;
}

void run_scaling(const Args& args, int max_threads) {
  // Same low starting point as the GPU sweep (2^2) so the crossover where
  // the GPU is still slower than the CPU is visible in one plot.
  const int n_max = args.n > 0 ? args.n : (1 << 20);
  const int n_min = args.n > 0 ? args.n : (1 << 2);
  std::vector<float> a(n_max, 1.0f), b(n_max, 0.0f);

  // The block column holds the thread count for CPU rows.
  if (args.header) std::printf("device,n,k,block,time_ms,gflops\n");
  for (int threads : {1, max_threads}) {
    const char* dev = (threads == 1) ? "cpu1" : "cpuN";
    for (int n = n_min; n <= n_max; n <<= 2) {
      double t = time_median_ms([&] { run_uniform(a.data(), b.data(), n, args.k, threads); });
      consume(b.data(), n);
      std::printf("%s,%d,%d,%d,%.5f,%.2f\n", dev, n, args.k, threads, t,
                  gflops_of(n, args.k, t));
    }
    if (max_threads == 1) break;  // cpu1 == cpuN, do not duplicate
  }
}

void run_divergence(const Args& args, int max_threads) {
  const int n = args.n > 0 ? args.n : (1 << 18);
  const int ds[] = {1, 2, 4, 8, 16, 32};
  std::vector<float> a(n, 1.0f), b(n, 0.0f);

  if (args.header) std::printf("device,mode,d,n,k,time_ms,gflops,slowdown_vs_d1\n");
  for (const char* mode : {"branch", "looplen"}) {
    double t_d1 = 0.0;
    for (int d : ds) {
      double t = time_median_ms([&] {
        if (!std::strcmp(mode, "branch"))
          run_branch(a.data(), b.data(), n, args.k, d, max_threads);
        else
          run_looplen(a.data(), b.data(), n, args.k, d, max_threads);
      });
      consume(b.data(), n);
      if (d == 1) t_d1 = t;
      std::printf("cpuN,%s,%d,%d,%d,%.5f,%.2f,%.3f\n", mode, d, n, args.k, t,
                  gflops_of(n, args.k, t), t / t_d1);
    }
  }
}

}  // namespace

int main(int argc, char** argv) {
  Args args = parse_args(argc, argv);
  const int max_threads = omp_get_max_threads();
  std::printf("# cpu baseline, omp_max_threads=%d\n", max_threads);
  if (!std::strcmp(args.exp, "scaling")) {
    run_scaling(args, max_threads);
  } else if (!std::strcmp(args.exp, "divergence")) {
    run_divergence(args, max_threads);
  } else {
    usage(argv[0]);
    return 1;
  }
  return 0;
}
