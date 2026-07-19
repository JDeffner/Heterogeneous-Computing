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
   - GPU needs ~[n_sat] threads before it saturates ([threads/SM] per SM);
     CPU is flat from the first kilobyte.
   - Read off: GPU peak vs CPU peak GFLOP/s.

4. **Experiment B: controlled warp divergence** (plot task1_divergence.png)
   - d distinct paths per warp, identical work per path.
   - GPU: slowdown ≈ d (measured [x]x at d = 32). CPU: flat.
   - Loop-length variant: saturates at ~2x, different mechanism, same cause.

5. **Why serialization happens**
   - Pre-Volta: one PC per warp, paths executed one after another under the
     active mask. Volta+: per-thread PC, but divergent instruction streams
     still issue serially.
   - CPU: independent control flow per core + branch prediction, so the same
     branch is nearly free.

6. **Experiment C: memory access patterns** (plot task2_patterns.png)
   - Coalesced hits [x] % of theoretical peak; stride kills bandwidth
     geometrically; random gather is worst ([x] % of peak).
   - One warp = 32/128-byte transactions; scattered lanes waste the bus.

7. **Experiment D: occupancy and latency hiding** (plot task2_occupancy.png)
   - Bandwidth rises with resident warps, then plateaus: enough warps to hide
     DRAM latency, after that the DRAM itself is the limit.
   - Occupancy is a means (latency hiding), not a goal in itself.

8. **Takeaways**
   - Give the GPU enough parallel work (saturation point!).
   - Keep warps convergent; if you must branch, branch at warp granularity.
   - Layout: structure-of-arrays, coalesced access, avoid gather.
   - CPU and GPU hide latency differently: caches vs warp oversubscription.
