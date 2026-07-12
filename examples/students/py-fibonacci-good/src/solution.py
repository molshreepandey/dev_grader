"""A reference solution: iterative, so fib(60) returns instantly."""


def fib(n: int) -> int:
    if n < 0:
        raise ValueError("n must be non-negative")
    current, following = 0, 1
    for _ in range(n):
        current, following = following, current + following
    return current


def fib_sequence(n: int) -> list[int]:
    sequence = []
    current, following = 0, 1
    for _ in range(n):
        sequence.append(current)
        current, following = following, current + following
    return sequence


def sum_even_fibs(limit: int) -> int:
    total = 0
    current, following = 0, 1
    while current < limit:
        if current % 2 == 0:
            total += current
        current, following = following, current + following
    return total
