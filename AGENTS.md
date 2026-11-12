# AGENTS.md

## Read the guides first

Before writing or reviewing code in `hiphile`, read both guides for the app you work on and follow
them.

The GPUI app in `crates/` follows the gpui-kit guides:

- Coding: `.agents/skills/gpui-kit/references/coding-guides.md`
- UI/UX: `.agents/skills/gpui-kit-design-guides/references/design-guides.md`

The AntiWebUI app in `next/` follows the guides of the move, which are written in Japanese:

- Coding: `docs/antiwebui/coding-guide.md`
- UI/UX: `docs/antiwebui/design-guide.md`

The sections below apply to both.

## No self-evident text

Do not add descriptions, notes, hints, legends or tooltips that only repeat what the screen
already shows: the page title, a control's label, its current state, or what an obvious
action does.

Keep text only when the user cannot learn it from the screen, such as:

- errors and how to recover from them
- names for screen readers

## Comments

The default is no comment, including on public items. Add one only when a maintainer could not
recover the information from the code, names, types or tests, such as:

- why surprising code exists, or why a simpler version would be wrong
- external constraints, invariants, and ordering, lifecycle or concurrency requirements
- contracts the type cannot express: units, special values, side effects

Never invent a reason to justify a comment, and do not mention what changed or what was discussed.
Write the durable state. Prefer clearer code over a comment. Update or remove comments made stale by
a change.

A single-line comment is a short clause that starts uppercase and has no closing punctuation.
Leave tooling directives and machine-read comments as they are.

## Keep views small

- When a second view needs the same piece of UI, move it to `crates/ui` (`next/ui` in `next/`)
  rather than copy it.
- Put a view's tests in `<view>/tests.rs`.

## Before committing

Run what CI runs (`.github/workflows/ci.yml`):

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `script/check-gpui-free`

After a change to `hiphile-core` or a crate under it, also run `script/check-mobile`.

Tests that play through a device are `#[ignore]`d. Run one by its exact name with
`HIPHILE_TEST_DEVICE` set to a loopback device, never the whole `--ignored` set: on a DAC, DoP
plays at full level.
