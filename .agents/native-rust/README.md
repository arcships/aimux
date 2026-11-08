# Agent notes for native-Rust work

For coding agents (and people) doing two kinds of work on aimux:

- aligning a Rust module with its pinned AI SDK upstream: `../skills/ts-rust-port/SKILL.md`;
- rewriting TS-shaped Rust into native Rust without changing behaviour: `../skills/rust-native-rewrite/SKILL.md`.

`PROFILE.md` holds the repository facts both read. Each skill has an `evals/` folder with rubric-graded cases for `claude plugin eval`; run them after changing a skill (they are real, billed model runs). `eval-scaffold.sh` sets each case up: it copies the case's crate and `PROFILE.md` into the workspace and writes `.eval/cargo`, which finds the local rustup toolchain from the temporary HOME the run gets. Run the evals from a checkout of this repository (each case's `place-files.sh` finds `eval-scaffold.sh` by its path in `.agents/`), and as a normal user: as root the eval sandbox hides `/root`, where the toolchain lives. Results go to `evals/results/` inside the skill, which git ignores. Nothing here is built or run by CI. The skills and profile are written in Chinese.
