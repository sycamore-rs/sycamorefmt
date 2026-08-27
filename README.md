# sycamorefmt

A code formatter for [Sycamore](https://github.com/sycamore-rs/sycamore)'s
`view! { ... }` macro syntax, built on top of `rustfmt`.

## How it works

1. The whole input file is first run through the real `rustfmt` binary. This
   normalizes all the "plain Rust" parts of the file (imports, function
   signatures, indentation, etc). `rustfmt` cannot understand the `view!`
   grammar, so it leaves the contents of any `view! { ... }` invocation
   byte-for-byte untouched -- but it does correctly indent the line the macro
   call itself sits on.
2. `sycamorefmt` then walks the resulting syntax tree (via `syn::visit::Visit`)
   to find every macro invocation whose path ends in `view` (matching both
   `view!{...}` and qualified paths like `sycamore::view!{...}`).
3. Each invocation's token stream is parsed using the project's own
   [`sycamore-view-parser`](../sycamore/packages/sycamore-view-parser) crate
   (referenced as a local path dependency), producing the same
   `ir::Root`/`Node`/`Prop`/... tree that `sycamore-macro` itself uses for
   code generation.
4. That tree is pretty-printed with a small width-aware printer, and the
   result is spliced back into the file in place of the original tokens,
   using exact byte offsets (`proc_macro2::Span::byte_range()`) so nothing
   outside the macro's braces is disturbed.
5. Rust expressions embedded in the view (dyn-node bodies, prop values) are
   formatted by re-serializing them with `quote!` and running them back
   through the *same* pipeline (steps 1-4) inside a throwaway wrapper
   function. This means a `view! { ... }` macro nested inside an expression
   (e.g. inside an `if`/`else` that each produce a `View`) is found and
   formatted recursively, even though it's invisible to the outer file's
   syntax tree.

Any `view!` body that can't be parsed, that isn't brace-delimited, or that
appears to contain a comment (comments are not currently preserved through
the pretty-printer, so we conservatively leave such bodies untouched) is
skipped with a warning rather than causing the whole run to fail.

## CLI

```
sycamorefmt [OPTIONS] [FILES...]

  With no FILES, reads a single Rust source file from stdin and writes the
  formatted result to stdout. With one or more FILES, formats each file in
  place (unless --check is given).

OPTIONS:
    --check               Don't write any files; exit with a non-zero status
                           if any input would be reformatted.
    --edition <EDITION>    Rust edition to format for (default: 2021).
    --max-width <WIDTH>    Maximum line width (default: 100).
    -h, --help             Print this help message.
```

## Layout

- `src/config.rs` -- shared `Config` (max width, edition, tab size).
- `src/rustfmt.rs` -- invokes the system `rustfmt` binary as a subprocess.
- `src/finder.rs` -- `syn::visit::Visit` pass that locates `view!` macros.
- `src/printer.rs` -- pretty-prints `sycamore_view_parser::ir` trees.
- `src/exprfmt.rs` -- formats embedded Rust expressions (and recurses into
  nested `view!` macros within them).
- `src/format.rs` -- ties the above together into `format_source`.
- `src/main.rs` -- CLI.
- `tests/fixtures/*.input.rs` / `*.expected.rs` -- hand-verified before/after
  pairs used by `tests/format_fixtures.rs`, plus a set of "robustness" cases
  (spread props, directives, hyphenated custom elements, nested `view!`,
  qualified macro paths, ...) that are checked for idempotency and
  re-parseability rather than exact output.

## Known limitations (MVP)

- Comments inside a `view! { ... }` body are not preserved; such bodies are
  detected heuristically (presence of `//` or `/*`) and left untouched
  rather than risking silently dropping them.
- Only brace-delimited macro invocations (`view! { ... }`) are reformatted;
  `view!(...)` / `view![...]` are left as-is.
- Width-fitting for embedded Rust expressions is approximate in deeply
  nested contexts: expressions are formatted via a wrapper function at a
  fixed base indentation and then shifted into place, so `rustfmt`'s own
  line-breaking decisions are made relative to that wrapper's column, not
  the expression's final column. Output is always valid, just occasionally
  not perfectly width-optimal at deep nesting levels.

## Verification status

This was developed and reviewed in a sandbox without a working `cargo`/
`rustfmt` toolchain or network access to crates.io, so it could not be
compiled or test-run here. All API usage (`syn`, `proc-macro2`, the local
`sycamore-view-parser` crate, and the `rustfmt` CLI) was cross-checked
against current documentation/source, and the fixture expectations in
`tests/fixtures/` were derived by manually tracing the implementation
line-by-line. Running `cargo fmt --check`, `cargo build`, and `cargo test` in
an environment with a real toolchain is the recommended next step.
