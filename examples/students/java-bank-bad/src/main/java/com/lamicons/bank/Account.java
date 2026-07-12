// A deliberately broken solution, used to exercise the failure-reporting path.
//
// Defects: deposit() and withdraw() accept non-positive amounts, and withdraw() happily
// overdraws the account instead of throwing InsufficientFundsException.
package com.lamicons.bank;

public class Account {

  private final String id;
  private long balance;

  public Account(String id, long openingBalance) {
    this.id = id;
    this.balance = openingBalance;
  }

  public String id() {
    return id;
  }

  public long balance() {
    return balance;
  }

  public void deposit(long amount) {
    balance += amount;
  }

  public void withdraw(long amount) throws InsufficientFundsException {
    balance -= amount;
  }
}
