// Hidden tests for java-bank. Students never see this file.
package com.lamicons.bank;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Optional;
import org.junit.jupiter.api.Test;

class BankTest {

  @Test
  void openRegistersAnAccountThatCanBeFound() {
    Bank bank = new Bank();
    Account opened = bank.open("a-1", 100);

    Optional<Account> found = bank.find("a-1");
    assertTrue(found.isPresent());
    assertEquals(opened.id(), found.get().id());
    assertEquals(100, found.get().balance());
  }

  @Test
  void findReturnsEmptyForAnUnknownAccount() {
    Bank bank = new Bank();
    assertEquals(Optional.empty(), bank.find("nope"));
  }

  @Test
  void openRejectsADuplicateId() {
    Bank bank = new Bank();
    bank.open("a-1", 100);
    assertThrows(IllegalArgumentException.class, () -> bank.open("a-1", 50));
  }

  @Test
  void transferMovesMoneyBetweenAccounts() throws InsufficientFundsException {
    Bank bank = new Bank();
    bank.open("a-1", 500);
    bank.open("a-2", 100);

    bank.transfer("a-1", "a-2", 200);

    assertEquals(300, bank.find("a-1").orElseThrow().balance());
    assertEquals(300, bank.find("a-2").orElseThrow().balance());
  }

  @Test
  void transferRejectsAnOverdraftAndLeavesBothBalancesUntouched() {
    Bank bank = new Bank();
    bank.open("a-1", 100);
    bank.open("a-2", 100);

    assertThrows(InsufficientFundsException.class, () -> bank.transfer("a-1", "a-2", 101));

    assertEquals(100, bank.find("a-1").orElseThrow().balance());
    assertEquals(100, bank.find("a-2").orElseThrow().balance());
  }

  @Test
  void transferRejectsAnUnknownAccount() {
    Bank bank = new Bank();
    bank.open("a-1", 100);
    assertThrows(IllegalArgumentException.class, () -> bank.transfer("a-1", "ghost", 10));
  }

  @Test
  void totalAssetsSumsEveryBalanceAndIsConservedByATransfer() throws InsufficientFundsException {
    Bank bank = new Bank();
    bank.open("a-1", 500);
    bank.open("a-2", 100);
    assertEquals(600, bank.totalAssets());

    bank.transfer("a-1", "a-2", 250);
    assertEquals(600, bank.totalAssets());
  }

  @Test
  void totalAssetsOfAnEmptyBankIsZero() {
    assertEquals(0, new Bank().totalAssets());
  }
}
