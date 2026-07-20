# Slide outline (10 minutes, ~8 slides)

1. **Motivation**
   - Same chip, 100x different performance depending on how you use it.
   - Two questions: what does control flow cost (Task 1), what does data
     layout cost (Task 2) on a GPU vs a CPU?

2. **The SIMT model in one picture**
   - Warp = 32 threads, one instruction stream, active mask.
   - GPU = throughput machine (many resident warps), CPU = latency machine
     (caches, prediction, out-of-order).
   - Methodology in one line: CUDA events, 3 warmups, median of 10, T4 on Colab.

3. **Experiment A: throughput vs problem size** (plot task1_scaling.png)
   - Both ends measured: below n ≈ 16 the GPU is slower than the CPU
     (fixed ~10 µs launch overhead); it saturates only from ~2^20 = 1M
     threads (~26k scheduled threads per SM, ~26 waves of full
     residency). CPU is flat over the entire sweep.
   - Read off: GPU plateau ~5,173 GFLOP/s vs CPU 3.2 GFLOP/s (2 cores),
     ~1,600x on this workload (caveats: 2-core VM, scalar CPU chain).

4. **Experiment B: controlled warp divergence** (plot task1_divergence.png)
   - d distinct paths per warp, identical work per path.
   - GPU: slowdown grows ~proportionally, 19.9x at d = 32 (about 2/3 of
     ideal: Volta+ interleaves the divergent streams). CPU: absolute
     times flat over d within VM noise.
   - Loop-length variant: saturates at 1.85x, different mechanism, same cause.

5. **Why serialization happens**
   - Pre-Volta: one PC per warp, paths executed one after another under the
     active mask. Volta+: per-thread PC, but divergent instruction streams
     still issue serially.
   - CPU: independent control flow per core + branch prediction, so the same
     branch is nearly free.

6. **Experiment C: memory access patterns** (plot task2_patterns.png)
   - Coalesced hits 243 GB/s = 76 % of theoretical peak; stride kills
     bandwidth geometrically (36 % at stride 2, 2.6 % at stride 128);
     random gather 14 GB/s = 5.9 % of peak.
   - One warp = 32/128-byte transactions; scattered lanes waste the bus.

7. **Experiment D: occupancy and latency hiding** (plot task2_occupancy.png)
   - Bandwidth rises 96 -> 156 -> ~185 GB/s from 12.5 % to 50 % occupancy,
     then plateaus: enough warps to hide DRAM latency, after that the
     DRAM itself is the limit.
   - Occupancy is a means (latency hiding), not a goal in itself.

8. **Takeaways**
   - Give the GPU enough parallel work (saturation point!).
   - Keep warps convergent; if you must branch, branch at warp granularity.
   - Layout: structure-of-arrays, coalesced access, avoid gather.
   - CPU and GPU hide latency differently: caches vs warp oversubscription.
