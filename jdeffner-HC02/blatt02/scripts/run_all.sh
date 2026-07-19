#!/usr/bin/env bash
# Runs every sweep and writes the CSVs into results/. Idempotent; finishes in
# well under 10 minutes on a T4. Binaries are expected in ./build (override
# with BIN=<dir>).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-build}"
mkdir -p results/plots

echo "== Task 1, experiment A: throughput scaling =="
"$BIN/task1_divergence" --exp scaling > results/task1_scaling.csv
"$BIN/cpu_baseline" --exp scaling --no-header >> results/task1_scaling.csv

echo "== Task 1, experiment B: warp divergence =="
"$BIN/task1_divergence" --exp divergence > results/task1_divergence.csv
"$BIN/cpu_baseline" --exp divergence --no-header >> results/task1_divergence.csv

echo "== Task 2, experiment C: access patterns =="
"$BIN/task2_bandwidth" --exp patterns > results/task2_patterns.csv

echo "== Task 2, experiment D: occupancy =="
"$BIN/task2_bandwidth" --exp occupancy > results/task2_occupancy.csv

echo "done, CSVs in results/"
