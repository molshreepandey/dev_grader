// Defect: open() overwrites an existing account instead of rejecting a duplicate id.
package com.lamicons.bank;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Optional;

public class Bank {

  private final Map<String, Account> accounts = new LinkedHashMap<>();

  public Account open(String id, long openingBalance) {
    Account account = new Account(id, openingBalance);
    accounts.put(id, account);
    return account;
  }

  public Optional<Account> find(String id) {
    return Optional.ofNullable(accounts.get(id));
  }

  public void transfer(String fromId, String toId, long amount) throws InsufficientFundsException {
    Account from = require(fromId);
    Account to = require(toId);
    from.withdraw(amount);
    to.deposit(amount);
  }

  public long totalAssets() {
    return accounts.values().stream().mapToLong(Account::balance).sum();
  }

  private Account require(String id) {
    return find(id).orElseThrow(() -> new IllegalArgumentException("no such account: " + id));
  }
}
