package com.lamicons.bank;

import java.util.Optional;

/** A collection of accounts, keyed by id. */
public class Bank {

  /** Opens a new account. Rejects an id that is already taken. */
  public Account open(String id, long openingBalance) {
    throw new UnsupportedOperationException("not implemented");
  }

  public Optional<Account> find(String id) {
    throw new UnsupportedOperationException("not implemented");
  }

  /** Moves money between two accounts, atomically. */
  public void transfer(String fromId, String toId, long amount) throws InsufficientFundsException {
    throw new UnsupportedOperationException("not implemented");
  }

  /** The sum of every account balance. */
  public long totalAssets() {
    throw new UnsupportedOperationException("not implemented");
  }
}
