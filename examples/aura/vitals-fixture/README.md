# `vitals-fixture` — one machine, frozen

A `/proc` and `/sys` tree as a Raspberry Pi 4 would present it, so that
`aura-vitals` (Rust) and `aura-vitals-go` (Go) can be run against **the same
facts** and their output compared byte for byte.

`PLAN.md` §112.2 asks for two producers against one endpoint contract, and
`ADR-036` §6.2B is why: *"`provides` is an interface, and two implementations
are what prove it."* An interface is only proven by two implementations that
agree, and agreement is only checkable against fixed input.

## What is in it, and why these numbers

| file | value | chosen because |
| :-- | :-- | :-- |
| `sys/class/thermal/thermal_zone0/temp` | `52412` | an ordinary warm Pi, and a millidegree value whose tenths (`524`) are not a round number |
| `proc/uptime` | `273600.00` | exactly 3 days 4 hours, so the two-unit format is exercised at its widest |
| `proc/meminfo` | 946684 / 422948 kB | a 1 GB Pi 4, and `422948 / 1024` is `413` remainder `36` — an integer division that TRUNCATES, which is the arithmetic the two languages have to agree about |
| `proc/loadavg` | `0.14 0.10 0.09 1/128 4242` | a real shape: two decimals the kernel formatted, and a `running/total` field |
| `proc/sys/kernel/hostname` | `rivet-01` | the board this is aimed at |

## `expected-card.txt` is the contract

**One expectation, two readers.** `aura-vitals`'s `card.rs` and
`aura-vitals-go`'s `card_test.go` both assert against this file rather than
against a string of their own — two suites each carrying a private copy would
prove only that each matches itself.

It is `SPEC-04` §6.3's canonical encoding: the same keys in the same order,
`state` and `role` present even at their defaults, and a trailing newline after
`end`.
