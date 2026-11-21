# Experiments

| ID | Change | Correctness result | Runtime | Kept or reverted |
| --- | --- | --- | --- | --- |
| rust-01 | Exact static Rust pipeline with parallel CSV indexing, byte-wise deduplication, person-owned aggregation, and Snappy Parquet | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, and s5-20m | Median of 3 local runs with `taskset -c 0-7`, `NCPU=8`, and `MEM_GB=24`. 20M 13.32 s, 2.09 GiB peak RSS. 10M 4.71 s, 1.06 GiB. Dev 0.95 s, 123 MiB. Hyperfine medians on 20M and 10M were 11.27 s and 5.42 s | Kept |
| rust-02 | Fuse CSV comma and quote scanning for 16–128 byte lines with baseline x86-64 SSE2 | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, and s5-20m | Alternating comparison with rust-01. 20M 9.24 → 7.34 s, 2.10 GiB peak RSS. 10M 3.38 → 3.10 s, 1.06 GiB. Dev 0.40 → 0.36 s, 123 MiB | Kept |
| rust-03 | Pack short linked aliases losslessly and rewrite the union-find interner values in place | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, s5-20m, and the long-ID and CSV-length boundary check | Alternating comparison with rust-02. 20M 11.68 → 7.86 s, 2.06 GiB peak RSS. 10M 3.28 → 3.03 s, 1.04 GiB. Dev 0.84 → 0.77 s, 122 MiB. Shared load varied between runs | Kept |
| rust-04 | Index campaign tallies by campaign within each active day | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, and s5-20m | Alternating comparison with rust-03. 20M 5.77 → 6.05 s, 2.06 GiB peak RSS. 10M 2.60 → 2.54 s. Dev 0.35 → 0.37 s | Reverted |
| rust-05 | Promote the static Rust image to the root Dockerfile and set the team metadata | Docker passed 36/36 fixtures. MATCH on the published dev answer and frozen oracle outputs for dev, s2-200k, s3-3m, s4-10m, and s5-20m. The timed 20M output also MATCHed | Docker 20M 6.51 s median of 3 after one warmup. Cold image build 102.96 s with pinned Rust 1.90.0. Image 1.06 MB content, 3.57 MB on disk | Kept |
| rust-06 | Add a standalone writer for both output schemas using uncompressed PLAIN data pages | Rust 1.90.0 unit tests passed. PyArrow and Polars `load_actual` passed random and edge-case round-trips and a 7M-row single-file check. `compare.py` reported MATCH on dev and s5-20m with one and eight files per table | s5-20m writer-only A/B from the same in-memory columns, median of 9 unpinned runs per method. On tmpfs, Snappy to fastpq took 429.62 → 69.01 ms for one file per table and 96.57 → 21.45 ms for eight. On ext4, 644.96 → 151.40 ms and 124.58 → 37.73 ms. fastpq wrote 98,407,269 bytes and 98,415,547 bytes respectively | Kept as a standalone module |

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

## Round 2 · separate partitioned binary

Round 2 introduced an opt-in image for the partitioned binary. The root Dockerfile now builds that binary for scoring. For a native build, enable the `redesign` feature, select `--bin shopping-redesign`, and use the static CRT and `x86-64-v3` flags from the root Dockerfile.

| ID | Change | Correctness | Local 20M median | Disposition |
|---|---|---|---|---|
| R2-M1 | Single CSV decode, exact inline or mapped keys, partition-local dedup and user lookup, parallel canonical union-find, person-owned session processing and plain Parquet fallback | 36/36 fixtures, dev, 200k, 3M, 10M and 20M both tables MATCH. PyArrow read every output file in these five datasets. Last timed native and Docker outputs also MATCH | Native 2.329 s and 15.60 CPU-s. Docker 3.063 s from invocation to return and 15.54 in-process CPU-s. Stock Step 0 controls were 5.336 s native and 6.254 s Docker | Kept alongside default |

Native and Docker use the same static Rust 1.90 ELF. Hyperfine 2.0.0, five measured runs after one warmup, clean output before each, NCPU=8 and CPUs 0–7. Native CPU is process-lifetime user plus system time. Docker CPU is the application's process clock between entry and writer close. In-process stage times include work and deallocation performed before a stage ends. The remaining native wall interval includes static startup and kernel teardown after exit. In Docker that residual also includes client and daemon overhead.

| 20M stage | Native wall budget | M1 native wall | M1 native CPU | M1 Docker wall | M1 Docker CPU |
|---|---:|---:|---:|---:|---:|
| Parse, map, dimensions and first scatter | 0.220 | 0.923 | 6.837 | 0.926 | 6.755 |
| Link interning, union and canonical names | 0.030 | 0.042 | 0.262 | 0.038 | 0.248 |
| ID dedup, validate, late, scatter by user | 0.070 | 0.456 | 3.259 | 0.469 | 3.308 |
| Resolve users and scatter by person | 0.070 | 0.219 | 1.497 | 0.199 | 1.327 |
| Person sort, sessions, credit, assists, aggregation | 0.100 | 0.465 | 3.126 | 0.473 | 3.175 |
| Parquet fallback | 0.030 | 0.114 | 0.784 | 0.129 | 0.758 |
| Startup and exit including pool setup | 0.020 | 0.145 | — | 0.802 | — |
| **End to end** | **0.550 wall / 3.500 CPU** | **2.329** | **15.601** | **3.063** | **15.545** |

M1 peak RSS at 20M was 3.82 GiB. Seed 6 at 60M took 22.52 s wall, 105.20 CPU-s and 9.49 GiB peak RSS. Both 60M tables matched the earlier validated Rust output, with 6,604,995 user/day and 28,014 campaign/day rows. That additional comparison is independent of the five frozen reference datasets.

### Focused parse experiment and M2 stopping gate

| ID | Change | Correctness | Local 20M median | Disposition |
|---|---|---|---|---|
| R2-SCAN | Fuse newline, comma and quote detection in 32-byte AVX2 loads. BMI2 packs the opening and closing quote flags per field. Long lines and mapping tails use the existing exact parser. | Scanner boundary test PASS. 36/36 fixtures and all five frozen datasets MATCH. PyArrow read every output part. Both timed outputs MATCH. | Native 2.174 s / 14.629 CPU-s. Docker 3.054 s / 15.603 in-program CPU-s. Five alternating native pairs below confirm lower parse and end-to-end times. | Kept alongside default. M2 stopped at the parse gate. |

| 20M stage | Native wall target | M2 maximum (2×) | Native wall | Native CPU | Docker wall | Docker CPU |
|---|---:|---:|---:|---:|---:|---:|
| Parse, map, dimensions and first scatter | 0.220 | 0.440 | 0.828 | 6.049 | 0.949 | 6.399 |
| Links | 0.030 | 0.060 | 0.045 | 0.286 | 0.041 | 0.247 |
| Dedup, validate and late | 0.070 | 0.140 | 0.435 | 3.113 | 0.471 | 3.212 |
| Resolve and scatter by person | 0.070 | 0.140 | 0.204 | 1.347 | 0.240 | 1.522 |
| Sort, sessions, credit and aggregates | 0.100 | 0.200 | 0.441 | 3.067 | 0.509 | 3.274 |
| Parquet fallback | 0.030 | 0.060 | 0.098 | 0.667 | 0.119 | 0.826 |
| Startup and exit including pool setup | 0.020 | 0.040 | 0.147 | — | 0.760 | — |
| **Total** | **0.550 wall / 3.500 CPU** | | **2.174** | **14.629** | **3.054** | **15.603** |

The separate native median-of-five runs put M1 parse at 0.923 s and R2-SCAN at 0.828 s. A second comparison alternated which ELF ran first in each pair, with five measurements each after warming both. M1 → R2-SCAN medians were 2.695 → 2.542 s end to end, 15.409 → 14.376 CPU-s, and 1.004 → 0.883 s for parse/scatter. Four of five pairs favored the new scanner on wall time. Docker host-wall medians differed by 0.009 s in the separate series, below the observed run-to-run variation.

The focused scanner still used **1.88× the 0.440 s M2 cutoff** in its five-run series. Its parse phase alone took 6.049 CPU-s against the 3.500 CPU-s total target. M1 is exact. M2 and M3 remain unmet. Further optimization stopped at the specified gate. The 0.90 s board target remains unmeasured.

The final 60M check took 10.55 s wall and 67.90 CPU-s at 10.02 GiB peak RSS. Its complete output matched the prior validated 60M output. These are single runs with uncontrolled cache residency at 60M. The shared-code default binary also passed 36 fixtures and all five frozen oracle comparisons. The additional long-ID and CSV-boundary dataset matched the M1 redesign.

## Calibration

| ID | Change | Correctness result | Runtime | Kept or reverted |
| --- | --- | --- | --- | --- |
| rust-07 | Package the partition-local redesign in the root Dockerfile | Native default and redesign each passed 36/36 fixtures and matched dev, s2-200k, s3-3m, s4-10m, and s5-20m. Both fastpq unit tests and the redesign scanner test passed offline. The calibration container passed 36/36 fixtures and matched all five frozen datasets and the published dev answer. The final timed 20M output of each image matched the oracle. | 20M Docker median 9.718 s for the original image at `88fee99` and 3.467 s for the calibration image. Ranges were 5.960–10.953 s and 3.030–4.657 s, respectively. Cold rebuild 47.05 s. Image 1.16 MB content and 3.93 MB on disk. | Kept |

The calibration keeps fastpq standalone and uses the redesign's existing Parquet output. The root Dockerfile uses the digest-pinned Rust 1.90.0 builder with `-C target-feature=+crt-static -C target-cpu=x86-64-v3`. The resulting `linux/amd64` image uses `FROM scratch` and runs as UID and GID 1000.

Native gates used Rust 1.100.0, `taskset -c 0-7`, and `NCPU=8`. Their single 20M runs took 7.40 s for the default and 4.26 s for the redesign. Docker gates used the scorer flags with CPUs 0–7 and the requested 24 GiB limit on a Docker Desktop VM with 15.62 GiB available. Hyperfine 2.0.0 measured Docker invocation through return, with one warmup and five measurements per image. Its prepare step emptied the output and scratch directories between runs. The cold rebuild reused the pinned base image and downloaded and compiled the locked dependencies afresh.
