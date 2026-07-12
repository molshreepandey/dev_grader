package com.lamicons.bank;

/** A single account. Balances are whole cents, so there is no floating-point rounding. */
public class Account {

  public Account(String id, long openingBalance) {
    throw new UnsupportedOperationException("not implemented");
  }

  public String id() {
    throw new UnsupportedOperationException("not implemented");
  }

  public long balance() {
    throw new UnsupportedOperationException("not implemented");
  }

  /** Adds {@code amount} to the balance. Rejects a non-positive amount. */
  public void deposit(long amount) {
    throw new UnsupportedOperationException("not implemented");
  }

  /** Removes {@code amount} from the balance. Rejects a non-positive amount or an overdraft. */
  public void withdraw(long amount) throws InsufficientFundsException {
    throw new UnsupportedOperationException("not implemented");
  }
}
