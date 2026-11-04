# AGENTS.md

## No self-evident text

Do not add descriptions, notes, hints, legends or tooltips that only repeat what the screen
already shows: the page title, a control's label, its current state, or what an obvious
action does.

Keep text only when the user cannot learn it from the screen, such as:

- errors and how to recover from them
- names for screen readers

## Keep views small

- When a second view needs the same piece of UI, move it to `crates/ui` rather than copy it.
- Put a view's tests in `<view>/tests.rs`.
