# JAMA on Raspberry Pi Pico — design document

**Status: design only. Nothing in this document has been built, flashed, or
run on real hardware.** This environment has no physical RP2040/RP2350
board, no debug probe, and no way to observe real USB enumeration, real
flash timing, or real SD card behaviour. Every recommendation below is
grounded in the current (September 2026) state of the RP2040/RP2350 specs
and the embedded-Rust ecosystem, verified against current sources (see
citations), but "compiles for the target" is the only thing that can
actually be checked from here — see [§9](#9-verification-limits--what-i-can-and-cant-check-from-here).

**Confirmed: this is a standalone, battery-powered handheld device** — not
a USB-tethered gadget you type commands into from a laptop. That resolves
what was "open question #3" and reshapes §6 below: physical buttons and a
small display are the primary interface; USB serial becomes a secondary
setup/debug/export channel, not the main way anyone uses the thing.

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

## 2a. Reference hardware for a handheld build

A bare Pico has no display and no buttons — a handheld device needs both,
plus a battery. Rather than wiring discrete components from scratch,
two existing RP2040 boards already combine "buttons + display + battery
connector" and are worth using as-is (or as the reference to clone),
each with a real, opposite trade-off:

| | **Pimoroni Badger 2040** | **Pico + Pimoroni Pico Display Pack** |
|---|---|---|
| Form factor | One integrated board (RP2040 built in) | Pico + a clip-on add-on board (two-part) |
| Display | 2.9" e-ink, 296×128, monochrome | 1.14" IPS LCD, 240×135, 18-bit colour |
| Buttons | 5, along the front edge | 4 (A/B/X/Y) + an RGB LED |
| Refresh feel | Slow (roughly 1–2s for a full refresh; partial refresh is faster but e-ink still isn't snappy) — but **draws power only while refreshing**, near-zero at rest | Fast, backlit, good for a live interactive menu — but the backlight draws power continuously while on |
| Battery | No onboard charging circuit; ships with a 2×AAA holder (swap-and-discard, or add an external LiPo charger like Pimoroni's "LiPo Amigo" yourself) | Whatever you wire in — a Pico has no charging circuit either; typically a LiPo + separate charger module (e.g. a TP4056 board) feeding VSYS |
| Rust support | CircuitPython is Pimoroni's documented path; Rust needs a driver for its e-ink controller written or ported (not yet confirmed to exist as a ready-made no_std crate) | A documented [Rust gist](https://gist.github.com/9names/476e20e055b9fc9a5fdb523068a19290) drives it via `embedded-graphics`, and `pimoroni_gfx_pack` (a crate for Pimoroni's similar GFX Pack product) shows the pattern is established for this product family |

**Neither is a slam-dunk without a trade-off**, and this is a genuine
product-feel decision, not just an engineering one: Badger 2040 feels
like an e-reader/badge — sips power, refreshes slowly; Pico + Display
Pack feels like a snappy little gadget with a bright screen — more
pleasant to navigate quickly, drains its battery faster. **Open question
#6** (§13).

Either way, the RP2040 itself is a real constraint on battery life:
current sources put its lowest achievable sleep current around 180 µA
even in its best "dormant" mode, and dormant mode doesn't support RTC
wake without an external clock source, so a practical low-power sleep
that can still wake on a timer sits closer to a few mA rather than the
sub-µA figures purpose-built low-power MCUs (e.g. an nRF52) achieve. A
handheld device that's mostly idle in a pocket should still expect to be
charged every few days to a couple of weeks, not months — worth setting
that expectation now rather than after a battery disappoints someone.
Source: [RP2040 dormant/sleep current discussion](https://news.ycombinator.com/item?id=40155807), [embassy-rp dormant_sleep docs](https://docs.embassy.dev/embassy-rp/git/rp2040/clocks/fn.dormant_sleep.html).

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
- `embedded-graphics` for on-device rendering (text, simple shapes) — the
  de facto standard `no_std` 2D graphics crate that essentially every
  Rust display driver (LCD or e-ink) targets, plus the specific panel
  driver crate for whichever board is chosen (§2a) — the exact crate
  name depends on the panel controller chip and should be pinned down
  once a board is picked, not guessed here.
- A software debounce for the buttons (a few lines: require N
  consecutive stable reads, or a short timer, before treating a press as
  real) — no crate strictly needed for 4–5 buttons.
- `sequential-storage` for the onboard-flash log backend (§4, Phase 2):
  a log-structured, wear-levelling, corruption-repairing key-value/queue
  store built specifically for this problem, used in production outside
  its own maintainer (Tweede golf) per current docs.
- `embedded-sdmmc` for the SD-card backend (§4, Phase 4): pure-Rust,
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

### Option A — onboard QSPI flash, binary log (Phase 2: no extra hardware)

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

### Option B — SD card, native plain text (Phase 4: needs a breakout board)

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

Phase 2 ships the flash-log backend — fastest path to something real and
testable with zero extra parts (§12). A later phase adds the SD-card
backend as a hardware upgrade for whoever wants the fully-native
plain-text promise and more headroom; the menu UI and double-entry logic
don't change, only which `LedgerStore` impl is wired up. **Open question
#4** (§13): is an SD breakout actually part of your hardware plan, or
should the flash-log approach be treated as the permanent answer, not a
stepping stone?

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
- **Quick-account list**: with no keyboard, typing a colon-separated
  account path button-by-button isn't realistic (see §6's text-entry
  discussion). v0's answer is a small, on-device-editable list of
  pre-declared accounts (`heapless::Vec<AccountEntry, 16>`, say) that
  the button UI scrolls through instead of accepting free text. New
  accounts get added to the list via the USB-serial admin channel (§6),
  which *can* reasonably take typed text since it's a one-time setup
  action on a real keyboard, not a per-transaction one on a 5-button pad.
- **Narration**: v0 does **not** support free-text narration entry on
  the device itself — the same "no realistic on-device typing" problem.
  A transaction's narration defaults to its selected quick-account's own
  label (e.g. picking the "Coffee" quick-account gives narration
  `"Coffee"` for free); true free-text narration, if ever wanted, is a
  later feature entered via the USB-serial admin channel or the
  desktop companion tool (§10), not typed on the handheld.
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

**Primary: buttons + display, no host PC needed.** This is the whole
point of "handheld" — it has to work standalone, pulled out of a pocket,
with no laptop nearby. USB serial (below) exists for setup and recovery,
not day-to-day use.

### Menu-driven UI (primary)

With 4–5 buttons and no keyboard, free text is off the table (see §5), so
the UI is a hierarchical menu navigated with Up/Down/Select/Back
(the exact button count and labels depend on which board — §2a):

```
┌─────────────────┐
│ JAMA             │   Main menu, Up/Down to move, Select to enter
│ > Add            │
│   Balance        │
│   Check          │
│   Settings       │
└─────────────────┘

Add → pick a quick-account (from-side)   [Up/Down through the list]
    → pick a quick-account (to-side)
    → dial in the amount via a digit spinner:
         [ 0 1 2 . 5 0 ]  ← Up/Down changes the highlighted digit,
                             Select moves to the next one, long-press
                             Select on the last digit confirms
    → date defaults to "today" (from the on-chip RTC, §7); Select to
      accept, or Up/Down to nudge it a day at a time for a backdated entry
    → confirmation screen, Select to save
```

`Balance` scrolls the quick-account list showing each one's running
total; `Check` shows a pass/fail summary (transaction count, imbalance
count — the same numbers `jama check` reports on desktop, computed the
same way); `Settings` is where the quick-account list itself gets edited
— either with the same button UI (slow, tedious for typing a new
16-character account name one letter at a time) or, more realistically,
deferred to the USB-serial admin channel below for anything beyond
picking from what's already there.

### USB serial (secondary: setup, recovery, export)

`usbd-serial` (USB CDC-ACM) presents as `/dev/ttyACM0` (Linux/macOS) or a
COM port (Windows) when plugged in — no host driver needed on any current
OS. A small line-buffered command set for the things that genuinely need
a real keyboard or need to move a lot of text:

```
jama> quickadd "Coffee" from=assets:checking to=expenses:cafe
added quick-account "Coffee"
jama> settime 2026-09-23T10:00:00
jama> export
...streams the full ledger as JAMA text-format lines, for the host
     terminal to capture (e.g. `picocom --logfile ledger.beancount`)...
```

No colour, no Unicode box-drawing on this channel — plain ASCII is
sufficient for an occasional setup/export session, and keeping it simple
saves flash. **Open question #7** (§13): should the day-to-day `add`
flow ever be reachable over USB serial too (e.g. for someone who'd rather
type at a desk sometimes), or is button-only intentional?

## 7. Time

Neither RP2040 nor RP2350 has a **battery-backed** real-time clock: the
chip's RTC peripheral free-runs from whatever it was last set to, but its
counter resets like everything else on power loss (no onboard coin-cell
or supercap backup). This matters more now than it did for a
USB-tethered design: a handheld device is *supposed* to be used away from
any host, so "the host sets the time each session" doesn't hold — between
charges, the device may never see a host at all.

Two honest options, not one assumed answer:

1. **v0 minimum**: `settime` over USB serial whenever it happens to be
   plugged in to charge; the on-chip RTC free-runs from there until the
   next power loss (a dead battery, a hard reset). Dates recorded while
   genuinely "wrong" (long unplugged) would need manual correction later
   — acceptable for a first cut, not for daily reliance.
2. **Recommended given standalone use is now primary**: add a
   battery-backed external I2C RTC module (DS3231 or similar, ~$2–5,
   its own coin-cell) so accurate dates survive the main battery being
   fully drained or swapped. This moves from "Phase 4 nice-to-have" (its
   status in the USB-tethered draft of this doc) to "worth doing in
   Phase 1" now that the device has no host to fall back on.

**Open question #8** (§13): is the extra RTC module an acceptable
addition to the bill of materials, or should v0 ship with the
host-sets-time limitation and accept its consequences?

## 8. Command surface for v0

| Feature | Where | v0 | Why |
|---|---|---|---|
| Add a transaction | Button menu | ✅ | Core loop (§6) |
| Recent transactions list | Button menu | ✅ | Core loop |
| Balance (per quick-account) | Button menu | ✅ (the quick-account list, §5 — not an arbitrary pattern) | Core loop |
| Check | Button menu | ✅ | Cheap, high-value: catches an imbalance before it's trusted |
| Export (stream full ledger text) | USB serial | ✅ | The plain-text promise (§4) |
| Add/edit a quick-account | USB serial | ✅ | Needs real text entry — a one-time setup action, not per-transaction (§5/§6) |
| Set time | USB serial | ✅ (v0 minimum, §7) | No host to fall back on otherwise |
| Import a CSV | — | ❌ | No path onto a handheld with no host tethering during normal use |
| Full free-text narration | — | ❌ (later) | Same "no realistic on-device typing" problem as account names (§5) |
| `register`/`networth` | — | ❌ (later) | Needs streaming-account-history machinery beyond v0's quick-account balances |
| `edit` an existing transaction | — | ❌ | No `$EDITOR`, and no realistic on-device text re-entry either |
| Colour, JSON | — | ❌ | No terminal to render colour into; no scripting consumer for JSON on a handheld |

Because `balance` on-device only ever covers the fixed quick-account list
(§5, capped at 16 in the sketch above), its accumulator size is known at
compile time — no unbounded-RAM concern the way an arbitrary full-tree
`balance` would have. A true full-tree balance over every account that's
ever appeared (like desktop's `jama balance` with no pattern) stays a
later feature, reachable via `export` + the desktop companion tool (§10)
in the meantime.

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

**Open question #2** (§13).

## 11. Risk register

| Risk | Mitigation |
|---|---|
| Power loss mid-write corrupts the log | `sequential-storage`'s CRC + repair-on-open; always assemble a full record in RAM before the flash program call, never write partial records |
| Flash wear-out | Personal-ledger write volume is far below typical NOR endurance (~100k erase cycles/sector) over any realistic device lifetime; log-structured writes amortise erases further |
| RAM exhaustion from unbounded input | `heapless` fixed-capacity buffers everywhere; loud, explicit rejection at the limit, never silent truncation |
| USB enumeration quirks on some hosts | CDC-ACM is a standard, driverless device class on current OSes; still needs real cross-OS testing once hardware exists |
| SD card absent, wrong format, or removed mid-write | Mount-check on boot with a clear serial error; never assume presence |
| Battery drains faster than expected (RP2040's best sleep current is ~180 µA, not the sub-µA figures a purpose-built low-power MCU reaches — §2a) | Set the expectation up front (charge every few days to ~two weeks, not months); use dormant/sleep mode between button presses; e-ink (if chosen) draws power only during a refresh |
| Time drifts or resets after the battery fully drains, with no host nearby to `settime` | External battery-backed RTC module (§7) if Q8 confirms it's worth the extra part; otherwise a documented, accepted limitation |
| **Silent data loss** | The one failure mode worse than any of the above for a financial record. Every choice here — loud errors over guesses, full-record-before-write, explicit caps — follows the same principle as the earlier fix that stopped `jama add` from silently assuming SAR: fail loudly, never assume. |

## 12. Suggested implementation phases

0. Toolchain bring-up: blink an LED, confirm `cargo build` → flash →
   run works end to end on your actual hardware.
1. Display + button bring-up: render a static menu, react to button
   presses, no ledger logic yet — confirms the *primary* interaction
   model (not USB serial) works on real hardware before anything is
   built on top of it. USB serial's minimal `settime`/`export` shell can
   come up in parallel since it shares little with the display code.
2. Onboard-flash log backend + the Add/Balance/Check menu flow (§6, §8).
3. Quick-account add/edit over USB serial (§5/§6).
4. *(if Q4 confirms SD hardware)* SD-card plain-text backend behind the
   same `LedgerStore` trait.
5. *(if Q8 confirms it)* External RTC module integration.
6. Power management: dormant/sleep between button presses, wake-on-press,
   real battery-life measurement against §2a's expectations.

## 13. Open questions before any code gets written

1. **Board**: RP2040 (original Pico) or RP2350 (Pico 2)? Sets the target
   triple and the RAM/flash budget in §2.
2. **The existing desktop build** (§10): companion tool, kept-but-frozen,
   or removed?
3. **Debug probe**: do you have (or plan to get) a `probe-rs`-compatible
   debug probe — a second Pico running `debugprobe` firmware works — or
   is iteration BOOTSEL-drag-and-drop only? Affects how smooth the actual
   bring-up loop will be, not the design itself.
4. **SD card**: part of the actual hardware plan, or onboard-flash-only?
   Decides whether Phase 2's flash-log approach (§4) is a stepping stone
   or the permanent answer.
5. ~~Interaction model~~ — **resolved**: standalone, buttons + display
   (§6).
6. **Reference board** (§2a): Badger 2040 (integrated, e-ink, sips power,
   slower to navigate) or Pico + Pico Display Pack (two-part, colour LCD,
   snappier, thirstier)? Or a different board/custom PCB entirely? This
   is as much a feel decision as an engineering one.
7. **USB-serial `add`** (§6): button-only by design, or should the same
   `add` flow also be reachable by typing over USB serial for someone at
   a desk?
8. **RTC module** (§7): worth the extra ~$2–5 part and I2C wiring for
   accurate standalone timekeeping, or accept the host-sets-time
   limitation for v0?

Once these are answered, §12's phases are the concrete plan to start
building against.
