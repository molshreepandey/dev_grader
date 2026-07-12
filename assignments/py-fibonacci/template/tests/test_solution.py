"""Hidden tests for py-fibonacci. Students never see this file."""

import pytest

from solution import fib, fib_sequence, sum_even_fibs


def test_fib_base_cases():
    assert fib(0) == 0
    assert fib(1) == 1


@pytest.mark.parametrize(
    ("n", "expected"),
    [(2, 1), (3, 2), (4, 3), (5, 5), (6, 8), (10, 55)],
)
def test_fib_small_values(n, expected):
    assert fib(n) == expected


def test_fib_is_not_exponentially_slow():
    # A naive doubly-recursive fib would take minutes here and trip the sandbox timeout.
    assert fib(60) == 1548008755920


def test_fib_rejects_negative_input():
    with pytest.raises(ValueError):
        fib(-1)


def test_fib_sequence_edge_lengths():
    assert fib_sequence(0) == []
    assert fib_sequence(1) == [0]


def test_fib_sequence_returns_first_n_numbers():
    assert fib_sequence(7) == [0, 1, 1, 2, 3, 5, 8]


def test_sum_even_fibs_below_ten():
    assert sum_even_fibs(10) == 10


def test_sum_even_fibs_with_no_terms():
    assert sum_even_fibs(0) == 0


def test_sum_even_fibs_large_limit():
    assert sum_even_fibs(4_000_000) == 4613732
