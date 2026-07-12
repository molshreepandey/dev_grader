"""A deliberately broken solution, used to exercise the failure-reporting path.

Defects: fib() does not reject negative input, fib_sequence() starts at fib(1), and
sum_even_fibs() sums every term rather than only the even ones.
"""


def fib(n: int) -> int:
    current, following = 0, 1
    for _ in range(n):
        current, following = following, current + following
    return current


def fib_sequence(n: int) -> list[int]:
    return [fib(i) for i in range(1, n + 1)]


def sum_even_fibs(limit: int) -> int:
    total = 0
    current, following = 0, 1
    while current < limit:
        total += current
        current, following = following, current + following
    return total
