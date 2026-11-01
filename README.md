# sycamorefmt

A code formatter for [Sycamore](https://github.com/sycamore-rs/sycamore)'s
`view! { ... }` macro syntax.

## Usage

```sh
cargo install sycamorefmt
sycamorefmt [FILES]
```

With no files, sycamorefmt reads a Rust source file from standard input and
writes the result to standard output. Use `--check` to report files that would
change without writing them, and `--max-width` to control view layout. Run
`sycamorefmt --help` for all options. Use `--fail-on-warnings` to fail whenever
the formatter emits warnings (exit status 2). An input that produces warnings
is not emitted to stdout or written to disk; this applies to stdin and file
inputs, including when combined with `--check`.

**Only view macros are formatted.** The surrounding Rust source is left
unchanged; run `rustfmt` separately if you want to format it too. Neither
installing nor running sycamorefmt requires a `rustfmt` executable. The
`--edition` option is retained for compatibility; the `syn` parser and the
in-process printer do not currently use it to change their behavior.

When formatting files, sycamorefmt writes a temporary file beside the resolved
source file and replaces it only after the write succeeds. Symlinks are followed
and permission bits are retained. Because replacement creates a new file,
other hard links still point to the old contents; ownership, ACLs, extended
attributes, and crash durability of the directory entry are not guaranteed.

## How it works

1. Parse the source with `syn` and locate macro invocations whose paths end in
   `view`, including qualified paths such as `sycamore::view!`.
2. Parse each brace-delimited body with `sycamore-view-parser`, the same parser
   used by `sycamore-macro`. A width-aware printer renders its view AST and
   preserves blank lines between view nodes.
3. Format embedded Rust expressions and match patterns in-process with
   [`prettyplease`](https://github.com/dtolnay/prettyplease), a `syn` AST
   pretty-printer. Views nested in ordinary expression syntax are formatted
   recursively, but the formatter does not inspect the token input to another
   macro.
4. Splice formatted views into the original source using byte spans, without
   rewriting unrelated Rust or its comments.

Like [leptosfmt](https://github.com/bram209/leptosfmt), this uses a
pretty-printer derived from Oppen's algorithm rather than launching a
`rustfmt` process for each embedded expression. Leptosfmt uses its own fork
of prettyplease and a dedicated printer; sycamorefmt uses upstream
prettyplease and its own Sycamore view printer.

A view that cannot be parsed, is not brace-delimited, or contains comments is
left untouched with a warning rather than risking lost source text.

## Known limitations

- Comments inside a `view!` body are not preserved by the view parser, so the
  entire body is left untouched when comments are detected.
- Only brace-delimited invocations are formatted; `view!(...)` and
  `view![...]` are left as-is.
- The view printer uses `max_width` and `tab_spaces`, but prettyplease uses
  its own fixed layout width for embedded Rust. Its wrapping can therefore
  differ from the requested width, especially for deeply nested expressions.
- Rust source whitespace inside an expression, including blank lines around
  nested views, is not preserved by the Rust AST printer. String literal
  contents are preserved.
- Discovery follows Rust syntax parsed into the `syn` AST and the ordinary
  expression ASTs inside Sycamore views; it does not recurse into any macro's
  opaque input tokens. A `view!` inside another macro's tokens—whether that
  outer macro is top-level or appears in an expression—is left untouched,
  because the outer macro may inspect or stringify those tokens. Views are
  skipped with a warning if opaque macro input cannot be preserved safely;
  macro invocations in patterns are currently skipped for this reason.
