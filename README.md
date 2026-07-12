# dev_engine — assignment autograder

Sandboxed autograder for university coding assignments across **Python**, **MERN/JavaScript (Bun)**,
and **Java**. A student submits a **whole project repository** on GitHub; we combine it with our
private hidden tests, install dependencies offline, run the test suite inside a Linux sandbox
(namespaces + cgroups + seccomp, no Docker), and return `{passed, failed, failing tests}`.

The grading unit is a **project directory tree** (a real MERN app, a Maven project, a Python
package) — not a single solution file.

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
| `grader-types` | Shared vocab: `Stack`, `StackConfig`, `MergeMode`, `Submission`, `TestReport`, `GradeResult` | ✅ done |
| `report-parser` | JUnit XML → `TestReport` (pytest / bun / surefire dialects) | ✅ done |
| `project-merge` | Combine student submission + template (two modes); anti-cheat | ✅ done |
| `fetcher` | GitHub tarball download + extract (safe unpack) | ✅ done |
| `sandbox` | namespaces + cgroups + seccomp "project" profile | ✅ done |
| `grader-engine` | Orchestrate the pipeline (ports + adapters) | ✅ done |
| `apps/grader` | Kafka worker binary (consume → grade → produce) | ✅ done |

### `sandbox` runtime requirements

The namespaced runner (`run_project_sandbox`) needs, at run time:
* the ability to create **user namespaces** (rootless), and
* a baked **per-stack rootfs** at `config.rootfs` containing the toolchain + warmed dependency
  caches (e.g. `~/.m2`, bun cache, pip wheels) so the run stays **offline**.

Its pure helpers (shell-command assembly, `cpu.max`, the seccomp program) are unit-tested; the
`unshare`/`pivot_root`/`exec` core is integration-tested only on a provisioned host.

## Develop

```bash
cargo test          # all crates (42 tests, no root needed)
cargo clippy --all-targets
cargo fmt
cargo build -p grader   # the worker binary (builds librdkafka from source)
```

## Run (worker)

The `grader` binary is configured via the environment:

| Var | Default | Meaning |
|---|---|---|
| `KAFKA_BROKERS` | `localhost:9092` | Kafka bootstrap servers |
| `KAFKA_GROUP_ID` | `grader-workers` | consumer group |
| `SUBMISSION_TOPIC` | `assignment-submission` | consumed `Submission` events |
| `RESULT_TOPIC` | `assignment-result` | produced `GradeResult` events |
| `ASSIGNMENTS_ROOT` | `/opt/assignments` | on-disk assignments |
| `ROOTFS_BASE` | `/opt/sandbox_rootfs` | baked per-stack rootfs (`<base>/<stack>`) |
| `WORK_ROOT` | `/tmp/grader` | scratch for per-submission dirs |
| `GRADER_CONCURRENCY` | `4` | max concurrent grades |

**Assignment layout** (`ASSIGNMENTS_ROOT/<assignment_id>/`):

```
grader.json     # serialized StackConfig (stack, merge mode, install, test, report)
template/       # trusted template: hidden tests, locked config, stubs
```

**Deployment prerequisites** (for the sandbox to actually run untrusted code):
* host allows rootless **user namespaces**;
* baked rootfs at `ROOTFS_BASE/<stack>` with the toolchain + **warmed dependency caches**
  (so install/test run **offline**): pip wheels, bun global cache, `~/.m2`.

## Data contracts

* **Submission** (consumed): `{ submission_id, assignment_id, stack, repo_url, git_ref? }`
  — `stack` accepts `python` | `javascript` (aliases `js`/`mern`/`node`) | `java`.
* **GradeResult** (produced): `{ submission_id, status, passed, failed, skipped, total,
  failing_tests[], error? }` — `status` ∈ `graded` | `build_error` | `fetch_error` |
  `timeout` | `internal_error`.

## Merge modes (`MergeMode`)

Two assignment shapes, both anti-cheat by construction:

* **`SolutionFiles`** — "implement these files". Base is a fresh copy of the trusted template;
  only the declared student files are copied in. Best for function-level exercises.
* **`WholeProject`** — the student submits a whole project. Base is *their* repo; the template's
  `protected_paths` (hidden tests + locked build/test config) are stamped on top, always winning,
  and the student's version of each protected path is deleted first. `.git` and symlinks in the
  student tree are never copied.

Either way, tests and build config come from the template and the pipeline runs **its own** test
command — never the student's scripts. Combined with an offline, network-namespaced sandbox, this
blocks the common cheats (rewriting the test script, smuggling a `conftest.py`, phoning home).
