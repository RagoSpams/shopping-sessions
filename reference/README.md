# Frozen Shopping Sessions oracle

This directory contains the frozen correctness oracle. Never edit
`reference/pipeline.py`. It passed all 36 fixtures and exactly matched the
published 1M-event dev answer before it was frozen.

Run it with Python 3.12 and the pinned PyArrow dependency in the repository's
`requirements.txt`. It reads `DATA_DIR`, writes both Parquet tables to
`OUTPUT_DIR`, and accepts `SCRATCH_DIR` like the main pipeline.

For future pipeline changes, run the oracle and the candidate on the same
input with separate output directories. Compare the output directories with
`python scripts/compare.py ORACLE_OUTPUT CANDIDATE_OUTPUT`.
