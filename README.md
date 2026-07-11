# dev_engine — assignment autograder

Sandboxed autograder for university coding assignments across **Python**, **MERN/JavaScript (Bun)**,
and **Java**. A student submits a GitHub repo; we merge our private hidden tests into a fixed
template, install dependencies offline, run the test suite inside a Linux sandbox
(namespaces + cgroups + seccomp, no Docker), and return `{passed, failed, failing tests}`.

Reference implementation for the sandboxing primitives: `../lamicons-code-engine` (IronJudge).
This repo re-implements the pipeline cleanly, test-first.

## Pipeline

```
Submission (Kafka)
  → fetch student repo (GitHub tarball)
  → merge: copy ONLY the student's solution files into a fresh template (anti-cheat)
  → install deps offline (pip --no-index / bun install / maven -o)
  → run tests in sandbox → JUnit XML
  → parse JUnit XML → normalized TestReport
  → GradeResult (Kafka)
```

**One report format for every stack.** pytest (`--junitxml`), Bun (`bun test --reporter=junit`),
and Maven surefire all emit JUnit XML, so there is a single parser.

## Workspace

| Crate | Role | Status |
|---|---|---|
| `grader-types` | Shared vocab: `Stack`, `StackConfig`, `Submission`, `TestReport`, `GradeResult` | ✅ done |
| `report-parser` | JUnit XML → `TestReport` (pytest / bun / surefire dialects) | ✅ done |
| `project-merge` | Copy student solution into template; anti-cheat overlay | ⬜ next |
| `fetcher` | GitHub tarball download + extract | ⬜ |
| `sandbox` | namespaces + cgroups + seccomp "project" profile | ⬜ |
| `grader-engine` | Orchestrate the pipeline | ⬜ |
| `kafka-io` + `apps/grader` | Kafka worker binary | ⬜ |

## Develop

```bash
cargo test          # all crates
cargo clippy --all-targets
cargo fmt
```

## Anti-cheat model

The merge step copies **only** `StackConfig::solution_files` out of the student repo into a
fresh copy of the template. Tests, lockfiles, and config always come from the template, and the
pipeline runs **its own** test command — never the student's scripts. Combined with an offline,
network-namespaced sandbox, this blocks the common cheats (rewriting the test script, smuggling
a `conftest.py`, phoning home for answers).
