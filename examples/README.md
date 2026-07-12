# Example submissions

Three per assignment, each a stand-in for a student's GitHub repository:

| Suffix | What it is | Used for |
|---|---|---|
| `-starter` | What you **hand to the student**: stubs, a couple of visible sample tests, and a README stating the contract they must satisfy. | the assignment handout |
| `-good` | A reference solution that passes every hidden test. | `scripts/smoke.sh` |
| `-bad` | A plausible wrong solution that compiles but fails some tests. | proves failures are reported, not just crashes |

```
examples/students/
  py-fibonacci-{starter,good,bad}      # single file: only src/solution.py is taken
  mern-todo-api-{starter,good,bad}     # whole project: tests/ + package.json are stamped over
  java-bank-{starter,good,bad}         # whole project: src/test/ + pom.xml are stamped over
```

## Grading them

`grade-local` runs the real pipeline — merge, sandbox, JUnit parse — against a directory instead of
a GitHub URL, so it needs no Kafka and no network:

```bash
docker compose run --rm grader grade-local py-fibonacci /opt/examples/students/py-fibonacci-good
docker compose run --rm grader grade-local py-fibonacci /opt/examples/students/py-fibonacci-bad
```

The first prints `"status": "graded"` with `failed: 0`; the second, `graded` with the failing test
names listed. `./scripts/smoke.sh` does this for all six and asserts the outcomes.

To exercise the *real* path instead — GitHub fetch included — push a `-starter` copy to a public
repo, implement it, and submit the URL with `./scripts/submit.sh`.

## Handing an assignment out

Copy the `-starter` directory into a fresh repository (drop the `.git` of this one), and give the
student that. They implement, push to public GitHub, and submit the URL. What is *not* in the
starter — the hidden tests — is what they are graded on.
