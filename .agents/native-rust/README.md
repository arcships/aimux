# Agent notes for native-Rust work

These files are for coding agents (and people) doing two kinds of work on aimux:

- aligning a Rust module with its pinned AI SDK upstream: the `ts-rust-port` skill;
- rewriting TS-shaped Rust into native Rust without changing behaviour: the `rust-native-rewrite` skill.

Nothing here is built or run by CI.

| Path | What it is |
|---|---|
| `../skills/ts-rust-port/SKILL.md` | Port, fix or check a module against its pinned upstream |
| `../skills/rust-native-rewrite/SKILL.md` | Behaviour-preserving native rewrite, proven by an old-vs-new differential |
| `PROFILE.md` | The repository facts both skills read: design-doc decisions, surfaces that must not change, oracles, checks, known bugs |
| `gate.sh` | Runs every CI check locally and totals passing tests per test binary, to compare a base and a change |
| `probe/` | Cross-build differential used by #259 and #260: replays the cassettes through every public language model on two builds and compares requests and stream parts |

The skills and profile are written in Chinese.
