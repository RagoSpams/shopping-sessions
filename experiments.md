# Experiments

| ID | Change | Correctness result | Runtime | Kept or reverted |
| --- | --- | --- | --- | --- |
| rust-01 | Exact static Rust pipeline with parallel CSV indexing, byte-wise deduplication, person-owned aggregation, and Snappy Parquet | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, and s5-20m | Median of 3 local runs with `taskset -c 0-7`, `NCPU=8`, and `MEM_GB=24`. 20M 13.32 s, 2.09 GiB peak RSS. 10M 4.71 s, 1.06 GiB. Dev 0.95 s, 123 MiB. Hyperfine medians on 20M and 10M were 11.27 s and 5.42 s | Kept |
| rust-02 | Fuse CSV comma and quote scanning for 16–128 byte lines with baseline x86-64 SSE2 | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, and s5-20m | Alternating comparison with rust-01. 20M 9.24 → 7.34 s, 2.10 GiB peak RSS. 10M 3.38 → 3.10 s, 1.06 GiB. Dev 0.40 → 0.36 s, 123 MiB | Kept |
| rust-03 | Pack short linked aliases losslessly and rewrite the union-find interner values in place | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, s5-20m, and the long-ID and CSV-length boundary check | Alternating comparison with rust-02. 20M 11.68 → 7.86 s, 2.06 GiB peak RSS. 10M 3.28 → 3.03 s, 1.04 GiB. Dev 0.84 → 0.77 s, 122 MiB. Shared load varied between runs | Kept |
| rust-04 | Index campaign tallies by campaign within each active day | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, and s5-20m | Alternating comparison with rust-03. 20M 5.77 → 6.05 s, 2.06 GiB peak RSS. 10M 2.60 → 2.54 s. Dev 0.35 → 0.37 s | Reverted |
| rust-05 | Promote the static Rust image to the root Dockerfile and set the team metadata | Docker passed 36/36 fixtures. MATCH on the published dev answer and frozen oracle outputs for dev, s2-200k, s3-3m, s4-10m, and s5-20m. The timed 20M output also MATCHed | Docker 20M 6.51 s median of 3 after one warmup. Cold image build 102.96 s with pinned Rust 1.90.0. Image 1.06 MB content, 3.57 MB on disk | Kept |

## Final Rust verification

Local release runs used an i5-12450H with 15 GiB visible RAM, `taskset -c 0-7`, `NCPU=8`, and `MEM_GB=24`. A clean offline static build took 45.73 s with the local Rust 1.100.0 toolchain.

| Dataset | Runs | Wall time | Peak RSS |
| --- | ---: | ---: | ---: |
| dev (1M) | 3 | 0.34 s median | 124,236 KiB |
| s4-10m | 3 | 3.09 s median | 1,089,560 KiB |
| s5-20m | 3 | 5.66 s median | 2,161,468 KiB |
| s6-60m | 1 | 30.98 s | 6,312,548 KiB |

Local Hyperfine medians were 3.21 s at 10M and 5.99 s at 20M. The 60M check timed seed 6 with 15M clicks and verified the cross-table totals and assist invariants. Peak RSS was 6.02 GiB, compared with a 6.18 GiB linear estimate from 20M.

The Docker runs used the scorer flags and the requested 24 GiB container limit on a Docker Desktop VM with 15.62 GiB of RAM. The 20M Docker median was above the 1.32 s leaderboard target.

## DuckDB fallback on local branch `fm/shop-duckdb`

| ID | Change | Correctness result | Runtime | Kept or reverted |
| --- | --- | --- | --- | --- |
| D04 | Materialize typed events before validation to avoid recomputing field splits and timestamp conversions | 36/36 fixtures, dev, s2-200k, s3-3m, s4-10m, s5-20m MATCH | Dev 3.84 s median, 572,076 KiB peak RSS. s5-20m 57.81 s median, 5,848,988 KiB peak RSS | Kept |
