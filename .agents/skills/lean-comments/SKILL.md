---
name: lean-comments
description: 'Audits comments and suppressions in a Rust workspace and removes those that add nothing. Run only when explicitly invoked as /lean-comments, optionally with a crate or path. Not for routine code changes, which follow the comment rules in AGENTS.md.'
disable-model-invocation: true
---

# Lean Comments

Audit the commentary of this Rust workspace, or of the crate or path given as an argument. Without
an argument, audit the whole workspace. Edit the working tree and do not commit.

## Core rule

The default is **no comment**. For each comment, ask:

> Would a competent maintainer, reading the code months from now without this conversation or the
> diff, lose meaningful information if it were gone?

If not, delete it. If unsure, delete it unless the repository shows that the information matters.

A comment does not survive for being correct, already present, public, or worded as a warning.
`Never`, `Always`, `Important` and `Security` prove nothing: judge the information.

Decide in this order, and decide necessity before wording:

1. Delete it if the information is already clear
2. Express it through a tiny behavior-preserving change, such as a better local name
3. Shorten it
4. Rewrite it if it is unclear, stale or inaccurate
5. Keep it only if it is necessary, minimal, accurate and durable

Base every kept or rewritten reason on the code, tests, configuration, upstream documentation or
another reliable source. Never invent a reason to save a comment.

## What to keep

- Why surprising code exists, or why a simpler version would be wrong
- External constraints: server, OS and upstream API behavior
- Invariants and non-obvious coupling between items
- Ordering, lifecycle, blocking, concurrency and real-time constraints
- Meaning the type cannot carry:
    - what `None`, an empty collection or a `bool` return means
    - units of a bare number, such as seconds, rem or bytes
    - ordering of a returned `Vec`
    - which of several tuple elements is which
    - error behavior, side effects and what `Drop` does
    - whether a call blocks, and so must stay off the UI thread

## What to delete

- Text that restates a name, signature, type or the next statement
- `/// Returns the name` on `fn name()`, `/// The user's name` on `name: String`
- Docs on plain structs, enum variants and fields that add nothing to the type
- `//!` headers that only say what the crate is called or exports
- Section headings and visual separators
- Change history: `now`, `previously`, `no longer`, `new approach`
- Anything addressed to a reviewer, or that refers to a task, prompt or pull request
- Bare rules with no reason behind them
- Stale `TODO`, `FIXME`, `NOTE`, `HACK` and `XXX` markers. Keep one only if it is still accurate and
  actionable
- Commented-out code, unless it is a deliberate example or a documented fallback

Being `pub` earns no doc comment here: this is an application, and its crates have no outside users.
Do not add docs for coverage, or add `# Errors` and `# Panics` sections unless they say what the
signature cannot.

## Rust forms

- **`//`** explains the implementation. Keep it next to what it explains.
- **`///`** belongs to the item and states its contract. Move a `//` above an item to `///` only if
  callers need it.
- **`//!`** is for a durable boundary, invariant or architectural rule of the crate or module.
- **`// SAFETY:`** on `unsafe` always stays, and must argue why the block is sound.
- **Doc tests:** a code block in a doc comment runs as a test unless it is tagged otherwise, such as
  ```` ```toml ````. Removing one can remove a test.
- **Intra-doc links:** a link such as ``[`Foo`]`` breaks if its target is renamed or removed, and is
  checked by the `cargo doc` command below.

### Doc comments that exist at runtime

A `///` is sometimes program data rather than commentary. Check before deleting or rewording:

- **clap derive:** docs on commands, arguments and `Args` structs are the `--help` output. Treat them
  as user-facing text and keep them.
- **GPUI actions:** `#[derive(Action)]` and `actions!` read docs into `App::action_documentation`.
  They are ordinary comments as long as nothing in the workspace calls it, so search for it first.
- **Macro input:** docs inside a macro call, such as `actions!`, are tokens passed to that macro.

### Suppressions and attributes

Attributes are not prose. Do not change them for style.

- Remove a suppression that the tooling proves unnecessary. To test an `#[allow(...)]`, remove it or
  swap it for `#[expect(...)]`, then run clippy.
- Compare against the lints the workspace enables in `[workspace.lints]`. A `clippy::pedantic` lint
  that nothing enables never fires, so its `#[allow]` does nothing.
- Prefer `#[expect(lint, reason = "...")]` for a suppression that stays, since the compiler reports
  it once it is no longer needed. Do not broaden a suppression.
- Code behind `cfg(windows)` or another `cfg` does not compile on every machine. A clean run on one
  platform proves nothing about suppressions in code it did not build, so leave those alone.
- Keep `#![cfg_attr(...)]`, `#[rustfmt::skip]`, license headers and generated markers intact.

## Style

- **Single-line `//` prose:** as short as it can be without being cryptic, starting uppercase, with
  no closing `.`, `!` or `?`, and no em dashes, en dashes or semicolons between clauses.
- **Multi-line `//`, and `///` or `//!`:** normal sentence punctuation.
- Keep the exact form of identifiers, paths, commands, URLs and values, and write identifiers in
  backticks.
- Prefer one concise line when one is enough. Do not add personality, emphasis or extra rationale.

Examples:

```rust
// Delete: it repeats the code
// Return the normalized result
return normalize(result);

// Keep: the order is an upstream constraint
// `getAlbumList2` pages by offset, so the order must hold between requests

// Keep: the type cannot say what `None` means
/// `None` when its songs have not been fetched yet.
pub fn album(&self, id: &AlbumId) -> Result<Option<AlbumWithSongs>, Error>

// Delete: the declaration already says this
/// A watchlist row.
pub struct WatchlistItem {
    pub media: Media,
    pub added_at: String,
}
```

## Audit steps

1. Back up an uncommitted tree first, so the audit's edits stay separable from what was there:
   `git diff --binary > <scratchpad>/before.patch`
2. Collect every comment in scope with its next line of code, and read all of them. Comments that
   survived an earlier pass are as suspect as any other. Include `tests` modules, and judge test
   comments by whether they explain a scenario or number the test name does not.
3. Also read `#`-comments in `Cargo.toml`, other TOML, `script/` and `.github/`. Version comments on
   pinned actions (`# v7.0.1`) and directives (`# zizmor:`) are machine-read.
4. Skip generated, vendored and third-party content, `.agents/` and data that merely looks like a
   comment, such as strings, fixtures and URLs.
5. Search `#[allow` and `#[expect`, and test each as above.
6. Make the edits. Scripts must assert that each old string matches exactly once.
7. Review the result:
    - No comment line was left duplicated, broken mid-sentence or attached to the wrong item
    - No single-line `//` prose ends in `.`, `!` or `?`
    - `git diff -U0 -- '*.rs'` shows no change to code apart from rustfmt collapsing a macro call or
      enum variant that had held a comment
8. Validate:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links' \
  cargo doc --workspace --no-deps --document-private-items --locked
cargo test --workspace --locked
```

Do not aim for a number of deletions. Stop when further deletion would remove useful information, and
report a small result as small. If almost all comments survive and the changes are mostly wording,
reconsider necessity once more.

Report what changed by kind, what was validated and what was not, including any bug found on the way.
