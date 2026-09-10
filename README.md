# font-scrambler

A small experiment. It converts a font to WOFF2, scrambles the bytes, and has the
browser reconstruct the real font in memory at paint time. The served file is
never a usable font on its own. A tiny JS shim un-scrambles it and registers it
through the `FontFace` API via a Blob URL.

## The idea in one paragraph

A browser's WOFF2 decoder is fixed, so you cannot make it natively unscramble a
custom transform. The only way to serve an obfuscated font is to reconstruct it
in JS first: `fetch` the scrambled bytes, reverse the transform in memory, then
hand a valid WOFF2 `Blob` to `@font-face`. The transform itself (XOR / reverse /
rotate) is trivial CPU, usually well under a millisecond for a subset font, so
the unscramble math is near-free. What it actually costs you is that the font is
loaded by script instead of the browser's preload scanner, so it can't be
discovered as early. That trade-off is exactly what the demo lets you measure.

## Quick start

```sh
# Build the CLI
cargo build --release

# Scramble the bundled Quicksand sample (or any TTF/OTF you have the rights to)
./target/release/font-scrambler scramble samples/Quicksand-Regular.ttf \
  -o samples/output.scram.woff2 --scheme xor --key 0x1234abcd

# Serve the demo. Run this from the PROJECT ROOT, the folder that holds both
# web/ and samples/ (ES modules + fetch need a real origin, not file://)
python3 -m http.server 8080 #fron the folder root
# then open http://localhost:8080/web/
```

A scrambled sample generated from Quicksand (SIL licensed) ships in `samples/`,
so the demo works the moment you serve it. No build step needed to look around.
The demo auto-loads it, shows fetch / unscramble / total timings, and has a
`Load plain (control)` button that loads a normally-served WOFF2 the ordinary
way, so you can compare the overhead directly. The scheme dropdown and key box
are preloaded with the values used to make the sample. Edit the key to anything
else and reload, and reconstruction fails, because the recovered bytes are no
longer valid WOFF2. That is the core demonstration: the key must match.

## How the key works

The scramble is symmetric: the same key both scrambles and unscrambles. The
browser always ends up holding the real key, because it needs it to rebuild the
font.

In this demo the key is typed into the page's input, a stand-in for your own app
supplying it. It is never fetched from a file served next to the font. The CLI
prints the scheme, key and preserve-magic settings it used, so you can copy them
into the demo.

## CLI

```
$ font-scrambler --help

Convert a font to WOFF2 and scramble it for in-browser reconstruction

Usage: font-scrambler <COMMAND>

Commands:
  scramble    Convert TTF/OTF to WOFF2, then scramble the bytes
  descramble  Undo a scramble, writing a valid WOFF2 (for cross-checking the JS)
  verify      Scramble then descramble in memory and assert the font survives
  help        Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

Per-command flags:

```
font-scrambler scramble   <in.ttf|otf> -o <out.woff2> [--scheme xor|rev|swap] [--key 0x..] [--no-preserve-magic]
font-scrambler descramble <in.woff2>   -o <out.woff2> [--scheme ...] [--key ...] [--no-preserve-magic]
font-scrambler verify     <in.ttf|otf>               [--scheme ...] [--key ...] [--no-preserve-magic]
```

- `scramble`: convert to WOFF2, scramble, write the artefact.
- `descramble`: reverse a scramble into a valid WOFF2 (used to make the demo's timing control).
- `verify`: scramble then descramble in memory, assert the bytes round-trip, then decode the WOFF2 to an SFNT and parse it, which proves the font really survives.

`--key` accepts hex (`0x1234abcd`) or decimal. Default key: `0x9e3779b9`.

### Examples

Using the bundled `samples/Quicksand-Regular.ttf`:

```sh
# Scramble the Quicksand source into a demo-ready artefact
./target/release/font-scrambler scramble samples/Quicksand-Regular.ttf \
  -o samples/quicksand.scram.woff2 --scheme xor --key 0x1234abcd

# Reverse it back to a valid WOFF2 (the demo's plain control)
./target/release/font-scrambler descramble samples/quicksand.scram.woff2 \
  -o samples/quicksand.plain.woff2 --scheme xor --key 0x1234abcd

# Prove the round-trip survives against the original source font
./target/release/font-scrambler verify samples/Quicksand-Regular.ttf --scheme xor --key 0x1234abcd

# A different scheme, no key needed
./target/release/font-scrambler scramble samples/Quicksand-Regular.ttf \
  -o samples/quicksand.rev.woff2 --scheme rev
```

### Schemes

All operate on the bytes after the 4-byte `wOF2` signature, which is kept in the
clear by default so the file still reports as WOFF2. Use `--no-preserve-magic` to
scramble it too. Every scheme is reversible, and `xor` and `rev` are their own
inverse.

| `--scheme`    | transform                                                | uses `--key` |
|---------------|----------------------------------------------------------|--------------|
| `xor` (default) | XOR each byte with an xorshift32 keystream seeded by the key | yes |
| `rev`         | reverse the byte region                                  | no |
| `swap`        | rotate the byte region left by half its length           | no |

## Repository layout

```
font-scrambler/
├── Cargo.toml
├── src/
│   ├── main.rs        # CLI: scramble | descramble | verify
│   └── scramble.rs    # the scramble rules, the source of truth for the wire format
├── web/
│   ├── unscramble.js  # hand-mirrored copy of scramble.rs (keep in sync)
│   └── index.html     # browser demo: load a scrambled font, render it, time it
└── samples/           # a scrambled Quicksand sample and its plain control (for the demo)
```

`src/scramble.rs` and `web/unscramble.js` intentionally implement the same
algorithm twice, since Rust produces the file and JS reads it back. If you change
one, change the other and regenerate the sample.

## Development

```sh
cargo test              # Rust: round-trip + keystream determinism
cargo clippy --all-targets
```

## Security model and limitations

- It deters, it does not prevent. The unscrambled font must exist in the browser
  to render, so it can always be captured from memory or by mirroring the public
  JS. Treat this as raising the bar plus a licensing and audit signal, not as DRM.
- A single global key is a single point of failure. Real deployment would use a
  per-tenant or per-face key held server-side and handed to entitled clients over
  an authenticated channel, rotated over time.
- There is a render-path cost and a sync burden. The JS unscrambler becomes part
  of the critical path and must stay bug-for-bug in sync with the encoder.
  Consider generating one from the other rather than hand-mirroring, as done here.

## Licensing

- Code in this repository is MIT licensed. See [LICENSE](LICENSE).
- Sample font: `samples/` bundles `Quicksand-Regular.ttf`, the demo WOFF2s
  derived from it, and the full license. Quicksand is copyright 2011 The
  Quicksand Project Authors, licensed under the
  [SIL Open Font License 1.1](https://scripts.sil.org/OFL); see
  [samples/OFL.txt](samples/OFL.txt). Regenerate the demo with fonts you
  actually have the rights to serve, and do not commit binaries you are not
  licensed to redistribute.