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
