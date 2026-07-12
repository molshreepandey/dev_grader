"""Sample tests — a small subset of what the grader checks. Yours to edit; not graded."""

from solution import fib, fib_sequence, sum_even_fibs


def test_fib_base_cases():
    assert fib(0) == 0
    assert fib(1) == 1


def test_fib_sequence():
    assert fib_sequence(7) == [0, 1, 1, 2, 3, 5, 8]


def test_sum_even_fibs():
    assert sum_even_fibs(10) == 10
