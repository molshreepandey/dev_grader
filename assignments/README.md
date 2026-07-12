# Assignments

One directory per assignment; the directory name **is** the `assignment_id` a submission refers to.
This whole tree is mounted read-only into the worker at `ASSIGNMENTS_ROOT` (`/opt/assignments`), so
adding or editing an assignment needs a restart, not a rebuild — see [the caveat](#dependencies)
below.

```
<assignment_id>/
  grader.json     # StackConfig — how to merge, install, test, and where the JUnit XML lands
  template/       # the trusted tree: hidden tests, locked build config, stubs
```

The three shipped assignments are also the worked examples of the two assignment shapes:

| Assignment | Stack | Shape | The student submits |
|---|---|---|---|
| `py-fibonacci` | Python / pytest | `solution_files` | one file, `src/solution.py` |
| `mern-todo-api` | JavaScript / Bun | `whole_project` | a whole project; `tests/` + `package.json` are stamped over |
| `java-bank` | Java / Maven | `whole_project` | a whole project; `src/test/` + `pom.xml` are stamped over |

`grader.json` is the serialized `StackConfig`; `contract.md` §5 documents every key, and
`StackConfig::default_for` in `crates/grader-types/src/config.rs` is the copy-paste starting point
for each stack.

## Authoring a new one

1. **Write the template.** Everything the student does *not* write: the hidden tests, the build
   config, and — for `solution_files` — a stub of each file they must implement, with the
   signatures the tests call.
2. **Write `grader.json`.** Pick the shape: `solution_files` when they implement declared files
   inside your scaffold, `whole_project` when they build the project themselves and you protect the
   tests and build config.
3. **Write a reference solution** under `examples/students/<assignment_id>-good/` and a broken one
   under `-bad/`. These are not decoration: `scripts/smoke.sh` grades all of them to prove the
   assignment actually works before a student ever sees it.
4. **Write the starter repo** under `examples/students/<assignment_id>-starter/` — the stubs, a
   couple of *visible* sample tests, and a README stating the contract. This is what you hand out.
5. `docker compose build && ./scripts/smoke.sh`.

## Dependencies

Dependencies are installed **per submission, at grade time**, by the sandbox's online install
phase — nothing is pre-installed in the image, so each assignment brings whatever it needs and
changing that needs no rebuild:

| Stack | Declared in | `install` (online) | Lands in |
|---|---|---|---|
| Python | `template/requirements.txt` (must include pytest) | `python3 -m venv .venv && .venv/bin/pip install -r requirements.txt` | `/work/.venv` |
| JavaScript | `template/package.json` | `bun install` | `/work/node_modules` |
| Java | `template/pom.xml` | `mvn -B -q dependency:go-offline` | `$HOME/.m2/repository` |

The tests then run **offline**, over what the install left behind. Two consequences for authors:

* **Everything the tests need must be resolvable during install.** Maven is the trap: surefire
  picks its JUnit provider and the platform launcher lazily, *at test time* — i.e. offline. The
  shipped `pom.xml` declares `junit-platform-launcher` and `surefire-junit-platform` explicitly for
  exactly this reason. Copy that pom rather than writing one from scratch.
* **Whether the student may choose the dependencies is your call.** Keep the manifest in
  `protected_paths` (as all three shipped assignments do) and they get yours. Take it out and they
  can add libraries — at the cost of handing them control of the build, which is what stops them
  from disabling your tests.

## Writing tests that can actually run

Tests run with **no network** (only the install phase is online) and no reachable database, and the
loopback interface is down — so a test may not start an HTTP server and call it over a socket. Test the handler
directly instead: `mern-todo-api` exercises the API by passing a `Request` to `app.fetch()` and
inspecting the `Response`, which is both faster and sandbox-safe.

Whatever the runner, it must emit JUnit XML — pytest (`--junitxml`), `bun test --reporter=junit`,
and Maven surefire all do, which is why one parser serves all three.
