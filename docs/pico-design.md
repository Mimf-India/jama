# JAMA on Raspberry Pi Pico — design document

**Status: design only. Nothing in this document has been built, flashed, or
run on real hardware.** This environment has no physical RP2040/RP2350
board, no debug probe, and no way to observe real USB enumeration, real
flash timing, or real SD card behaviour. Every recommendation below is
grounded in the current (September 2026) state of the RP2040/RP2350 specs
and the embedded-Rust ecosystem, verified against current sources (see
citations), but "compiles for the target" is the only thing that can
actually be checked from here — see [§10](#10-what-i-can-and-cant-verify-from-here).

## 1. Why this is a different product, not a port

The v0 desktop build (`jama-core`/`jama-cli`, already built, tested, and
pushed to `claude/jama-v0-build-e6yxsh`) is a native binary: SQLite for
storage, `clap` for argument parsing, `std::fs`/`std::process` for the
filesystem and `$EDITOR`, a real terminal with ANSI colour. None of that
exists on a Pico:

| Desktop v0 | Pico reality |
|---|---|
| SQLite (`rusqlite`, bundled C library) | No OS, no filesystem syscalls, no malloc-heavy C library — infeasible |
| `clap` (heap-allocating arg parser) | No heap by default; even with one, a full flag/subcommand parser is a lot of code for very little flash |
| `std::fs` for `jama.db`/`jama.beancount` | No filesystem unless you add one yourself (below) |
| `std::process::Command` for `$EDITOR` | No processes, no editor |
| A real terminal, ANSI colour, `unicode-width` | A raw USB serial byte stream; a terminal emulator on the *host* renders it |
| `rust_decimal` | Needs its own no_std-compatible fixed-point type (see §5) |
| 264 KB–520 KB RAM ceiling, no virtual memory | Every buffer needs a compile-time-checked upper bound |

This means "JAMA on Pico" is a **new `#![no_std]` firmware crate** that
implements the same philosophy — plain text you own, double-entry,
local-only, no network, no lock-in — with its own code, not a
cross-compile of what already exists. The double-entry *rules* (every
transaction sums to zero per commodity, never sum across commodities) and
the *text format's grammar* (see `docs/format.md`) are the shared spec
both implementations should conform to; the Rust code implementing them
can't realistically be shared given `no_std` vs `std`.

## 2. Hardware target

Two boards are in scope, and picking one changes the target triple and
the RAM/flash budget:

| | RP2040 (original Pico) | RP2350 (Pico 2) |
|---|---|---|
| Cores | 2× Cortex-M0+ @ 133 MHz | 2× Cortex-M33 (or 2× Hazard3 RISC-V), @ 150 MHz |
| SRAM | 264 KB | 512 KB |
| On-board flash | 2 MB (external QSPI) | 4 MB (external QSPI) |
| FPU | None | Yes (Cortex-M33 variant) |
| Target triple | `thumbv6m-none-eabi` | `thumbv8m.main-none-eabihf` (Cortex-M33) or a RISC-V triple (Hazard3) |
| Security | — | TrustZone, signed boot, SHA-256 accel, HW TRNG |

RP2350 nearly doubles both RAM and flash for the same $5 price point, which
matters directly for how large a ledger can live in the flash-log backend
(§4) and how much headroom the REPL/USB stack leaves for application data.
**Open question #1** (§13): which board is the actual target?

Sources: [Raspberry Pi Pico 2 / RP2350 specs](https://www.adafruit.com/product/6006), [Tom's Hardware on RP2350](https://www.tomshardware.com/raspberry-pi/raspberry-pi-pico/whats-inside-the-raspberry-pi-pico-2s-rp2350), [RP2040 specifications](https://www.raspberrypi.com/products/rp2040/specifications/).

## 3. Toolchain

- `#![no_std]`, `#![no_main]`, `cortex-m-rt` for the vector table/reset
  handler, `panic-probe` (or `panic-halt` for the simplest possible
  bring-up) for panic handling.
- **HAL: `embassy-rp`**, not the synchronous `rp2040-hal`. Current
  sources describe embassy-rp as implementing both blocking and async
  `embedded-hal`/`embedded-hal-async` traits across the RP2040/RP235x
  family with more consistency between chip families than the
  chip-specific `rp-hal` crates, and it's the actively-recommended path
  for new RP2040/RP2350 projects as of this research. `rp2040-hal` is
  the more mature choice specifically if pairing with RTIC, which isn't
  a requirement here.
- `usb-device` + `usbd-serial` for a USB CDC-ACM virtual serial port —
  standard, mature, no host-side driver needed on Linux/macOS/modern
  Windows.
- `heapless` for every buffer (`String<N>`, `Vec<T, N>`) instead of a
  global allocator — see §5 for why.
- `sequential-storage` for the onboard-flash log backend (§4, Phase 1):
  a log-structured, wear-levelling, corruption-repairing key-value/queue
  store built specifically for this problem, used in production outside
  its own maintainer (Tweede golf) per current docs.
- `embedded-sdmmc` for the SD-card backend (§4, Phase 2): pure-Rust,
  `no_std`, no-alloc FAT16/32 driver; needs a `BlockDevice` impl talking
  SPI to the card (the crate ships one).
- Flashing: the RP2040/2350 ROM bootloader (hold BOOTSEL at power-up)
  exposes a USB mass-storage device you drag a `.uf2` file onto
  (`elf2uf2-rs` converts the build's ELF, typically wired up as the
  `cargo run` runner). The smoother loop for active development is
  `probe-rs` with a debug probe — a second Pico flashed with
  `debugprobe` firmware works as one — giving flash + attach + defmt
  logging in one `cargo run`.

Sources: [embassy-rp](https://github.com/embassy-rs/embassy), [rp-hal releases](https://github.com/rp-rs/rp-hal/releases), [embedded Rust RTIC vs embassy comparison](https://www.willhart.io/post/embedded-rust-options/), [sequential-storage](https://crates.io/crates/sequential-storage), [Sequential-storage — Interrupt blog](https://interrupt.memfault.com/blog/sequential-storage-crate), [embedded-sdmmc-rs](https://github.com/rust-embedded-community/embedded-sdmmc-rs).

## 4. Storage design

The desktop build's "plain text is the source of truth" promise is easy
to keep with a full filesystem; on a microcontroller it's a real design
choice with two credible answers, plus a middle path.

### Option A — onboard QSPI flash, binary log (Phase 1: no extra hardware)

Reserve a region of the onboard flash (e.g. the last 512 KB–1 MB) as an
append-only log of small binary records via `sequential-storage`'s queue
API, rather than hand-rolling flash erase/program logic (NOR flash bits
only go 1→0 on program; getting back to 1 needs a whole-sector — typically
4 KB — erase, and partial/unaligned writes have real gotchas that a
purpose-built crate has already solved and tested).

- **Pro:** zero additional hardware. A bare Pico and a USB cable is
  everything you need to try it.
- **Con:** not human-readable off the device. The plain-text promise is
  kept indirectly, via a `dump`/`export` REPL command that streams the
  log back out **as JAMA's own text-format grammar** over the serial
  console, for the host terminal to capture to a file (e.g.
  `picocom --logfile ledger.beancount`). One extra step, not automatic.
- Flash endurance is a non-issue at personal-ledger write volumes (NOR
  flash is typically rated for roughly 100k erase cycles per sector;
  even daily use for decades doesn't approach that with a log-structured
  writer that only erases a sector when it fills), and
  `sequential-storage` adds CRC + repair-on-open for power-loss safety.

### Option B — SD card, native plain text (Phase 2: needs a breakout board)

Wire a µSD breakout over SPI (4 signal wires + power; ~$3 part, not
present on a bare Pico board) and mount it with `embedded-sdmmc`. Store
the ledger as an actual text file (e.g. `LEDGER.TXT`) in **exactly** the
grammar `docs/format.md` describes — the same dialect the desktop build's
`jama.beancount` uses. Pull the card, read it on any computer: no export
step, because the native on-device format *is* the interchange format.
This is the option that most fully honours the ownership pitch.

- **Pro:** genuinely plain-text-native, much larger practical capacity
  (SD cards are gigabytes, not fractions of a megabyte), higher write
  endurance than raw NOR flash.
- **Con:** extra hardware, extra wiring, a FAT driver that's far less
  battle-tested than a desktop OS's; a partial write on power loss can
  leave a corrupt line or directory entry with less recovery machinery
  than `sequential-storage` provides.
- Computing `balance`/`check` from a plain-text file on a 264 KB-RAM
  device means **streaming**, not slurping the file into a `Vec` the way
  the desktop parser does — a line-at-a-time incremental parser that
  folds running per-account, per-commodity sums as it reads, discarding
  each transaction's text once folded in.

### Recommendation: build both, in phases, behind one trait

```rust
trait LedgerStore {
    fn append(&mut self, txn: &TxnRecord) -> Result<(), StoreError>;
    fn iter(&self) -> impl Iterator<Item = Result<TxnRecord, StoreError>>;
}
```

Phase 1 ships the flash-log backend — fastest path to something real and
testable with zero extra parts. Phase 2 adds the SD-card backend as a
hardware upgrade for whoever wants the fully-native plain-text promise
and more headroom; the REPL and double-entry logic don't change, only
which `LedgerStore` impl is wired up. **Open question #2** (§13): is an
SD breakout actually part of your hardware plan, or should Phase 1's
flash-log approach be treated as the permanent answer, not a stepping
stone?

## 5. Data model for 264 KB–520 KB of RAM

No heap allocator, no `Vec`/`String`/`HashMap` from `alloc` unless
explicitly justified — `heapless`'s fixed-capacity, compile-time-bounded
collections everywhere instead. A tiny heap (`embedded-alloc`) is
possible but trades a real risk (fragmentation/OOM on a device recording
financial history) for convenience this domain shouldn't want.

- **Money**: `struct Money { minor_units: i64, scale: u8 }` — the same
  idea as `rust_decimal`'s internal representation (an integer scaled by
  a power of ten), hand-rolled because `rust_decimal` isn't a realistic
  fit for a no-heap, code-size-constrained target. `i64` at `scale = 2`
  covers ±92 quadrillion currency units — nowhere near a real limit.
  Arithmetic is exact integer math; no floats anywhere, matching the
  desktop build's rule for the same reason (money must never drift).
- **Account names**: `heapless::String<48>` (a generous cap for real
  account paths like `expenses:transport:careem`). An account name that
  doesn't fit is a **loud, explicit error** at input time, never a
  silent truncation — silently truncating a financial account name is
  exactly the kind of failure mode this whole project exists to avoid.
- **Dates**: reuse the desktop `jama-core::date::Date`'s exact algorithm
  (days-since-epoch via the Howard Hinnant civil-calendar functions) —
  pure integer math, no_std-compatible as-is, genuinely shareable logic
  even though the surrounding code isn't.
- **Transaction record**: date (`u32` days-since-epoch), flag (1 byte),
  narration (`heapless::String<64>`), postings
  (`heapless::Vec<Posting, 4>` — caps a transaction at 4 legs, covering
  the overwhelming majority of real personal transactions; a 5th leg is
  a clear rejected-input error, not silent data loss).
- No JSON, no `serde` on-device — `--json` has no real consumer on a
  serial console anyway (§8).

## 6. Interaction model

USB CDC-ACM (`usbd-serial`) presenting as `/dev/ttyACM0` (Linux/macOS) or
a COM port (Windows) once plugged in — no host-side driver install on any
current OS. A line-buffered REPL over that port:

```
jama> add "Coffee" 12.50 from=assets:checking to=expenses:cafe
ok, txn #47
jama> balance assets:checking
assets:checking  -142.50 SAR
jama> list 5
...last 5 transactions...
jama> export
...streams the full ledger as JAMA text-format lines...
```

A few deliberate departures from the desktop CLI's grammar, not
oversights:

- `from=`/`to=` key=value tokens instead of `--from`/`--to` GNU-style
  flags — a full flag parser is unnecessary code size for a small fixed
  grammar typed over a raw serial line; a simple positional +
  `key=value` tokenizer is enough and still legible.
- No colour, no Unicode box-drawing — most terminal programs a user
  would attach with (`screen`, `minicom`, `picocom`, PuTTY) render plain
  ANSI fine if ever wanted later, but it isn't load-bearing for v0.
- Echo, backspace, and a bounded line length (e.g. 128 bytes) need to be
  handled explicitly — a raw serial port doesn't locally echo the way a
  real terminal does.

**Open question #3** (§13): is a USB-tethered serial console the right
interaction model, or is the actual goal a standalone appliance (physical
buttons + a small display, no host PC needed)? The latter is a
substantially larger scope — button debouncing, on-device text entry
without a keyboard, a display driver — and isn't assumed here without
confirmation.

## 7. Time

Neither RP2040 nor RP2350 has a **battery-backed** real-time clock: the
chip's RTC peripheral free-runs from whatever it was last set to, but its
counter resets like everything else on power loss, because the Pico
boards have no onboard coin-cell or supercap backup circuit. v0's honest
answer: the host sets the time once per USB session
(`jama> settime 2026-09-22T10:00:00`) using the connecting computer's own
clock; the on-chip RTC free-runs correctly for the rest of that session.
Unplugged and running standalone (e.g. off a power bank), time is
unknown after the first boot until set again. An external I2C RTC module
(DS3231 or similar, battery-backed, ~$2–5) is the real fix and a natural
Phase 4 hardware addition — flagged here, not silently assumed away.

## 8. Command surface for v0

| Command | v0 | Why |
|---|---|---|
| `add` | ✅ | Core loop |
| `list N` | ✅ | Core loop |
| `balance [account]` | ✅ (single account or a small fixed set tracked at once — see below) | Core loop |
| `check` | ✅ | Cheap, high-value: catches an imbalance before it's trusted |
| `export`/`dump` | ✅ | The plain-text promise, streamed over serial (§4) |
| `import` | ❌ | No path to get a CSV onto a bare Pico without SD + a file-transfer story of its own |
| `register` | ❌ (later) | Needs the same streaming-account-history machinery as `balance --full`; not v0-critical |
| `networth` | ❌ (later) | Same reason |
| `edit` | ❌ | No `$EDITOR` on a microcontroller |
| `--json` | ❌ | No real consumer on a raw serial console |
| colour | ❌ (later, low priority) | Not load-bearing |

A full-tree `balance` (every account, like the desktop's `jama balance`
with no pattern) needs an accumulator sized to the number of distinct
accounts touched, which isn't bounded at compile time the way a single
account's running total is — v0 should probably cap it (e.g. track up to
32 distinct accounts in one `balance` call, erroring clearly past that)
rather than assume unbounded RAM for the accumulator.

## 9. Verification limits — what I can and can't check from here

I can (and, once code exists, would):

- Run `cargo build --target thumbv6m-none-eabi` (or the RP2350 target)
  and confirm it compiles and links.
- Run `cargo test` on host for anything hardware-independent — the
  `Money` type, the double-entry balancing rules, the text-format
  grammar, the command tokenizer — the same way `jama-core`'s existing
  test suite already validates that logic on desktop.
- Review flash/RAM usage from the linked binary's size (`cargo size` /
  `.map` file) against the budgets in §2.

I categorically cannot, in this environment:

- Flash anything to real silicon or observe it boot.
- Confirm USB actually enumerates on a real host OS.
- Confirm flash program/erase timing or wear behaviour against real
  NOR flash.
- Confirm SPI communication with a real SD card.
- Catch a panic, hang, or brownout that only reproduces on hardware.

Any implementation phase therefore needs a real bring-up step — on your
hardware, or via a remote debug-probe workflow if this environment is
ever extended to reach one — before "compiles" becomes "works."

## 10. Relationship to the existing desktop build

`jama-core`/`jama-cli` are fully built, tested (49 tests passing),
benchmarked against their own performance budget, and pushed. Nothing
about this pivot has touched them. Three honest options, not a decision
made on your behalf:

1. **Keep as a companion tool.** The desktop build reads/writes the same
   text-format grammar (`docs/format.md`) the Pico's `export` command
   emits — it could become the "review and back up what the Pico
   captured" half of a two-tier product, with genuine ongoing value.
2. **Keep in the repo, deprioritised.** Frozen where it is; the Pico
   firmware becomes the active development target; no code removed.
3. **Remove it.** Deleting ~6,000 lines of tested, working code is easy
   to do and hard to walk back cleanly — worth a explicit yes before I
   do it, not an inference from "the Pico is the real target."

**Open question #4** (§13).

## 11. Risk register

| Risk | Mitigation |
|---|---|
| Power loss mid-write corrupts the log | `sequential-storage`'s CRC + repair-on-open; always assemble a full record in RAM before the flash program call, never write partial records |
| Flash wear-out | Personal-ledger write volume is far below typical NOR endurance (~100k erase cycles/sector) over any realistic device lifetime; log-structured writes amortise erases further |
| RAM exhaustion from unbounded input | `heapless` fixed-capacity buffers everywhere; loud, explicit rejection at the limit, never silent truncation |
| USB enumeration quirks on some hosts | CDC-ACM is a standard, driverless device class on current OSes; still needs real cross-OS testing once hardware exists |
| SD card absent, wrong format, or removed mid-write | Mount-check on boot with a clear serial error; never assume presence |
| **Silent data loss** | The one failure mode worse than any of the above for a financial record. Every choice here — loud errors over guesses, full-record-before-write, explicit caps — follows the same principle as the earlier fix that stopped `jama add` from silently assuming SAR: fail loudly, never assume. |

## 12. Suggested implementation phases

0. Toolchain bring-up: blink an LED, confirm `cargo build` → flash →
   run works end to end on your actual hardware.
1. USB CDC serial REPL that echoes input, no ledger logic — confirms
   the interaction model on a real host OS before building on top of it.
2. Onboard-flash log backend + `add`/`list`/`balance`/`check`/`export`.
3. *(if Q2 confirms SD hardware)* SD-card plain-text backend behind the
   same `LedgerStore` trait.
4. *(if wanted)* External RTC module integration.
5. *(if Q3 confirms standalone appliance)* Physical buttons + small
   display, replacing or supplementing the serial console.

## 13. Open questions before any code gets written

1. **Board**: RP2040 (original Pico) or RP2350 (Pico 2)? Sets the target
   triple and the RAM/flash budget in §2.
2. **SD card**: part of the actual hardware plan, or bare-Pico-only? Decides
   whether Phase 1's flash-log approach (§4) is a stepping stone or the
   permanent answer.
3. **Interaction model**: USB-tethered serial console (assumed above), or
   a standalone appliance with physical buttons + a display (§6, much
   larger scope)?
4. **The existing desktop build** (§10): companion tool, kept-but-frozen,
   or removed?
5. **Debug probe**: do you have (or plan to get) a `probe-rs`-compatible
   debug probe — a second Pico running `debugprobe` firmware works — or
   is iteration BOOTSEL-drag-and-drop only? Affects how smooth the actual
   bring-up loop will be, not the design itself.

Once these are answered, §12's phases are the concrete plan to start
building against.
