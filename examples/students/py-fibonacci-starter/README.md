# Assignment: Sequences (`py-fibonacci`)

Implement the three functions in `src/solution.py`. Push this repository to **public** GitHub and
submit its URL.

## What you must implement

| Function | Contract |
|---|---|
| `fib(n) -> int` | The nth Fibonacci number, `fib(0) == 0`, `fib(1) == 1`. Raises `ValueError` when `n` is negative. Must stay fast for `n` up to at least 60 — a naive doubly-recursive version will time out. |
| `fib_sequence(n) -> list[int]` | The first `n` Fibonacci numbers starting at `fib(0)`; `fib_sequence(0) == []`. |
| `sum_even_fibs(limit) -> int` | Sum of the **even** Fibonacci numbers strictly less than `limit`; `sum_even_fibs(10) == 10`. |

## How you are graded

**Only `src/solution.py` is taken from your repository.** Everything else — the test runner, the
configuration, and a hidden test suite — comes from the instructor's template. The sample tests in
`tests/` are for your own feedback; they are *not* the tests you are graded on, and rewriting or
deleting them changes nothing.

Run the samples locally with:

```bash
python -m pytest
```
