# Experiments

| ID | Change | Correctness result | Runtime | Kept or reverted |
| --- | --- | --- | --- | --- |
| rust-01 | Exact static Rust pipeline with parallel CSV indexing, byte-wise deduplication, person-owned aggregation, and Snappy Parquet | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, and s5-20m | Median of 3 local runs with `taskset -c 0-7`, `NCPU=8`, and `MEM_GB=24`. 20M 13.32 s, 2.09 GiB peak RSS. 10M 4.71 s, 1.06 GiB. Dev 0.95 s, 123 MiB. Hyperfine medians on 20M and 10M were 11.27 s and 5.42 s | Kept |
| rust-02 | Fuse CSV comma and quote scanning for 16–128 byte lines with baseline x86-64 SSE2 | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, and s5-20m | Alternating comparison with rust-01. 20M 9.24 → 7.34 s, 2.10 GiB peak RSS. 10M 3.38 → 3.10 s, 1.06 GiB. Dev 0.40 → 0.36 s, 123 MiB | Kept |
| rust-03 | Pack short linked aliases losslessly and rewrite the union-find interner values in place | 36/36 fixtures. MATCH on dev, s2-200k, s3-3m, s4-10m, s5-20m, and the long-ID and CSV-length boundary check | Alternating comparison with rust-02. 20M 11.68 → 7.86 s, 2.06 GiB peak RSS. 10M 3.28 → 3.03 s, 1.04 GiB. Dev 0.84 → 0.77 s, 122 MiB. Shared load varied between runs | Kept |
