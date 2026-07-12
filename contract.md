# Grading contract

What the autograder consumes, what it produces, and exactly what comes back in every condition —
including the ones where the student's code never runs.

The worker is a pure Kafka consumer/producer; it has no HTTP surface.

```
                 assignment-submission                        assignment-result
  test-engine ─────── Submission ───────▶  grader worker  ─────── GradeResult ───────▶ test-engine
                                     fetch → merge → install → test → parse → grade
```

Grading a submission is five stages, and **each one has its own failure status**, so a student can
be told which step let them down. The two that execute commands — install and test — are two
*separate* sandboxed runs over the same workspace, and they differ in exactly one important way:

| Stage | Runs | Network | Failure ⇒ |
|---|---|---|---|
| fetch | our code | — | `fetch_error` |
| merge | our code | — | `merge_error` |
| **install** | our command, over the project's manifest (`bun install`, `pip install -r`, `mvn dependency:go-offline`) | **yes** | `install_error` |
| **test** | the hidden tests, against the student's code | **no** | `build_error` |
| parse | our code | — | `graded` |

Dependencies are resolved **per submission, at grade time** — nothing is pre-installed in the
image. That is why the install phase is online: it is the only way a project that declares its own
dependencies can get them. It runs *our* command over the manifest, before any student code
executes. The test phase — the one that actually runs untrusted code — has an empty network
namespace: no interface, no route, nothing to phone home to. Both phases share `/work` and `$HOME`,
so everything the install downloaded (`node_modules/`, `.venv/`, `~/.m2/repository`) is right there
when the tests run offline.

| | |
|---|---|
| Consumes | `SUBMISSION_TOPIC` (default `assignment-submission`), value = `Submission` JSON |
| Produces | `RESULT_TOPIC` (default `assignment-result`), value = `GradeResult` JSON, **key = `submission_id`** |
| Offsets | committed manually, *after* the result is produced |

---

## 1. Input — `Submission`

```json
{
  "submission_id": "sub-7f3a91",
  "assignment_id": "py-fibonacci",
  "stack": "python",
  "repo_url": "https://github.com/student/hw1",
  "git_ref": "a1b2c3d"
}
```

| Field | Type | Required | Meaning |
|---|---|---|---|
| `submission_id` | string | yes | Echoed back on the result and used as the result's Kafka key. Make it unique per attempt. |
| `assignment_id` | string | yes | Selects the template, hidden tests and grader config on disk. No `/` or `..`. |
| `stack` | enum | yes | `python` \| `javascript` (aliases: `js`, `mern`, `node`) \| `java`. Informational — the assignment's `grader.json` is what actually drives the run. |
| `repo_url` | string | yes | Public GitHub repository. `https://github.com/o/r`, `…/r.git`, `…/r/tree/main`, `git@github.com:o/r.git` and the bare `github.com/o/r` all parse. **Only `github.com`.** |
| `git_ref` | string | no | Branch, tag or commit SHA. Omitted ⇒ default-branch head. |

**A message that is not a valid `Submission` produces no result at all.** An empty payload or one
that fails to deserialize is logged, its offset is committed, and the message is dropped — there is
no `submission_id` to answer with. The producer is responsible for sending well-formed JSON.

## 2. Output — `GradeResult`

```json
{
  "submission_id": "sub-7f3a91",
  "status": "graded",
  "passed": 12,
  "failed": 2,
  "skipped": 0,
  "total": 14,
  "failing_tests": [
    "POST /todos::rejects an empty title with 400",
    "DELETE /todos/:id::removes the todo and 204s"
  ]
}
```

| Field | Type | Meaning |
|---|---|---|
| `status` | enum | `graded` \| `fetch_error` \| `merge_error` \| `install_error` \| `build_error` \| `timeout` \| `internal_error` |
| `passed` / `failed` / `skipped` / `total` | int | Counted from the test cases in the JUnit XML, never from its summary attributes. `failed` includes both assertion failures and errors (a test that threw). `total` counts every case, skipped included. |
| `failing_tests` | string[] | Qualified names of the failing/errored cases — `ClassName::test_name`, or just `test_name` when the report carries no classname. Student-facing. |
| `error` | string? | Present only when `status != graded`. Absent otherwise (the key is omitted, not null). |

**`status: graded` means the test suite ran to completion — not that the student passed.** A
submission where every test failed is still `graded`, with `failed == total`. Compute the score
from `passed` / `total`; use `status` to decide whether a score exists at all.

For every non-`graded` status the counts are all `0` and `failing_tests` is `[]`.

---

## 3. Every condition, and what comes back

### `graded` — the suite ran

| Condition | Result |
|---|---|
| All tests pass | `status: graded`, `failed: 0`, `failing_tests: []` |
| Some tests fail | `status: graded`, `failed > 0`, the failing names listed |
| Every test fails | `status: graded`, `failed == total` — still a successful grade |
| Tests were skipped | counted in `skipped` and in `total`; skipped is **not** a failure |

### `fetch_error` — the submission never arrived

The student's repository could not be downloaded. Nothing was executed. Safe to retry only if the
cause is transient (the student making the repo public, say).

| Condition | `error` |
|---|---|
| Not a GitHub URL | `unsupported host in url \`https://gitlab.com/a/b\` (only github.com is allowed)` |
| Malformed URL (no repo segment, traversal) | ``not a valid github repository url: `https://github.com/onlyowner` `` |
| Private repo, deleted repo, bad `git_ref`, GitHub down | `http error fetching repo: …` (GitHub answers 404 for a private repo — the student almost always forgot to make it public) |
| Archive > 100 MiB total, or any file > 25 MiB | `archive too large (limit …)` |
| Archive has > 20,000 files | `archive has too many files (limit 20000)` |
| Tarball is corrupt, or contains a path that escapes the extraction root | `malformed archive: …` |

### `merge_error` — the submission is not shaped like the assignment

Nothing was executed: no workspace could even be assembled. The student pushed the wrong thing.

| Condition | `error` |
|---|---|
| A required solution file is missing (`solution_files` mode) | ``student did not provide required file `src/solution.py` `` |
| A solution path is a directory rather than a file | ``solution path `…` is not a regular file`` |
| A solution path is a symlink, or escapes the repo | ``unsafe solution path `…`: …`` |
| The *template* is missing a declared protected path | ``template is missing protected path `src/test` `` — an authoring bug, so this one is reported as `internal_error`, not the student's fault |

### `install_error` — the dependencies could not be installed

The install phase — our command, run over the project's manifest with the network up — exited
non-zero. **No student code has run at all**, so this is never a code failure: it is a broken or
unsatisfiable manifest, or a registry problem on our side.

`error` is ``dependency install failed (exit N); output:`` followed by the tail of what the
installer printed — which is the actual diagnostic, and is meant to be shown to the student:

| Condition | What the output shows |
|---|---|
| A package does not exist, or the version does not | `ERROR: No matching distribution found for …` / `error: package "…" not found` |
| The manifest is malformed | `error: failed to parse package.json` / `Non-parseable POM` |
| Dependencies cannot be resolved together | pip's or bun's resolver conflict message |
| The registry is unreachable, or DNS fails | a connection/timeout error — **our infrastructure**, not the student's; safe to retry |
| The install exceeded 300 s | reported as `timeout`, not `install_error` |

That last-but-one row is the wart worth knowing: a registry outage and a student's typo'd package
name both land in `install_error`. Read the output before blaming the student.

### `build_error` — the code did not compile, or the tests could not be collected

Dependencies installed fine, the tests ran offline — and produced **no report**.

| Condition | `error` |
|---|---|
| Compile error (`javac`, a syntax error at import time) | `no test report produced (the code did not compile, or the tests could not be collected); output:` + the tail of the output |
| A test file imports something that does not exist | same — the output is the student-facing diagnostic |
| The run was OOM-killed (> 2 GiB) | `no test report produced (memory limit exceeded); output: …` |

The rule is mechanical: **no JUnit XML at the expected path ⇒ `build_error`.** A test runner that
starts always writes a report, even when every test fails, so a missing report means it never got
that far. Any report file left behind by the install phase is **deleted before the tests run**, so
a report that exists afterwards can only have been written by our test command (see §6).

### `timeout` — a phase exceeded its budget

The whole cgroup is killed. Which phase blew the budget is in `error`:

| | Budget | `error` |
|---|---|---|
| install | 300 s wall clock (a cold Maven or npm resolve is slow, and it is *our* command) | `dependency install exceeded its time limit` |
| test | 120 s wall clock, 120 s CPU | `tests exceeded the wall-clock limit` |

An infinite loop in the student's code, or a test that awaits a promise that never resolves, lands
in the second row.

### `internal_error` — our fault, retry is safe

Never the student's fault. Re-deliver the same submission once the cause is fixed.

| Condition | `error` |
|---|---|
| Unknown `assignment_id`, or `grader.json` missing | `read /opt/assignments/<id>/grader.json: No such file or directory` |
| `grader.json` is malformed | `parse grader.json: …` |
| `assignment_id` contains `/` or `..` | ``invalid assignment id `…` `` |
| The template directory is missing | `template dir missing: …` |
| The template lacks a declared protected path | ``template is missing protected path `src/test` `` — an authoring bug |
| The sandbox could not start (no rootfs for the stack, cgroups unwritable, container not privileged) | `failed to prepare sandbox: …` / `failed to spawn sandboxed process: …` |
| The grading thread panicked | `grader panicked` |

---

## 4. What the two phases run inside

Both phases get the same sandbox: the merged project bind-mounted **read-write at `/work`** (the
working directory) inside a per-stack rootfs mounted **read-only**, in fresh user, PID, IPC, UTS
and mount namespaces, under a cgroup, behind a default-deny seccomp filter. They differ in two
rows of this table, and those two rows are the whole design:

| | install | test |
|---|---|---|
| **Network** | **Yes** — the host's network namespace. Dependencies are downloaded here, by *our* command, before any student code runs. | **No** — an empty network namespace: no interface, no route, no DNS. This is the phase that executes untrusted code. |
| **Wall clock** | 300 s | 120 s, plus `RLIMIT_CPU` 120 s as a busy-loop backstop |
| Writable | `/work` (the project), `$HOME` (`/home/grader`, per submission), `/tmp` (512 MiB tmpfs) — **shared with the other phase**, which is how the downloaded packages get there | same |
| Read-only | `/usr`, `/etc`, `/opt`, and the rest of the rootfs: the toolchain only (python+pip, bun, jdk+maven) | same |
| Memory | 2048 MiB (cgroup `memory.max`, swap off). Exceeding it ⇒ OOM kill | same |
| CPU / processes / file size | 2 cores (`cpu.max`), 512 pids, 256 MiB `RLIMIT_FSIZE` | same |
| Syscalls | Allowlist. `ptrace`, `mount`, `pivot_root`, `chroot`, `setns`, `unshare`, `bpf`, `keyctl`, `perf_event_open`, `setuid`/`setgid` are denied (`ENOSYS`). | same |
| Environment | `PATH`, `HOME=/home/grader`, `TMPDIR=/tmp`. Nothing else — no secrets are reachable. | same |

**What the install phase's network access does and does not mean.** It runs `bun install` /
`pip install -r` / `mvn dependency:go-offline` — our command, over a manifest — and a package's
install scripts (an npm `postinstall`) do execute during it, with network. That is the same trust
model as any CI runner: it is contained by the sandbox (no privileges, no host filesystem, its own
cgroup), but it is not airtight against a hostile *dependency*. It is airtight against hostile
*student code*, which only ever runs in the test phase, offline. If an assignment's manifest is a
protected path, the student cannot choose the dependencies at all.

Consequences worth knowing when authoring an assignment: during the tests there is no network, so
a test may not open a socket to a real server (bind a listener on `lo` and it will fail — the
interface is down) and may not reach a database or the internet.

## 5. Assignment layout — the third contract

`ASSIGNMENTS_ROOT/<assignment_id>/`:

```
grader.json     # StackConfig: stack, merge mode, install, test, report
template/       # the trusted tree: hidden tests, locked build config, stubs
```

```json
{
  "stack": "python",
  "merge": { "mode": "solution_files", "files": ["src/solution.py"] },
  "install": ["/bin/sh", "-c", "python3 -m venv .venv && .venv/bin/pip install --no-input -r requirements.txt"],
  "test": [".venv/bin/pytest", "--junitxml=report.xml", "-q"],
  "report": { "file": "report.xml" }
}
```

| Key | Meaning |
|---|---|
| `merge.mode` | `solution_files` — the template is the base and **only** the listed `files` are taken from the student. `whole_project` — the student's repo is the base and the template's `protected_paths` are stamped on top, always winning (their version of each is deleted first). |
| `install` | argv, run in `/work` **with network**, before any student code. Empty = skip the phase. A non-zero exit ⇒ `install_error`. |
| `test` | argv, run in `/work` **without network**; must emit JUnit XML. The pipeline runs **this**, never a script from the student's repo. |
| `report` | `{"file": "report.xml"}` or `{"glob": "target/surefire-reports/*.xml"}` (surefire writes one file per class; they are merged). |

Each is an `argv` array wrapped in its own `/bin/sh -c "cd /work && …"`, and every argument is
shell-quoted — a filename with a space (or a `;`) in it cannot become a second command. Use the
`["/bin/sh", "-c", "a && b"]` form when a phase genuinely needs two steps.

The three shipped recipes, as worked examples:

| Stack | `install` (online) | `test` (offline) |
|---|---|---|
| Python | `python3 -m venv .venv && .venv/bin/pip install -r requirements.txt` | `.venv/bin/pytest --junitxml=report.xml -q` |
| JavaScript | `bun install` | `bun test tests --reporter=junit --reporter-outfile=report.xml` |
| Java | `mvn -B -q dependency:go-offline` | `mvn -o -q test` |

The venv and `node_modules/` land in `/work`; Maven's repository lands in `$HOME/.m2`. Both are
writable in the install phase and still there, unchanged, when the tests run offline.

Two Java-specific gotchas, both already handled in the shipped `pom.xml`: surefire resolves its
JUnit **provider** and the **platform launcher** lazily, at test time — which here is the *offline*
phase — so both are declared explicitly in the pom, which is what makes `dependency:go-offline`
fetch them while the network is still up.

**Adding, editing, or re-scoping an assignment — including changing its dependencies — needs only a
restart.** Nothing about an assignment is compiled into the image.

## 6. Anti-cheat, precisely

* The hidden tests and the build/test configuration always come from the template. In
  `solution_files` mode the student contributes literally nothing else; in `whole_project` mode the
  `protected_paths` are removed from their tree and replaced by the template's copies.
* The pipeline runs its own `test` command, so rewriting `package.json`'s `test` script, a
  `conftest.py`, or a surefire `<skipTests>` changes nothing.
* Symlinks in the student's tree are never copied or followed, and `.git` is dropped, so a solution
  path cannot be redirected at a file outside the repo.
* **Student code never has network access.** Only the install phase is online, and it runs before
  any student code, so results cannot be phoned in from elsewhere.
* **A report left behind by the install phase is deleted before the tests run.** Otherwise a
  dependency's `postinstall` script could plant a `report.xml` full of passing tests and let a
  failing test phase leave it standing.
* Whether a student may choose their own dependencies is the assignment author's call: keep the
  manifest (`package.json`, `pom.xml`, `requirements.txt`) in `protected_paths` and they cannot —
  the instructor's manifest is what gets installed. Take it out and they can, at the cost of
  handing them control of the build.
* One gap to know about: in `whole_project` mode a student can add *extra* test files outside the
  protected directory. They cannot remove or edit the hidden tests, so they cannot hide a failure —
  but they can inflate `passed`/`total`. Score against the hidden suite's known case count, or keep
  the test command scoped to the protected directory (as `mern-todo-api` does with `bun test tests`).

## 7. Delivery semantics

* Offsets are committed manually, after the result is produced: a crash mid-grade re-delivers the
  submission, so grading is **at-least-once**. A consumer of `assignment-result` must be
  idempotent on `submission_id` (the Kafka key).
* Results are keyed by `submission_id`, so all attempts for one submission land on one partition and
  stay ordered.
* Submissions are graded concurrently up to `GRADER_CONCURRENCY`; results are therefore **not**
  ordered relative to submissions. A 2-second Python grade will overtake a 90-second Maven one.
* Known caveat: if producing the result fails (broker down for > 5 s), the failure is logged and the
  offset is still committed — that result is lost rather than retried. Worth hardening before you
  rely on it for high-stakes exams.
