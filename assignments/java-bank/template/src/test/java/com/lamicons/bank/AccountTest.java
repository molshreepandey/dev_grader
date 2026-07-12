// Hidden tests for java-bank. Students never see this file.
package com.lamicons.bank;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import org.junit.jupiter.api.Test;

class AccountTest {

  @Test
  void opensWithTheGivenBalance() {
    Account account = new Account("a-1", 500);
    assertEquals("a-1", account.id());
    assertEquals(500, account.balance());
  }

  @Test
  void depositAddsToTheBalance() {
    Account account = new Account("a-1", 500);
    account.deposit(250);
    assertEquals(750, account.balance());
  }

  @Test
  void depositRejectsNonPositiveAmounts() {
    Account account = new Account("a-1", 500);
    assertThrows(IllegalArgumentException.class, () -> account.deposit(0));
    assertThrows(IllegalArgumentException.class, () -> account.deposit(-1));
  }

  @Test
  void withdrawSubtractsFromTheBalance() throws InsufficientFundsException {
    Account account = new Account("a-1", 500);
    account.withdraw(200);
    assertEquals(300, account.balance());
  }

  @Test
  void withdrawMayEmptyTheAccountExactly() throws InsufficientFundsException {
    Account account = new Account("a-1", 500);
    account.withdraw(500);
    assertEquals(0, account.balance());
  }

  @Test
  void withdrawRejectsOverdraft() {
    Account account = new Account("a-1", 500);
    assertThrows(InsufficientFundsException.class, () -> account.withdraw(501));
    assertEquals(500, account.balance(), "a rejected withdrawal must not change the balance");
  }

  @Test
  void withdrawRejectsNonPositiveAmounts() {
    Account account = new Account("a-1", 500);
    assertThrows(IllegalArgumentException.class, () -> account.withdraw(0));
  }
}
