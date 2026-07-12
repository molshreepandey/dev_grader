# Assignment: Bank (`java-bank`)

Implement the classes under `src/main/java/com/lamicons/bank/`. Push this repository to **public**
GitHub and submit its URL.

## What you must implement

Balances are **whole cents** (`long`), so there is no floating-point rounding to worry about.

`Account`
- `Account(String id, long openingBalance)`, `id()`, `balance()`
- `deposit(long amount)` — adds to the balance; throws `IllegalArgumentException` when `amount <= 0`
- `withdraw(long amount)` — subtracts from the balance; throws `IllegalArgumentException` when
  `amount <= 0`, and `InsufficientFundsException` when the amount exceeds the balance. A rejected
  withdrawal must leave the balance untouched. Withdrawing the exact balance is allowed.

`Bank`
- `open(String id, long openingBalance)` — registers and returns a new account; throws
  `IllegalArgumentException` when the id is already taken
- `find(String id)` — `Optional<Account>`, empty when unknown
- `transfer(String fromId, String toId, long amount)` — moves money **atomically**: if the source
  cannot cover it, neither balance changes. Unknown account → `IllegalArgumentException`
- `totalAssets()` — the sum of every balance; a transfer never changes it

## How you are graded

Your whole repository is graded, but `pom.xml` and `src/test/` are **stamped over** whatever you
push: your copies are discarded and a hidden JUnit 5 suite is run against your `src/main/`. You
cannot change the dependencies or the build configuration, and adding your own tests under
`src/test/` has no effect on your grade.

Run your own checks locally with:

```bash
mvn test
```
