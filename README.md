# sycamorefmt

A code formatter for [Sycamore](https://github.com/sycamore-rs/sycamore)'s
`view! { ... }` macro syntax, built on top of `rustfmt`.

## Usage

```sh
cargo install sycamorefmt
sycamorefmt [FILES]
```

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
3. Each invocation's token stream is parsed using the `sycamore-view-parser`
   crate, producing the same `ir::Root`/`Node`/`Prop`/... tree that
   `sycamore-macro` itself uses for code generation.
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

## Known limitations

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
