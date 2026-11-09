# Contributing to spiritlevel

Thanks for your interest in contributing. This document covers the basics.

## Getting started

Relevant docs for contributors:

- [`doc/setup.md`](doc/setup.md) — build, test, MSRV and coverage commands
- [`doc/architecture.md`](doc/architecture.md) — role, scope, the rules for choosing the
  link and their sources, the types, the testing strategy and design principles
- [`doc/roadmap.md`](doc/roadmap.md) — what is planned and under consideration

## Issues and pull requests

**Open an issue first** if you're unsure whether something is a bug or if you want to
discuss a change before implementing it. For small, self-contained fixes a PR on its own
is fine.

- Bug reports: give the inputs to `TmdsLinkParams::choose` (rate, sink facts, source
  facts, policy), what it returned, and what you expected, ideally with the HDMI source
  implementation whose behaviour you expected it to match.
- Feature requests: a brief description of what you need and why is enough to start a
  conversation.
- PRs: keep them focused. One logical change per PR makes review faster and keeps history
  readable.

## Coding standards

- Run `cargo fmt` and `cargo clippy --all-targets -- -D warnings` before pushing.
- Public items need rustdoc comments (`cargo rustdoc -- -D warnings` must pass).
- Follow the existing patterns in the codebase — see
  [`doc/architecture.md`](doc/architecture.md) for the design principles behind them.
- `#![forbid(unsafe_code)]` is enforced; no unsafe code.
- Keep `no_std` compatibility without `alloc`, and keep constructors and `choose` usable
  in `const` contexts.
- Report every reason, not the first: a decision or error carries every condition that
  applies.
- A claim about HDMI behaviour (a rule, a default, a threshold) cites at least two
  independent public implementations (repository, commit, file and line) in the
  architecture document's Sources. Where implementations differ, the behaviour is a
  setting with the majority as default.

## Commit and PR expectations

- Write commit messages in the imperative mood ("Add support for …", not "Added …").
- Keep commits logically atomic. A PR that touches three unrelated things should be three
  commits (or three PRs).
- Tests are expected for every rule change. The input space is small enough to test whole:
  extend the tests that run every combination of inputs, and add a named case for the
  change.
- CI must be green before a PR can merge: fmt, clippy, docs, all test and build targets,
  the MSRV check, and coverage must not drop more than 0.1% below the baseline (stored in
  `.coverage-baseline`).

## Review process

PRs are reviewed on a best-effort basis. Expect feedback within a few days; if you haven't
heard back in a week feel free to ping the thread. Reviews aim to be constructive — if
something needs to change, the reviewer will explain why. Approval from the maintainer is
required to merge.

## Code of Conduct

This project follows the [Contributor Covenant 3.0](https://github.com/ashlar-display/.github/blob/HEAD/CODE_OF_CONDUCT.md),
the code of conduct of every [Ashlar](https://ashlar-display.org) repository. Please read it
before participating.
