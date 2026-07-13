# lamicons-dev-engine

Kafka worker that grades a student's assignment repo against a hidden test repo,
inside a sandbox, and reports a JUnit-derived score.

It is the assignment counterpart of `lamicons-code-engine`: that one runs a
single untrusted snippet under a strict seccomp profile, this one has to
*install dependencies and build a real project*, so it trades the syscall filter
for namespaces + cgroups and keeps the network.

## Flow

```
grader-submission (Kafka)
        │
        ├─ clone   student repo  ──┐  git clone --depth 1, http(s) only, size-capped
        │  clone   hidden tests  ──┘
        ├─ merge   purge the student's test dir, overlay the hidden tests on top
        ├─ install bun install / pip install --user / mvn test-compile
        ├─ test    bun test / pytest / mvn test          ← both run in the sandbox
        ├─ report  parse the JUnit XML every runner emits
        └─ produce result → grader-result (Kafka), then commit the offset
```

The workspace and its cgroups are torn down on every exit path, success or not.

### Merge, not `git merge`

The two repos share no history, so there is nothing to merge: the hidden test
repo is copied file-by-file over the student's checkout. First the student's own
test tree is **deleted** — `tests/` for JS and Python, `src/test/` for Java — so
a student who edits or deletes the tests still gets ours. Symlinks are never
followed during the copy: a repo containing `tests -> /etc` would otherwise
redirect our writes onto the host.

## Payload contract

Consume from `SUBMISSION_TOPIC` (default `grader-submission`):

```json
{
  "task_id": "task-001",
  "task_language": "javascript",
  "student_link": "https://github.com/pankajyadav-blitz/node-code.git",
  "test_link": "https://github.com/pankajyadav-blitz/node-test.git",
  "student_id": "test-student-998"
}
```

`task_language` is one of `javascript` (aliases `js`, `node`), `py` (alias
`python`), `java`. Both links must be `http(s)` — an ssh URL is rejected before
anything runs.

Produce to `RESULT_TOPIC` (default `grader-result`), keyed by `task_id`:

```json
{
  "task_id": "task-001",
  "student_id": "test-student-998",
  "status": "partially_passed",
  "stage": "report",
  "total_testcases": 9,
  "passed_testcases": 8,
  "failed_testcases": 1,
  "skipped_testcases": 0,
  "score": 88.89,
  "duration_ms": 10245,
  "testcases": [
    { "name": "handles a single element", "suite": "average — scale and edge cases",
      "outcome": "passed", "duration_ms": 5 },
    { "name": "handles negative and fractional values", "suite": "average — scale and edge cases",
      "outcome": "failed", "duration_ms": 1, "message": "AssertionError" }
  ]
}
```

`status` is the whole error contract:

| status | meaning |
|---|---|
| `passed` / `partially_passed` / `failed` | the suite ran; the counts are the grade |
| `clone_failed` | bad, private or oversized repo |
| `merge_failed` | the checkout's layout could not be combined |
| `install_failed` | dependencies or compilation failed — the suite never ran |
| `run_failed` | the runner crashed, wrote no report, or discovered no tests |
| `timeout` / `memory_limit_exceeded` | a stage hit its cgroup or wall-clock limit |
| `internal_error` | our side broke (cgroup, namespace, disk) |

Only the first three mean "this is the student's grade"; the rest carry `error`
and a `logs` tail (last 8 KB of the stage's output) instead. Skipped tests leave
the denominator, so they never cost a student marks. The result is produced
*before* the Kafka offset is committed, so a crash replays the task rather than
losing it.

## Isolation

Both the install and the test stage run under:

- **Namespaces** — user, pid, mount, ipc, uts. The workspace is `pivot_root`ed
  to `/`, with `/usr`, `/bin`, `/lib`, `/etc` bind-mounted **read-only** from the
  image, so the build sees the toolchain but cannot modify it. There is
  deliberately **no network namespace**: `bun install`, `pip` and `mvn` have to
  reach their registries. The user namespace still denies `CAP_NET_ADMIN`, so the
  run can use the network but not reconfigure it.
- **cgroup v2** — `memory.max`, `memory.swap.max=0`, `cpu.max`, `pids.max` per
  task. Teardown kills the cgroup, waits for it to actually drain, and only then
  `rmdir`s it: removing a populated cgroup returns `EBUSY`, and skipping the wait
  leaks the directory forever.
- **No seccomp filter**, unlike the judge. A build legitimately needs sockets,
  threads and subprocesses. `PR_SET_NO_NEW_PRIVS` still blocks setuid escalation.

Everything lives on **disk** (`/var/lib/grader`), never tmpfs — a `node_modules`
or `.m2` tree would eat the machine's RAM. The only tmpfs is a 64 MB `/dev/shm`
the JVM wants.

The container is the outer trust boundary: it runs `privileged` because
`mount()` and cgroup writes require it, and Docker's default seccomp/AppArmor
profiles block them. Run it on a host dedicated to grading.

## Running

```bash
cargo build --release
docker compose --profile local up --build    # grader + a local Kafka
docker compose up grader                     # production: KAFKA_BROKERS → real cluster
```

Grade one task without Kafka in the loop — same pipeline, same sandbox:

```bash
docker compose run --rm grader grade_once '{"task_id":"t1","task_language":"javascript", ...}'
```

On a dev box without root, set `GRADER_ISOLATION=false` to skip namespaces and
cgroups. Student code then runs with your own privileges — never do this on a
shared or production machine.

See `.env.example` for every knob (concurrency, per-task memory/cpu/pid caps,
per-stage timeouts, repo size cap).

## Layout

| Crate | Role |
|---|---|
| `app/grader` | the worker binary (`grader`) and the debug binary (`grade_once`) |
| `crates/grader_lib` | `scheduler` (the pipeline), `workspace` (clone + merge), `sandbox` (namespaces + pivot_root), `cgroups`, `languages` (the per-language commands), `junit` (the parser) |
| `crates/kafka_types` | consumer/producer setup, offset commit |
| `crates/task_types` | the two wire payloads |

Adding a language is one entry in `crates/grader_lib/src/languages.rs`: what to
purge, how to install, how to test, and where its JUnit XML lands.
