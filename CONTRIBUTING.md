# Contributing to tokcost

Thanks for your interest in tokcost. This document covers what you need to
know to make a change that lands cleanly.

## The one hard rule: zero dependencies

**`[dependencies]` in `Cargo.toml` stays empty.** No `regex`, no `serde`, no
`clap`, no `once_cell` — nothing. This isn't dogma for its own sake: it's
the entire reason the project exists, and it keeps `cargo install` fast,
the binary auditable, and the supply chain a single crate deep.

If a change seems to need a crate, it needs a different design instead.
Things that already exist here rather than as dependencies: a BPE encoder,
a base64 decoder, a JSON serializer, an argument parser, and ANSI escape
handling. `std` is fair game and often has what you want already
(`std::sync::OnceLock`, `std::io::IsTerminal`, `char::is_alphabetic`).

The same rule applies to `[dev-dependencies]`. `scripts/gen_golden_fixtures.py`
uses the real `tiktoken` Python package, but it's a developer tool run by
hand — it is never part of the build or the test run.

## Getting set up

```sh
git clone https://github.com/abhiprd2000/tokcost
cd tokcost
cargo build
cargo test
```

The first build runs `build.rs`, which parses the ~5 MB of canonical
`.tiktoken` rank files in `assets/` and generates compact blobs into
`OUT_DIR`. That takes a moment on a cold build and is cached afterward.

## Before you open a PR

CI runs exactly these three, and they must all be clean:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## Changing the tokenizer

`src/bpe.rs` is the part most worth being careful with, because a subtle
bug there produces *plausible but wrong* numbers rather than an obvious
failure.

Two things to understand before editing it:

1. **Read the module doc first.** It explains why the pretokenizer is
   deliberately *coarser* than tiktoken's real split regexes, and why that's
   safe (no learned merge can span a boundary the real splitter always
   draws, so the final token IDs are identical). It also explains the
   direction that is *not* safe.

2. **Coarser is safe; finer is not.** Merging two pretokens the real
   splitter would separate is provably harmless. Splitting one the real
   splitter would keep whole can suppress a merge and change the output.
   If a change makes the splitter finer anywhere, it needs golden-test
   evidence that real tiktoken agrees.

Any tokenizer change should come with fixture coverage:

```sh
pip install tiktoken
python3 scripts/gen_golden_fixtures.py   # regenerates both fixture files
cargo test --test golden
```

Add your cases to the `CASES` list in the script rather than hand-editing
`tests/fixtures/*.fixtures` — those files are generated from real tiktoken
output and hand-edits would defeat their purpose.

## Updating pricing

`src/pricing.rs` is a dated snapshot, not a live feed — tokcost makes zero
network calls by design. When updating it:

- Take rates from the providers' **official** pricing pages only.
- Update `PRICING_AS_OF` to the date you checked.
- Leave out anything ambiguous. A model that reports `n/a` is honest;
  a model with a guessed price is not. `TK_PRICES` exists to cover gaps.

## Reporting a wrong token count

This is the most valuable kind of bug report. Please include:

- The exact input text (base64 it if whitespace matters)
- The model or encoding you used
- What tokcost returned, and what real `tiktoken` returned

A failing case added to `scripts/gen_golden_fixtures.py` is even better
than a description.

## Style

- `cargo fmt` decides formatting; don't hand-format around it.
- Comments explain *why*, not *what* — the code already says what.
- No new abstractions without a second caller that needs them.
