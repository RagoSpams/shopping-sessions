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

## Round 3

### R3-PROFILE · 20M parse self time

The redesign control was `224d9d8`. The profiling build used Rust 1.90.0, the release optimization settings, debug information and frame pointers. Linux perf 7.0.14 recorded DWARF call graphs (4 KiB, 999 Hz, `cpu-clock:u`), pinned to CPUs 0–7 with eight workers. WSL reported hardware cycles unsupported. The unchanged release profile was used for timing comparisons.

The three monotonic-clock windows from end-of-setup to end-of-link-parse, end-of-link-build to end-of-event-parse, and end-of-event-dedup to end-of-click-parse contain 4,767 user-mode samples. The recording lost zero samples. The profile run spent 0.846 s wall and 6.213 process CPU-s in parse, including kernel time. Percentages below divide each instruction's sample period by the total sampled period in those windows. Each instruction is assigned to its innermost DWARF function. This resolves scanner and record code that the non-inlined `perf report --no-children` displays under Rayon worker closures.

| Rank | Function at sampled instruction | Samples | Parse self |
|---:|---|---:|---:|
| 1 | `core::ptr::write` | 1,340 | 28.11% |
| 2 | `redesign::key::Key::new` | 445 | 9.34% |
| 3 | `time::calendar_ms` | 387 | 8.12% |
| 4 | `__memcpy_avx_unaligned_erms` | 349 | 7.32% |
| 5 | `__memcmp_avx2_movbe` | 205 | 4.30% |
| 6 | `time::digits` | 186 | 3.90% |
| 7 | `redesign::scan::split` field closure | 133 | 2.79% |
| 8 | `redesign::scan::split` | 125 | 2.62% |
| 9 | `time::bounded_uint` | 111 | 2.33% |
| 10 | `alloc::vec::Vec::push_mut` | 105 | 2.20% |
| 11 | `redesign::records::parse_events` chunk closure | 100 | 2.10% |
| 12 | `hashbrown::raw::RawTable::find` closure | 96 | 2.01% |
| 13 | `_mm256_movemask_epi8` | 88 | 1.85% |
| 14 | `u128::trailing_zeros` | 81 | 1.70% |
| 15 | `u8::wrapping_sub` | 74 | 1.55% |

The control already partitions before user interning and ID dedup. During event and click parsing, it scatters into 1,024 per-worker ID buffers and consults the catalogs. The sampled store cost motivates testing sequential output. The software clock does not measure cache misses.

### R3-FLAT · sequential parse with deferred radix partitioning

Each worker appended to one contiguous vector. Records contained 64-bit hashes for the record ID and the user, a mapped location for exact ID equality, and an inline-or-mapped user key. The parser performed no catalog or ID-table lookup. After parsing, a histogram sized the per-ID radix buffers, then each partition selected winners by full ID equality and earliest ingest. Only winners accessed the catalog. User interning and routing still used exact equality after partitioning on the user hash. Link endpoints also moved to contiguous streams.

The release build passed 36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating Hyperfine pairs, both binaries warmed, eight workers on CPUs 0–7. Every entry is the median wall seconds / process CPU-seconds for that stage. End to end includes process exit.

| Version | Parse | Links | Dedup + ID partition | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Control | 0.850 / 6.244 | 0.041 / 0.255 | 0.468 / 3.225 | 0.192 / 1.322 | 0.488 / 3.176 | 0.082 / 0.522 | 0.001 / 0.002 | 2.272 / 14.904 | 3.81 |
| R3-FLAT | 0.562 / 3.934 | 0.058 / 0.291 | 1.083 / 7.769 | 0.191 / 1.272 | 0.436 / 3.018 | 0.077 / 0.488 | 0.001 / 0.002 | 2.587 / 17.152 | 3.93 |

**Reverted.** Parse saved 2.310 CPU-s (37.0%). Moving wide records into 1,024 partitions and returning to input bytes for winners cost 4.544 extra CPU-s in dedup. End-to-end wall rose 13.9%.

### R3-FILTER

Within each existing ID partition, a bit seen just once proves the row unique. A second bitmap routes all potentially repeated keys to the exact winner table. The number of repeat hits bounds its initial capacity. Colliding projections only increase work.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| base | 0.936 / 6.931 | 0.047 / 0.289 | 0.506 / 3.527 | 0.215 / 1.501 | 0.512 / 3.469 | 0.086 / 0.560 | 0.001 / 0.002 | 2.438 / 16.420 | 3.81 |
| filter | 0.922 / 6.799 | 0.041 / 0.265 | 0.405 / 2.930 | 0.222 / 1.542 | 0.511 / 3.383 | 0.090 / 0.580 | 0.001 / 0.002 | 2.385 / 15.826 | 3.82 |

**Kept.** The smaller winner tables reduced dedup CPU and end-to-end time.

### R3-GROUP

Use dense partition-local person IDs for a counting scatter of events and clicks. Sort timestamps within each resulting person range before sessionization and attribution.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| filter | 0.806 / 5.919 | 0.040 / 0.251 | 0.336 / 2.400 | 0.190 / 1.287 | 0.452 / 2.968 | 0.079 / 0.479 | 0.001 / 0.002 | 2.063 / 13.486 | 3.80 |
| grouped | 0.852 / 6.251 | 0.038 / 0.245 | 0.338 / 2.442 | 0.192 / 1.334 | 0.391 / 2.547 | 0.075 / 0.470 | 0.001 / 0.002 | 2.034 / 13.308 | 3.80 |

**Kept.** Person ordering and aggregation used less CPU and wall time, and the end-to-end median improved.

### R3-RADIX64

Reduce initial record-ID radix fanout from 1,024 to 64 after adopting selective dedup. This tests a shorter active write frontier without copying the full streams in a separate pass.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| grouped | 0.809 / 5.934 | 0.052 / 0.329 | 0.341 / 2.405 | 0.185 / 1.285 | 0.398 / 2.557 | 0.081 / 0.529 | 0.001 / 0.002 | 2.043 / 13.330 | 3.82 |
| coarse | 0.684 / 4.968 | 0.038 / 0.242 | 0.402 / 3.015 | 0.185 / 1.302 | 0.393 / 2.585 | 0.079 / 0.515 | 0.001 / 0.002 | 1.939 / 12.722 | 3.69 |

**Kept.** The lower parse cost outweighed the larger partition-local candidate tables. Calibration scored 2.28 s on the board. The local native target is about 0.85 s.

### R3-WIDEKEY

Build each short inline key from a masked 16-byte SIMD load when all 16 bytes lie in the mapping. Retain the scalar copy at the tail.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| coarse | 0.718 / 5.088 | 0.041 / 0.251 | 0.456 / 3.323 | 0.195 / 1.335 | 0.393 / 2.709 | 0.084 / 0.543 | 0.001 / 0.002 | 2.019 / 13.511 | 3.71 |
| widekey | 0.694 / 4.760 | 0.042 / 0.262 | 0.470 / 3.468 | 0.198 / 1.393 | 0.434 / 2.778 | 0.085 / 0.558 | 0.001 / 0.002 | 2.070 / 13.397 | 3.71 |

**Reverted.** Parse CPU fell, but the five-pair end-to-end wall median increased.

### R3-SCALE · calibration at 20M and 60M

The release diagnostic build of calibration `b9c43d2` recorded counters at stage boundaries. Each CSV page was queried with `mincore` before launch. Cache residency was observed passively. CPU affinity and worker count matched the 20M gates. The second 60M output matched both tables of the previously validated seed-6 run, including 6,604,995 user/day and 28,014 campaign/day rows.

| Input/run | CSV pages resident before run | Wall s | User + system CPU-s | Peak RSS GiB | Major faults | Parse CPU-s | Dedup CPU-s | Resolve CPU-s | Sessions CPU-s |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 20M | 100.00% | 2.24 | 12.60 + 2.21 | 3.81 | 0 | 6.16 | 3.09 | 1.34 | 3.25 |
| 60M first | 38.38% | 11.41 | 50.82 + 16.95 | 9.96 | 46,480 | 35.86 | 13.15 | 5.56 | 9.94 |
| 60M repeat | 88.43% | 8.11 | 47.41 + 7.64 | 10.35 | 7,866 | 21.48 | 14.42 | 5.51 | 10.51 |

The first 60M run reproduced the previously reported 68 CPU-s. Of its 46,480 major faults, 46,466 had occurred by end-of-click-parse. Compared with that run, the repeat saved 14.39 parse CPU-s and 9.31 system CPU-s across the whole process. At end-of-click-parse in the repeat, RSS included 4.49 GiB anonymous and 5.28 GiB file-backed pages. The roughly 10 GiB peak includes the mapped CSV pages. Even at higher initial residency, dedup used 4.66× the 20M CPU and resolve used 4.11×. Larger local tables and different seed distributions remain plausible contributors. Hardware cache counters were unavailable.

### R3-PACK

Represent up to nine 7-bit characters losslessly in one word. An explicit mapped form handles other lengths, high bytes, and nonempty fields ending in NUL. Materialize byte strings only for distinct directory names. Exact byte comparisons resolve mapped-key hash collisions.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| coarse | 0.931 / 4.740 | 0.055 / 0.285 | 0.581 / 2.893 | 0.481 / 1.305 | 1.007 / 3.177 | 0.174 / 0.604 | 0.003 / 0.003 | 3.689 / 13.477 | 3.70 |
| packed | 0.965 / 4.728 | 0.124 / 0.285 | 0.553 / 2.473 | 0.564 / 1.428 | 0.917 / 3.060 | 0.170 / 0.520 | 0.005 / 0.002 | 3.397 / 13.055 | 3.35 |

**Kept.** The string round-trip and equality test covered quoting, length boundaries, NUL, UTF-8 and CR. The parser boundary test also passed. Smaller records reduced peak RSS and improved end-to-end wall time on a contended host.

### R3-PACK versus calibration

A separate paired run compares the cumulative candidate to the calibration ELF. Host contention elevated absolute wall time in this series.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| base | 1.357 / 6.515 | 0.079 / 0.253 | 0.620 / 3.129 | 0.374 / 1.416 | 1.034 / 3.481 | 0.160 / 0.581 | 0.002 / 0.002 | 3.979 / 15.499 | 3.81 |
| packed | 0.736 / 4.197 | 0.097 / 0.279 | 0.395 / 2.310 | 0.374 / 1.315 | 0.950 / 2.813 | 0.176 / 0.502 | 0.001 / 0.002 | 3.199 / 11.621 | 3.36 |

The cumulative improvement triggered the container gate.

The root image for `53b6ea0` built in 53.61 s with the cached pinned Rust 1.90.0 base. It runs the static redesign in scratch as UID/GID 1000. With scorer isolation, CPUs 0–7 and NCPU=8 it passed 36/36 fixtures and all five frozen comparisons. The timed 20M output also matched. Docker wall median was 2.468 s, range 2.351–3.164 s, five after one warmup. The final five-pair native comparison was 3.979 to 3.199 s (19.6%) under shared load.

### R3-DIRECTCLICK

Route validated identical-copy clicks straight to user partitions. Duplicate multiplicity leaves both latest-click credit and the set of assisting campaigns unchanged.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| packed | 0.853 / 4.258 | 0.086 / 0.264 | 0.495 / 2.310 | 0.352 / 1.303 | 0.602 / 2.755 | 0.134 / 0.476 | 0.001 / 0.002 | 2.767 / 11.676 | 3.37 |
| directclick | 0.825 / 4.174 | 0.074 / 0.290 | 0.378 / 1.853 | 0.313 / 1.278 | 0.772 / 2.990 | 0.158 / 0.510 | 0.003 / 0.002 | 2.922 / 11.705 | 3.32 |

**Reverted.** The initial five-pair wall median increased. A second five-pair series measured 3.089 → 3.027 s, but the pooled ten-run medians were 2.928 → 2.930 s. CPU medians were 11.828 → 11.508 s. The wall gate was unmet.

### R3-CALENDAR

Cache exact Gregorian date prefixes in 32 per-chunk slots, selected by day of month. Reuse the scalar date decoder on misses. Validate the entire time-of-day mask and convert decimal pairs in one word. F2 offsets and F3 window checks remain exact.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| packed | 0.630 / 4.621 | 0.045 / 0.289 | 0.381 / 2.735 | 0.210 / 1.433 | 0.420 / 2.797 | 0.094 / 0.554 | 0.001 / 0.002 | 1.967 / 12.780 | 3.36 |
| calendar | 0.584 / 4.212 | 0.047 / 0.291 | 0.354 / 2.615 | 0.212 / 1.412 | 0.414 / 2.701 | 0.085 / 0.538 | 0.001 / 0.002 | 1.815 / 11.966 | 3.35 |

**Kept.** Exhaustive single-byte mutations at every position in four date/offset examples matched the scalar parser, including leap-century cases and non-ASCII bytes. Native end-to-end and parse medians improved.

### R3-TICKETS

Partition payloads by user during parsing. Send an eight-byte projected ID and row slot through the ID radix instead of moving the payload again. Only repeated projections enter an exact-key table. Its loser slots are marked after all winners are known. The user owners discard losing and invalid rows during routing.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| calendar | 0.576 / 4.203 | 0.049 / 0.292 | 0.373 / 2.824 | 0.217 / 1.472 | 0.438 / 2.793 | 0.088 / 0.543 | 0.001 / 0.002 | 1.868 / 12.132 | 3.36 |
| tickets | 0.686 / 5.019 | 0.043 / 0.276 | 0.148 / 1.069 | 0.208 / 1.500 | 0.415 / 2.769 | 0.094 / 0.535 | 0.001 / 0.002 | 1.755 / 11.659 | 3.42 |

**Kept.** The extra parse scatter was cheaper than repartitioning and indexing every payload. The revised target is 0.75 s local native for a board score below 0.80 s.
