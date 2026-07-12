# dev_engine — assignment autograder

Sandboxed autograder for university coding assignments across **Python**, **MERN/JavaScript (Bun)**,
and **Java**. A student submits a **whole project repository** on GitHub; we combine it with our
private hidden tests, install its dependencies, run the test suite inside a Linux sandbox
(namespaces + cgroups + seccomp, no Docker), and return `{passed, failed, failing tests}`.

The grading unit is a **project directory tree** (a real MERN app, a Maven project, a Python
package) — not a single solution file.

Reference implementation for the sandboxing primitives: `../lamicons-code-engine` (IronJudge).
This repo re-implements the pipeline cleanly, test-first.

## Pipeline

```
Submission (Kafka)
  → fetch student repo (GitHub tarball)
  → merge: student's code + the template's hidden tests and locked build config (anti-cheat)
  → install dependencies in the sandbox, ONLINE   (bun install / pip install / mvn go-offline)
  → run the hidden tests in the sandbox, OFFLINE  → JUnit XML
  → parse JUnit XML → normalized TestReport
  → GradeResult (Kafka)
```

Install and test are **two separate sandboxed runs** over the same workspace. Dependencies cannot
be known ahead of time — every project declares its own — so the install phase runs *our* command
over the project's manifest with the network up, before any student code executes. The test phase,
which is the one that runs untrusted code, gets an empty network namespace: no interface, no route.
Whatever the install downloaded (`node_modules/`, `.venv/`, `~/.m2`) is still there, because both
phases share `/work` and `$HOME`.

The full input/output contract — every field, and what comes back in every condition, including
the ones where the student's code never runs — is in **[`contract.md`](contract.md)**.

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
* the ability to create **user namespaces** (rootless) and to write cgroup v2 limits, and
* a **per-stack rootfs** at `config.rootfs` holding the toolchain (python+pip / bun / jdk+maven).

It runs **one phase**: `network: true` keeps the host's network namespace (the install phase's
whole purpose), `network: false` creates an empty one (the test phase, where untrusted code runs).

Its pure helpers (shell-command assembly, `cpu.max`, the seccomp program) are unit-tested; the
`unshare`/`pivot_root`/`exec` core is integration-tested only on a provisioned host.

## Develop

```bash
cargo test          # all crates (53 tests, no root needed)
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
| `ROOTFS_BASE` | `/opt/sandbox_rootfs` | per-stack toolchain rootfs (`<base>/<stack>`) |
| `WORK_ROOT` | `/tmp/grader` | scratch for per-submission dirs |
| `GRADER_CONCURRENCY` | `4` | max concurrent grades |

**Assignment layout** (`ASSIGNMENTS_ROOT/<assignment_id>/`):

```
grader.json     # serialized StackConfig (stack, merge mode, install, test, report)
template/       # trusted template: hidden tests, locked config, stubs
```

## Ship it

```bash
docker compose build                  # or: docker build -f apps/grader/Dockerfile -t dev-engine .
docker compose up -d grader           # server: KAFKA_BROKERS points at the existing Kafka
docker compose --profile local up -d  # laptop: also brings up a throwaway Kafka
```

Copy `.env.example` to `.env` first. The image ([`apps/grader/Dockerfile`](apps/grader/Dockerfile))
builds the worker and bakes one **rootfs per stack** — the *toolchain only*:

| Stack | In the rootfs | Installed per submission, at grade time |
|---|---|---|
| Python | `python3`, `venv`, `pip` | `python3 -m venv .venv && pip install -r requirements.txt` |
| JavaScript | `bun` | `bun install` |
| Java | JDK 17, Maven | `mvn dependency:go-offline` |

No third-party dependency is baked in, so the image knows nothing about the assignments. The
container runs **privileged** — the sandbox needs to create user namespaces and write cgroup v2
limits, both of which Docker denies an unprivileged container. Untrusted code is still confined, by
the namespaces + cgroups + seccomp the sandbox sets up one layer inside.

Assignments are *data*: `./assignments` is mounted read-only, so adding, editing, or re-scoping one
— **including changing its dependencies** — needs a restart, never a rebuild.

## Test it

```bash
./scripts/smoke.sh                    # grade all six example submissions, assert the outcomes
./scripts/submit.sh py-fibonacci python https://github.com/student/hw1   # the real Kafka path
```

`grade-local` (the second binary in the image) runs the real pipeline — merge, sandbox, JUnit parse
— against a directory instead of a GitHub URL, so an assignment can be proven without Kafka or a
push:

```bash
docker compose run --rm grader grade-local mern-todo-api /opt/examples/students/mern-todo-api-bad
```

## Assignments and templates

Three shipped assignments cover both shapes — see [`assignments/README.md`](assignments/README.md)
to author more, and [`examples/README.md`](examples/README.md) for what to hand students:

| Assignment | Stack | Shape | The student submits |
|---|---|---|---|
| `py-fibonacci` | Python / pytest | `solution_files` | one file, `src/solution.py` |
| `mern-todo-api` | JavaScript / Bun | `whole_project` | a project; `tests/` + `package.json` are stamped over |
| `java-bank` | Java / Maven | `whole_project` | a project; `src/test/` + `pom.xml` are stamped over |

Each ships a `-starter` (the handout), a `-good` (reference solution) and a `-bad` (fails some
tests) under `examples/students/`.

## Data contracts

* **Submission** (consumed): `{ submission_id, assignment_id, stack, repo_url, git_ref? }`
  — `stack` accepts `python` | `javascript` (aliases `js`/`mern`/`node`) | `java`.
* **GradeResult** (produced): `{ submission_id, status, passed, failed, skipped, total,
  failing_tests[], error? }` — `status` ∈ `graded` | `fetch_error` | `merge_error` |
  `install_error` | `build_error` | `timeout` | `internal_error`, one per pipeline stage.

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
