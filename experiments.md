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

### R3-TICKETS versus R3-PACK

Independent cumulative comparison against the last reported native milestone.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| packed | 0.734 / 4.998 | 0.053 / 0.302 | 0.493 / 3.165 | 0.222 / 1.446 | 0.515 / 3.334 | 0.173 / 0.574 | 0.001 / 0.002 | 2.477 / 14.883 | 3.36 |
| tickets | 0.780 / 5.538 | 0.050 / 0.285 | 0.158 / 1.108 | 0.238 / 1.690 | 0.429 / 2.929 | 0.094 / 0.590 | 0.001 / 0.002 | 1.939 / 12.413 | 3.42 |

R3-TICKETS crossed the revised 10% milestone. The 0.75 s native target is still open.

### R3-USERS32

Reduce user/person fanout from 256 to 32 to shorten the parse write frontier. This also increases the populations of the individual alias directories.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| tickets | 0.850 / 4.547 | 0.074 / 0.260 | 0.229 / 1.003 | 0.322 / 1.402 | 0.769 / 2.821 | 0.135 / 0.493 | 0.003 / 0.002 | 2.705 / 10.712 | 3.36 |
| user32 | 0.826 / 4.329 | 0.080 / 0.261 | 0.221 / 0.880 | 0.458 / 2.085 | 0.624 / 2.642 | 0.146 / 0.478 | 0.003 / 0.002 | 2.649 / 10.913 | 3.37 |

**Kept.** Both five-pair series improved end-to-end wall on this host. Pooled medians were 2.678 → 2.648 s. Resolve CPU increased in both series. The next experiment removes its second directory lookup.

### R3-ROOTVALUE

After union, replace each alias table value with its canonical global person ID so successful lookups return the root without consulting a separate root array.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| user32 | 0.838 / 4.516 | 0.048 / 0.265 | 0.268 / 0.897 | 0.553 / 2.054 | 0.684 / 2.775 | 0.135 / 0.473 | 0.001 / 0.002 | 2.716 / 11.127 | 3.34 |
| rootvalue | 1.032 / 4.682 | 0.055 / 0.259 | 0.297 / 0.941 | 0.497 / 2.139 | 0.598 / 2.753 | 0.131 / 0.508 | 0.003 / 0.002 | 2.771 / 11.418 | 3.31 |

**Reverted.** Resolve wall fell, but resolve CPU and end-to-end median rose.

### R3-RESERVE

Estimate initial per-worker capacities using complete newline-terminated rows sampled from the first page of each mapping. Divide the estimate among shards, cap speculative bytes against the input size, and retain normal Vec growth for skew.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| user32 | 0.573 / 4.218 | 0.046 / 0.320 | 0.123 / 0.871 | 0.239 / 1.768 | 0.364 / 2.630 | 0.071 / 0.433 | 0.001 / 0.002 | 1.534 / 10.324 | 3.38 |
| reserve | 0.531 / 3.653 | 0.049 / 0.320 | 0.132 / 0.889 | 0.244 / 1.793 | 0.389 / 2.672 | 0.074 / 0.470 | 0.001 / 0.002 | 1.509 / 9.828 | 3.05 |

**Kept.** Parse CPU and median wall fell with fewer reallocations. Peak RSS fell by 0.33 GiB.

### R3-OWNER

Route each parsed row to its final person owner using the immutable union directory and a 2,048-entry per-chunk exact alias cache. Intern unresolved singleton names at the receiving owner. Each accepted record then converts to a person row in its existing partition.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| reserve | 0.904 / 5.030 | 0.075 / 0.306 | 0.210 / 1.130 | 0.417 / 2.484 | 0.561 / 3.264 | 0.113 / 0.537 | 0.002 / 0.003 | 2.386 / 13.099 | 3.06 |
| owner | 2.218 / 14.710 | 0.075 / 0.318 | 0.246 / 1.203 | 0.099 / 0.570 | 0.661 / 3.797 | 0.120 / 0.566 | 0.001 / 0.002 | 3.584 / 21.603 | 3.03 |

**Reverted.** Resolve became cheaper, but global alias probes in the parse loop cost far more. This directly tests the cost of introducing per-row random interning lookups into the parser.

### R3-INGEST

Specialize the guaranteed-valid F1 ingest path using the cached date and fixed clock digits.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| reserve | 0.558 / 3.840 | 0.055 / 0.344 | 0.144 / 0.992 | 0.278 / 2.011 | 0.401 / 2.742 | 0.076 / 0.491 | 0.001 / 0.002 | 1.572 / 10.362 | 3.10 |
| ingest | 0.544 / 3.792 | 0.047 / 0.291 | 0.141 / 0.945 | 0.305 / 2.256 | 0.394 / 2.901 | 0.088 / 0.552 | 0.001 / 0.002 | 1.659 / 11.065 | 3.07 |

**Reverted.** Parse showed only a small reduction and the locked end-to-end median rose.

### R3-EPOCH

For nine to sixteen decimal epoch digits, validate two overlapping words and combine four decimal pairs at a time. Other lengths retain the scalar parser.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| reserve | 0.529 / 3.651 | 0.050 / 0.330 | 0.127 / 0.844 | 0.247 / 1.804 | 0.364 / 2.566 | 0.072 / 0.447 | 0.001 / 0.002 | 1.491 / 9.751 | 3.05 |
| epoch | 0.524 / 3.662 | 0.048 / 0.289 | 0.136 / 0.928 | 0.250 / 1.827 | 0.402 / 2.795 | 0.079 / 0.492 | 0.001 / 0.002 | 1.541 / 10.119 | 3.07 |

**Reverted.** Parse CPU was unchanged and end-to-end wall rose.

### R3-HOT

Cache recent exact packed aliases and canonical roots in 2,048 direct-mapped entries per user shard.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| reserve | 0.556 / 3.846 | 0.045 / 0.285 | 0.136 / 0.918 | 0.245 / 1.763 | 0.398 / 2.732 | 0.080 / 0.534 | 0.001 / 0.002 | 1.594 / 10.478 | 3.05 |
| hot | 0.547 / 3.803 | 0.048 / 0.318 | 0.122 / 0.819 | 0.249 / 1.809 | 0.372 / 2.702 | 0.080 / 0.489 | 0.001 / 0.003 | 1.558 / 10.170 | 3.05 |

**Reverted.** The second locked series reversed the end-to-end result (1.588 → 1.625 s). Resolve got slower in both series, including 1.984 → 2.111 CPU-s on confirmation.

### R3-DIMWORD

Create a second exact catalog index for keys that fit into seven bytes plus length. Use a masked word load when all eight bytes are mapped.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| reserve | 0.527 / 3.611 | 0.046 / 0.303 | 0.126 / 0.829 | 0.259 / 1.891 | 0.388 / 2.814 | 0.073 / 0.465 | 0.002 / 0.004 | 1.537 / 10.054 | 3.05 |
| dims | 0.526 / 3.691 | 0.052 / 0.346 | 0.155 / 1.031 | 0.265 / 1.964 | 0.377 / 2.675 | 0.076 / 0.465 | 0.001 / 0.002 | 1.599 / 10.385 | 3.09 |

**Reverted.** The extra index did not reduce parse time and increased end-to-end latency.

### R3-LINEAR

Replace the local Swiss alias table and separate root array with a two-thirds-full linear-probing table. Each slot holds the exact packed key, a hash tag and the canonical person together.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| reserve | 0.559 / 3.828 | 0.048 / 0.304 | 0.125 / 0.843 | 0.252 / 1.784 | 0.378 / 2.662 | 0.077 / 0.508 | 0.001 / 0.002 | 1.565 / 10.165 | 3.03 |
| linear | 0.556 / 3.848 | 0.047 / 0.307 | 0.131 / 0.894 | 0.272 / 2.046 | 0.395 / 2.800 | 0.075 / 0.459 | 0.001 / 0.002 | 1.611 / 10.623 | 3.07 |

**Reverted.** Resolving all rows cost more CPU and the end-to-end wall median rose.

### R3-IDPASS

Make an event-ID-only CSV pass. Radix-partition eight-byte projection/offset tickets, validate candidate copies structurally, compare decoded IDs and F1 ingest timestamps exactly, then skip sorted loser offsets during the full parse. Only surviving rows enter the user scatter.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| reserve | 0.880 / 4.919 | 0.078 / 0.325 | 0.233 / 1.188 | 0.467 / 2.652 | 0.550 / 3.288 | 0.099 / 0.512 | 0.002 / 0.002 | 2.496 / 12.852 | 3.05 |
| prepass | 0.721 / 3.763 | 0.081 / 0.324 | 0.562 / 2.691 | 0.535 / 2.897 | 0.573 / 3.284 | 0.140 / 0.572 | 0.001 / 0.002 | 2.891 / 13.650 | 2.96 |

**Reverted.** The second input pass and mapped byte checks cost more in dedup than the narrower full parse saved.

### R3-SCALE follow-up · accepted ticket pipeline

A diagnostic build of `572e871` used the same passive residency and stage counters as the calibration diagnostic. The 60M repeat matched both prior validated tables. These single-run measurements used the shared timing lock. Other host work still affected CPU speed.

| Input/run | CSV pages resident before run | Wall s | User + system CPU-s | Peak RSS GiB | Major faults | Parse CPU-s | Dedup CPU-s | Resolve CPU-s | Sessions CPU-s |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 20M | 100.00% | 3.11 | 12.77 + 2.63 | 3.10 | 87 | 5.83 | 1.43 | 3.32 | 3.69 |
| 60M first | 25.09% | 13.04 | 45.69 + 17.95 | 9.14 | 47,678 | 30.40 | 6.88 | 12.58 | 10.91 |
| 60M repeat | 97.68% | 7.13 | 39.07 + 7.69 | 9.37 | 8,276 | 16.12 | 5.81 | 11.55 | 10.04 |

The repeat reduced dedup from calibration's 14.42 to 5.81 CPU-s, while resolve rose from 5.51 to 11.55 CPU-s with the larger 32-way alias shards. Initial residency and host load differed between runs. A separate 20M resolve profile assigned 45.4% of user-mode samples to the Swiss table's find-or-insert-slot closure and 13.1% to the root fetch.

### R3-PREFETCH

Batch 32 row hashes and prefetch the corresponding exact linear alias slots before probing for canonical roots. This revisits the rejected linear table with independent outstanding memory requests.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| reserve | 0.580 / 4.062 | 0.048 / 0.311 | 0.145 / 0.989 | 0.265 / 1.985 | 0.378 / 2.766 | 0.074 / 0.476 | 0.001 / 0.002 | 1.600 / 10.546 | 3.06 |
| prefetch | 0.555 / 3.927 | 0.048 / 0.312 | 0.139 / 0.936 | 0.201 / 1.423 | 0.395 / 2.629 | 0.073 / 0.451 | 0.001 / 0.002 | 1.514 / 9.781 | 3.06 |

**Kept.** Batched probing lowered both resolve CPU and wall and improved the full-process median.

### R3-USERS128

Increase the user fanout from 32 to 128 after introducing batched linear alias probes, reducing the working set of each active directory.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| prefetch | 0.678 / 4.292 | 0.051 / 0.305 | 0.170 / 0.940 | 0.273 / 1.592 | 0.443 / 2.740 | 0.089 / 0.467 | 0.001 / 0.002 | 1.829 / 10.503 | 3.07 |
| user128 | 0.590 / 4.157 | 0.047 / 0.288 | 0.149 / 0.960 | 0.170 / 1.270 | 0.383 / 2.574 | 0.086 / 0.556 | 0.001 / 0.002 | 1.559 / 9.743 | 3.13 |

**Kept.** The smaller directories reduced resolve CPU and end-to-end wall.

### R3-HANDLES

Write every parsed row to one contiguous per-worker vector. Replace the user key with a per-worker handle, deduplicated through a 16,384-entry direct-mapped recent-user cache. After dedup, route only the handle aliases to the user directories, then scatter rows by person through a handle-to-root array.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| user128 | 0.598 / 4.139 | 0.051 / 0.340 | 0.141 / 0.951 | 0.186 / 1.305 | 0.346 / 2.429 | 0.082 / 0.515 | 0.001 / 0.002 | 1.551 / 10.083 | 3.11 |
| handles | 0.772 / 5.484 | 0.044 / 0.284 | 0.135 / 0.943 | 0.397 / 2.450 | 0.370 / 2.586 | 0.080 / 0.513 | 0.001 / 0.003 | 1.885 / 12.347 | 3.57 |

**Reverted.** The recent-user cache missed on 21.0M of 26.2M rows, so nearly every row still produced an alias. Parse rose from 0.598 to 0.772 s and resolve from 0.186 to 0.397 s.

### R3-INPLACE

Carry the event ID and ingest time (or the click ID) in the ID-partition ticket, apply the late check during parse, and shrink each row to its user key and final payload. A row then has the size and alignment of its person row. Resolution rewrites every user-partition run in place and scatters only the rows of persons owned by another partition, through per-owner lists built in parallel. Exact deduplication reads the ticket side data sequentially.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| user128 | 0.581 / 4.103 | 0.043 / 0.285 | 0.124 / 0.888 | 0.197 / 1.399 | 0.384 / 2.713 | 0.095 / 0.551 | 0.001 / 0.002 | 1.560 / 9.949 | 3.14 |
| inplace | 0.537 / 3.845 | 0.046 / 0.279 | 0.098 / 0.697 | 0.110 / 0.850 | 0.328 / 2.359 | 0.077 / 0.482 | 0.001 / 0.002 | 1.300 / 8.570 | 2.99 |

**Kept.** Resolve fell from 0.197 to 0.110 s and dedup from 0.124 to 0.098 s, with lower parse and aggregate medians and 0.15 GiB lower peak RSS. A routed variant that sent rows to their person's owner during parse through a 4-byte hint table needed no forwarding but cost 0.15–0.3 s more parse time, even with huge pages and a prefetch ring; without hints, 35% of rows move between partitions.

### R3-FIELDS

Decode hot fields with fewer branches and lookups: an exact open-addressing catalog index compares product and campaign names that pack into one word as integers (other names use the catalog's byte map); packed keys hash with one folded 64x64-bit product instead of the keyed hasher; thirteen-digit F3 timestamps convert with two overlapping eight-digit SWAR words, while other lengths keep the general parser.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| inplace | 0.724 / 4.466 | 0.061 / 0.312 | 0.128 / 0.720 | 0.148 / 1.045 | 0.502 / 2.901 | 0.088 / 0.551 | 0.002 / 0.003 | 1.804 / 10.238 | 2.99 |
| fields | 0.557 / 3.901 | 0.053 / 0.321 | 0.101 / 0.699 | 0.130 / 0.937 | 0.393 / 2.885 | 0.086 / 0.549 | 0.001 / 0.002 | 1.438 / 9.108 | 2.99 |

**Kept.** CPU-seconds were lower in all five pairs (median 9.108 vs 10.238). Wall medians were 1.438 vs 1.804 s; the first two pairs ran under heavy external load (2.35–2.56 s for both builds), and three of five pairs favored the new build. New unit tests compare the SWAR path with the general parser under every single-byte mutation of five thirteen-digit values and check both window bounds, and check the catalog index against the byte map, including long, non-ASCII and NUL-terminated names.

## Output experiments on the partition-local redesign

Native measurements use release builds with x86-64-v3, NCPU=8 and CPUs 0–7. Hyperfine measures process start to exit, with a warmup, five measured runs, and fresh output files. CPU seconds include user and system time. The native correctness gate covers all 36 fixtures and both tables on dev, s2-200k, s3-3m, s4-10m and s5-20m.

| ID | Change | Correctness | Native 20M median wall / CPU | Disposition |
| --- | --- | --- | --- | --- |
| R3-OUT-1 | Emit contiguous strings, nullable bitmap and numeric columns for fastpq. Write one uncompressed PLAIN user file per worker batch and one campaign file. | 36/36 and five MATCHes | 2.035 s / 13.492 CPU-s vs main 2.218 s / 14.444 CPU-s. Separate five-run writer timing was 0.024 s. | Kept |
| R3-OUT-2 | Request transparent huge pages for glibc allocations at startup with `glibc.malloc.hugetlb=1`. | Same writer and session implementation as R3-OUT-1. | 3.897 s / 15.248 CPU-s on the shared host. Median last timing line to waitpid was 0.159 s vs 0.128 s for R3-OUT-1. | Reverted |
| R3-OUT-3 | Discard read-only link and event page-table entries on a scoped thread during aggregation. Long names can fault back from immutable files. | 36/36 and five MATCHes | Alternating five-run medians 3.482 s / 14.533 CPU-s vs 2.596 s / 14.147 CPU-s for R3-OUT-1 under variable concurrent load. | Reverted |
| R3-OUT-4 | Compare mimalloc 3.2.8 with glibc using the same native executable. | Same writer and session implementation as R3-OUT-1. | Alternating five-run medians 2.545 s / 15.747 CPU-s vs 2.429 s / 15.318 CPU-s. Median residual after subtracting instrumented stages improved from 0.163 to 0.106 s, while peak RSS grew from 4.06 to 4.82 GB. | Reverted |
| R3-OUT-5 | Count rows by shard-local person, scatter into contiguous ranges, then sort each range by timestamp. | 36/36 and five MATCHes | Alternating five-run medians 2.358 s / 14.721 CPU-s vs 2.455 s / 14.967 CPU-s for R3-OUT-1. Aggregate CPU fell from 3.000 to 2.597 s; aggregate wall medians were 0.437 and 0.475 s respectively on the shared host. | Kept |
| R3-OUT-6 | Index campaign totals by manifest day and catalog campaign. Use the sparse map when the dense matrix would exceed 8 MiB per worker. | 36/36 and five MATCHes | Alternating five-run medians 2.061 s / 12.937 CPU-s vs 2.118 s / 13.369 CPU-s for R3-OUT-5. Aggregate improved from 0.369 s / 2.451 CPU-s to 0.322 s / 2.070 CPU-s. | Kept |
| R3-OUT-7 | Carry counted range ends into session attribution. Store each person's event and click payloads contiguously, omit repeated person IDs, and reuse daily category scratch. | 36/36 and five MATCHes | Alternating five-run medians 2.075 s / 13.626 CPU-s vs 2.205 s / 14.479 CPU-s for R3-OUT-6. Aggregate improved from 0.355 s / 2.263 CPU-s to 0.326 s / 1.971 CPU-s. | Kept |
| R3-OUT-8 | Pack each event into a sortable u64 when the manifest and catalog fit losslessly. Keep a full-width fallback. | 36/36 and five MATCHes | Alternating five-run medians 2.072 s / 13.522 CPU-s vs 2.078 s / 13.613 CPU-s for R3-OUT-7. Aggregate CPU fell from 1.867 to 1.807 s with near-identical wall time (0.295 s). | Kept for combined evaluation |

The original redesign already calls `process::exit` after all writers finish. A parent timed receipt of the final stage line through waitpid. In five fastpq runs the median was 0.128 s, with about 0.003 s from launch through the first instrumented stage boundary beyond the reported stages. An exit probe saw 3,909,440 KiB resident, including 2,073,248 KiB of anonymous pages. Discarding the PTEs for 282 input mappings (1.88 GB) took 0.075 s, and an explicit malloc trim took another 0.194 s in that diagnostic run. Both allocator experiments R3-OUT-2 and R3-OUT-4 subsequently passed 36/36 fixtures and all five oracle comparisons.

The initial cpu-clock profile (199 Hz, DWARF call graphs) attributed 7.21% of all samples to event quicksort, 2.12% to its small-sort and 3.90% to campaign map entry. After R3-OUT-8, the output-only branch measured 1.979 s / 13.042 CPU-s versus 2.240 s / 14.889 CPU-s for original main in five alternating pairs. Separate wide-window and wide-catalog cases matched the frozen oracle on both tables.

The combined branch incorporates the other lane's packed-key and selective dedup changes through `3d11a96` (same code as `53b6ea0`). Its counted ranges supersede the independent implementation of dense person grouping in that lane. On native it passed the full gate. Against original main, five alternating 20M pairs measured 1.595 s / 10.603 CPU-s versus 2.380 s / 15.127 CPU-s. Combined stage medians were 0.267 s for aggregation, 0.019 s for Parquet, and 0.126 s outside the in-process timers. The root Rust 1.90 image passed 36/36 fixtures and the five scorer-isolated frozen comparisons. This was the validation already in progress when the native release target changed to 0.750 s.

| ID | Change | Correctness | Native 20M median wall / CPU | Disposition |
| --- | --- | --- | --- | --- |
| R3-OUT-9 | Process the largest remaining person shard first from a shared work queue while retaining per-worker output buffers. | 36/36 and five MATCHes | Alternating medians 2.268 s / 10.342 CPU-s vs 1.997 s / 10.480 CPU-s for the combined branch. Aggregate medians improved from 0.366 to 0.330 s, but end-to-end wall regressed under shared load. | Reverted |
| R3-OUT-10 | Track the last eligible click time per campaign, retire expired campaigns while counting assists, and compare click IDs only at the winning timestamp. | 36/36 and five MATCHes | Alternating medians 2.547 s / 9.793 CPU-s vs 2.703 s / 9.889 CPU-s for the combined branch. Aggregate CPU fell from 1.735 to 1.661 s. | Kept |
| R3-OUT-11 | Sort large packed event histories with a four-pass stable timestamp radix and reuse scratch across each worker's shards. Keep comparison sort for smaller histories. | 36/36 and five MATCHes | Five alternating pairs 1.807 s / 10.523 CPU-s vs 2.072 s / 10.454 CPU-s. A second five pairs gave 1.602 s / 10.153 CPU-s vs 1.800 s / 10.886 CPU-s, with aggregation at 0.267 vs 0.295 s. | Kept |
| R3-OUT-M2 | Integrate the input lane through `0338629`, including compact tickets, calendar decoding and reduced user fanout. | 36/36 and five MATCHes | Five alternating pairs 1.297 s / 8.662 CPU-s vs 1.444 s / 9.330 CPU-s for R3-OUT-11 on the earlier input lane. This series completed before the shared timing lock was introduced. | Kept |
| R3-OUT-12 | Link pinned jemalloc into the redesign and advise transparent huge pages for payloads and metadata. The root builder installs pinned make to compile it. | 36/36 and five MATCHes. Rust 1.90 root image built and fixture E13 passed with scorer flags. | Shared-lock five-pair medians 1.381 s / 9.097 CPU-s vs 1.434 s / 9.494 CPU-s for R3-OUT-M2. Residual after timed stages fell from 0.113 to 0.055 s; setup grew from 0.002 to 0.011 s. Peak RSS was 3.83 vs 3.62 GB. | Kept |
| R3-OUT-13 | Fuse session boundary detection, day counts, purchase sums and credit in one linear pass over each person's events. | 36/36 and five MATCHes | Locked five-pair medians 1.490 s / 10.238 CPU-s vs 1.381 s / 8.967 CPU-s for R3-OUT-12. Aggregate medians were 0.231 vs 0.215 s. | Reverted |
| R3-OUT-14 | Drop the owned input mappings in parallel with writing the self-contained output columns. | 36/36 and five MATCHes | Locked five-pair medians 1.312 s / 8.941 CPU-s vs 1.340 s / 9.068 CPU-s for R3-OUT-12. The writer plus unmap stage took 0.049 s vs 0.018 s for writing alone, and median residual after stages fell from 0.054 to 0.017 s. | Kept |
| R3-OUT-15 | Sort a single u64 key per event holding person, timestamp and an index into a purchase side table. Fall back when the available bits cannot encode an input. | 36/36 and five MATCHes | Locked five-pair medians 1.244 s / 8.590 CPU-s vs 1.242 s / 8.650 CPU-s for R3-OUT-14. Aggregate worsened from 0.188 s / 1.349 CPU-s to 0.223 s / 1.508 CPU-s. | Reverted |
| R3-OUT-16 | Copy canonical names to owned columns and retire the link/event maps during aggregation. Rebase click locations into the remaining maps and retire them during writing. | 36/36 and five MATCHes | Locked five-pair medians 1.429 s / 9.191 CPU-s vs 1.413 s / 8.786 CPU-s for R3-OUT-14. The joined write stage fell from 0.054 to 0.025 s but aggregate rose from 0.228 to 0.264 s. | Reverted |
| R3-OUT-17 | Directly credit zero- and one-click people, bypassing the sliding campaign set. | 36/36 and five MATCHes | Locked five-pair medians 1.358 s / 9.000 CPU-s vs 1.300 s / 8.826 CPU-s for R3-OUT-14. Aggregate was 0.196 vs 0.192 s. | Reverted |
| R3-OUT-M3 | Integrate the input lane's reserved scatter buffers through `0ff36c3`. | Redesign and default each passed 36/36 and five MATCHes. Redesign also matched the frozen oracle with a 400-year window, a wide catalog and linked long UTF-8 names with a click-ID tie. All five Rust unit tests passed. | Locked alternating five-run medians after builds finished were 1.223 s / 8.557 CPU-s vs 1.307 s / 9.024 CPU-s for R3-OUT-14. An earlier series during validation builds measured 1.759 s / 10.671 CPU-s vs 1.666 s / 10.941 CPU-s. The final timing line to waitpid was 0.010 s median in five separate runs. | Kept |
| R3-OUT-M4 | Integrate the input lane's 128-way user fanout (`b46669e`). | 36/36 and five MATCHes | Locked alternating five-run medians 1.280 s / 8.820 CPU-s vs 1.290 s / 8.730 CPU-s for `9c1616c` on a loaded host. | Kept |
| R3-OUT-18 | Unmap the input chunks from a parallel iterator while the writers run. | 36/36 and five MATCHes | Locked five-pair medians 1.170 s / 7.950 CPU-s vs 1.180 s / 8.070 CPU-s for R3-OUT-M4. The writer plus unmap stage took 0.063 vs 0.059 s, so separate munmaps did not overlap. | Reverted |
| R3-OUT-19 | Count assists in a separate day-major campaign matrix. Expire inactive campaigns first, then increment one compact row per session. A diagnostic counted 3.42M sessions and 34.6M active-campaign increments at 20M. | 36/36 and five MATCHes. All Rust unit tests passed. | Locked five-pair medians 1.130 s / 7.760 CPU-s vs 1.210 s / 8.070 CPU-s for R3-OUT-M4. Aggregate improved from 0.191 s / 1.365 CPU-s to 0.173 s / 1.238 CPU-s. | Kept |
| R3-OUT-20 | Keep per-worker session and assist counts as dense `u32` day-major matrices when the event count fits. Touch purchase totals only for nonzero revenue. | 36/36 and five MATCHes | Three locked series against R3-OUT-19: 1.290 vs 1.310 s, 1.090 vs 1.040 s and 1.070 vs 1.065 s (10 pairs). Aggregate CPU fell only from 1.202 to 1.176 s in the 10-pair series. | Reverted |
| R3-OUT-21 | Revisit R3-OUT-3 under the timing lock: discard all input page-table entries on a scoped thread during aggregation. | 36/36 and five MATCHes | Seven locked pairs 1.220 s / 8.420 CPU-s vs 1.210 s / 8.430 CPU-s for R3-OUT-19. The write stage fell from 0.065 to 0.027 s, but aggregate rose from 0.203 to 0.224 s. | Reverted |
| R3-OUT-22 | At the write stage, discard input page-table entries with `MADV_DONTNEED` from the worker pool instead of unmapping serially. Exit removes the empty mappings. | 36/36 and five MATCHes | Locked series of seven and eight pairs on a loaded host: 1.440 vs 1.460 s and 1.305 vs 1.360 s for R3-OUT-19. The write stage fell from 0.081 to 0.044 s and from 0.073 to 0.040 s. | Kept |
| R3-OUT-23 | Write each worker's user file as soon as its shards finish. That worker then claims input chunks from a shared cursor and discards their page-table entries while others still aggregate. Rare later reads fault the immutable bytes back. | 36/36 and five MATCHes. All Rust unit tests passed. | Two locked eight-pair series against R3-OUT-22: 1.125 vs 1.215 s and 1.090 vs 1.140 s. Aggregate plus write was 0.214 vs 0.225 s and 0.207 vs 0.206 s. Per-run time from aggregate start to exit in the second series was 0.218 vs 0.225 s median. | Kept |
| R3-OUT-24 | Revisit R3-OUT-9 with 128 user shards and early release: workers claim the largest remaining shard from a shared queue instead of a fixed contiguous group. | 36/36 and five MATCHes | Two locked eight-pair series against R3-OUT-23: aggregate 0.185 vs 0.199 s and 0.193 vs 0.199 s; aggregate start to exit 0.194 vs 0.209 s and 0.207 vs 0.211 s median; end to end 1.015 vs 1.015 s and 1.060 vs 1.095 s. | Kept |
| R3-OUT-25 | Count assists lazily. Expire campaigns from a time-ordered click queue and record each activation's same-day session count. Flush counts on expiry, day change and person end instead of incrementing every active campaign per session. | 36/36 and five MATCHes | Three locked series against R3-OUT-24 (8, 8 and 10 pairs) gave aggregate start to exit 0.222 vs 0.235 s, 0.209 vs 0.216 s and 0.213 vs 0.209 s median. Aggregate CPU was 1.596 vs 1.581, 1.537 vs 1.559 and 1.548 vs 1.546 s. | Reverted |
| R3-OUT-26 | Group packed events by person in one pass over the routed runs, using 16-slot block chains in one arena. Gather multi-block people into scratch before the timestamp sort. This removes the counting pass's second read of the 24-byte rows. A diagnostic split of aggregate CPU before the change was 0.18 s click ordering, 0.47 s event ordering, 0.56 s processing and 0.11 s Parquet writing. | 36/36 and five MATCHes | Locked series of 8 and 10 pairs against R3-OUT-24: aggregate 0.207 vs 0.218 s, then 0.196 vs 0.191 s; aggregate start to exit 0.216 vs 0.228 s, then 0.210 vs 0.205 s. | Reverted |
| R3-OUT-27 | Pack `Event`, `Click` and `PersonRow` to four-byte alignment. Routed event rows shrink from 24 to 20 bytes and click rows from 32 to 24. Fields are only copied, never borrowed. | 36/36 and five MATCHes. All Rust unit tests passed. | Locked series of 8 and 10 pairs against R3-OUT-24: 1.000 vs 1.125 s and 1.035 vs 1.060 s end to end. Resolve fell from 0.147 to 0.122 s and from 0.146 to 0.124 s. Aggregate start to exit was 0.203 vs 0.203 s and 0.199 vs 0.207 s. | Kept |
| R3-OUT-M5 | Integrate the input lane's in-place user-partition resolve (`e536a5b`). Its conversion requires `Record<T>` and `PersonRow<T>` to have equal size and alignment, so this merge drops R3-OUT-27's four-byte packing. Most of that packing's gain came from routed-row traffic, and the new resolve no longer routes most rows. | 36/36 and five MATCHes. All Rust unit tests passed. | See the paired timing below. | Kept |
| R3-OUT-28 | Defer per-session campaign-session and assist increments into 16K-entry cell-index logs, and apply each log in a tight batch. A diagnostic that skipped the per-session session-cell update saved 0.12 CPU-s (0.017 s wall), so the update stalls. | 36/36 and five MATCHes, on both 8c520f7 and the merged 125f3eb | Locked 8 pairs on 8c520f7: aggregate 0.205 vs 0.212 s, aggregate start to exit 0.222 vs 0.226 s (mean 0.253 vs 0.230). Locked 10 pairs on 125f3eb under host load near 5: 0.299 vs 0.306 s; aggregate CPU 2.126 vs 2.155 s; end to end 1.475 vs 1.425 s. | Reverted |
| R3-OUT-29 | Counting-scatter rows by person and a monotone coarse time bucket (up to 2^16 cursors per shard), then sort only within buckets. | 36/36 and five MATCHes | Locked 8 pairs against 125f3eb: aggregate 0.233 vs 0.230 s; aggregate start to exit 0.251 vs 0.255 s. With about 8K people per shard only four buckets fit, and the event sort phase stayed at 0.287 vs 0.281 CPU-s. | Reverted |
| R3-OUT-30 | Bucket-sort event histories of 64 or more rows: about one bucket per four rows over the history's own time range, insertion sort for small buckets, comparison sort for crowded ones. | 36/36 and five MATCHes | Locked 8 pairs against 125f3eb: aggregate 0.197 vs 0.190 s. The phase diagnostic showed event sort CPU rising from 0.297 to 0.337 s and from 0.283 to 0.306 s. A size-class diagnostic on 125f3eb split event sort CPU almost evenly: 0.09 s for 4.3M rows in histories under 16, 0.07 s for 5.7M rows under 64, 0.10 s for 7.1M rows under 4096 and 0.04 s for 2.5M rows in larger histories. | Reverted |
| R3-OUT-31 | Sum equal-shaped dense campaign matrices cell by cell and emit non-empty cells directly. Other combinations still merge through a map. A diagnostic measured the serial hash-map merge at 4–7 ms after the parallel aggregation. | 36/36, five MATCHes, plus the wide-catalog and long-window edge sets (now in the gate). A diagnostic build forcing sparse totals matched s2-200k and s3-3m. | Locked 10 pairs against 125f3eb: aggregate start to exit 0.197 vs 0.205 s median (0.197 vs 0.207 mean); end to end 0.995 vs 0.980 s, with parse at 0.542 vs 0.514 s. | Kept |
| R3-OUT-32 | Write each output integer column as INT32 when every value in that file's column fits, else INT64. The rules accept any integer width. The 20M user output shrank from 93 to 67 MB. Ablations on the rebased head: skipping the user Parquet writes saved 0.19 CPU-s, and skipping emit plus writes saved 0.29 CPU-s. | 36/36, five MATCHes, wide-catalog and long-window sets; all Rust unit tests passed. | Locked 5-pair series against 11531f0: aggregate 0.165 vs 0.171 s and 0.203 vs 0.214 s; aggregate start to exit 0.178 vs 0.181 s and 0.219 vs 0.227 s; end to end 0.830 vs 0.830 s (quiet) and 1.060 vs 1.010 s (loaded). | Kept |

### R3-PACKED

Parse each event straight into the aggregate's sortable word (window-relative millisecond, category, price) when the window and catalog fit; a full price field marks a row that cannot survive, and wider inputs keep full-width rows. Dedup tickets shrink to the exact ID plus one word that holds the ingest second above the row slot. Event rows and tickets fall from 24 + 24 to 16 + 16 bytes, and resolution rewrites the 16-byte rows in place.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| merged | 0.688 / 4.645 | 0.063 / 0.385 | 0.102 / 0.716 | 0.153 / 1.186 | 0.242 / 1.874 | 0.001 / 0.001 | 0.007 / 0.009 | 1.287 / 8.927 | 3.14 |
| pk16 | 0.605 / 4.202 | 0.068 / 0.472 | 0.105 / 0.746 | 0.130 / 1.008 | 0.228 / 1.763 | 0.002 / 0.002 | 0.006 / 0.008 | 1.175 / 8.316 | 2.85 |

**Kept.** All five pairs favored the packed build (1.175 vs 1.287 s median); system CPU fell from 1.911 to 1.162 s and peak RSS from 3.14 to 2.85 GiB. Both builds also matched the frozen oracle on three edge sets derived from s2-200k: a 400-year window and a 1,000-category catalog (full-width path), and a 49.7-day window ending 296 ms short of the 32-bit offset limit (packed path).

### R3-POPREAD

Prefault each input chunk's cached file pages with `MADV_POPULATE_READ` before parsing it (links, events and clicks).

36/36 fixtures were not run. Three alternating single 20M runs against `965fab4` measured 1.08/0.89/0.94 s vs 1.04/0.91/0.92 s end to end, with system CPU 1.34/0.94/1.03 vs 1.32/0.91/1.04 s.

**Reverted.** Fewer minor faults did not lower system time or wall time; fault-around already maps cached file pages in 64 KiB groups.

### R3-CATALOG

Load products and campaigns directly into the packed-word index, with category IDs in byte order and no intermediate string maps, on one worker while the others map the inputs, parse the links and join them. The Links column now covers that whole startup join, and Parse covers only events and clicks.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| pk16 | 0.664 / 4.628 | 0.053 / 0.357 | 0.125 / 0.856 | 0.151 / 1.148 | 0.242 / 1.850 | 0.001 / 0.001 | 0.008 / 0.013 | 1.279 / 8.855 | 2.85 |
| catalogs | 0.506 / 3.787 | 0.080 / 0.467 | 0.115 / 0.811 | 0.132 / 1.030 | 0.236 / 1.836 | 0.001 / 0.001 | 0.005 / 0.009 | 1.104 / 8.022 | 2.83 |

**Kept.** The serial product catalog had taken 13.5–20.5 ms before link parsing began. Five pairs measured 1.104 vs 1.279 s and 8.02 vs 8.86 CPU-s on a loaded host; the three-pair screen measured 1.007 vs 1.044 s. All three edge sets still match.

### R3-SCAN

Three scanner variants against `a1bd272`, each screened with 36/36 fixtures, dev and s3-3m MATCH, and three alternating 20M pairs. A new randomized unit test compares the scanner with the reference line and CSV parser on 200 generated chunks per field count (2, 3, 4 and 7 fields), covering quotes in every position, empty lines, CR bytes, wrong field counts and rows longer than the vector window.

| Variant | Parse s (base / new) | End to end s (base / new) |
|---|---:|---:|
| Sliding 128-byte window of newline, comma and quote masks built once per 64-byte block | 0.395 / 0.453 | 0.814 / 0.877 |
| Per-line masks as before, comma positions from two `u64` halves and unchecked field slices | 0.411 / 0.403 | 0.828 / 0.850 |
| Per-line masks over all four 32-byte blocks without the early newline exit | 0.417 / 0.450 | 0.835 / 0.905 |

**Reverted.** None beat the per-line scanner, which runs at 3.35 GB/s per core against an 11–13 GB/s mask-only floor; carrying window state between rows cost more than recomputing masks per line. The randomized test is kept.

### R3-CHUNKS

Map links in 1 MiB chunks and clicks in 2 MiB chunks; events keep 8 MiB. The few, large click and link chunks had left most workers idle at the end of each of those phases.

36/36 fixtures. Both tables MATCH on dev, s2-200k, s3-3m, s4-10m and s5-20m. Five alternating 20M Hyperfine pairs, one warmup per binary. Cells show median wall seconds / CPU-seconds.

| Build | Parse | Links | Dedup | Resolve | Sessions + aggregate | Parquet | Setup | End to end | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| int1 | 0.407 / 3.103 | 0.050 / 0.288 | 0.077 / 0.540 | 0.099 / 0.769 | 0.171 / 1.342 | 0.001 / 0.001 | 0.004 / 0.006 | 0.831 / 6.119 | 2.83 |
| chunks | 0.396 / 3.057 | 0.046 / 0.281 | 0.079 / 0.561 | 0.099 / 0.770 | 0.171 / 1.352 | 0.001 / 0.001 | 0.004 / 0.006 | 0.802 / 6.007 | 2.83 |

**Kept.** Parse fell from 0.407 to 0.396 s and links from 0.050 to 0.046 s; end to end 0.831 to 0.802 s in five pairs (screen 0.815 to 0.798 s).

### R3-BITPAIR

Interleave each dedup partition's seen and repeated bitmap words so both tests for a ticket share a cache line.

Screen: 36/36 fixtures, dev and s3-3m MATCH. Three alternating 20M pairs against `9c4f186`: dedup 0.085 vs 0.088 s, end to end 0.868 vs 0.826 s.

**Reverted.** Dedup time did not fall.

### R3-PROVISION

Pre-create the event parse batches and let one helper thread map their pages with `MADV_POPULATE_WRITE`, 256 KiB per buffer per round, front to back while the workers parse.

Screen: 36/36 fixtures, dev and s3-3m MATCH. Three alternating 20M pairs against `9c4f186`: parse 0.396 vs 0.435 s, system CPU 0.986 vs 1.197 s, end to end 0.803 vs 0.881 s.

**Reverted.** On this host the helper's page provisioning competed with the workers and raised kernel time instead of replacing their faults. With the earlier input prefault (R3-POPREAD) and the jemalloc huge-page comparison, this exhausts the provisioning lever here; the kept reduction came from smaller rows (R3-PACKED).
