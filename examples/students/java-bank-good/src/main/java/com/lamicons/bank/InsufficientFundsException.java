package com.lamicons.bank;

/** Thrown when a withdrawal or transfer would push an account below zero. */
public class InsufficientFundsException extends Exception {

  public InsufficientFundsException(String message) {
    super(message);
  }
}
