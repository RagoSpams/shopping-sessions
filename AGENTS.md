# Shopping Sessions

- Correctness first, then speed. A change counts only if it stays exactly correct.
- Make one change per experiment, in its own branch or worktree. Keep the change only if all checks pass and runtime improves. Otherwise, revert and log it.
- Log every experiment in `experiments.md` with its id, change, correctness result, runtime, and whether it was kept or reverted.
- Never hard-code anything from the data. Seeds, windows, and dirt rates change.
- Use no network at run time. Install dependencies in the Dockerfile at build time and pin versions.
- Keep credentials, secrets, and raw data out of the repo. Keep the repo under 50 MB.

## How to test locally

```sh
make fixtures-local
PATH="$HOME/.cargo/bin:$PATH" cargo build --release --manifest-path "$HOME/shopping/generator/Cargo.toml"
make dev-local
make run-dev-local
make check-dev-local
make gen-local SEED=2 EVENTS=1000 CLICKS=250
```
