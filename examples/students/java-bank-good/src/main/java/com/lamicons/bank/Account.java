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
    requirePositive(amount);
    balance += amount;
  }

  public void withdraw(long amount) throws InsufficientFundsException {
    requirePositive(amount);
    if (amount > balance) {
      throw new InsufficientFundsException(
          "account " + id + " holds " + balance + ", cannot withdraw " + amount);
    }
    balance -= amount;
  }

  private static void requirePositive(long amount) {
    if (amount <= 0) {
      throw new IllegalArgumentException("amount must be positive, got " + amount);
    }
  }
}
