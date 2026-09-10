//! font-scrambler
//! idea: convert TTF/OTF to WOFF2, then scramble the bytes. The browser never
//! gets a directly usable font file; a tiny JS step (see `web/unscramble.js`)
//! reconstructs the real WOFF2 in memory and registers it via a Blob URL.
//!
//! Subcommands:
//!   scramble  <in.ttf|otf>  -> WOFF2 + scramble  -> out.woff2
//!   descramble <in.woff2>   -> undo scramble      -> valid WOFF2
//!   verify    <in.ttf|otf>  -> prove scramble/descramble round-trips and the
//!                              result decodes to a parseable SFNT

mod scramble;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};

use scramble::{MAGIC_LEN, Scheme, descramble, scramble, to_woff2};

#[derive(Parser)]
#[command(
    name = "font-scrambler",
    about = "Convert a font to WOFF2 and scramble it for in-browser reconstruction",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert TTF/OTF to WOFF2, then scramble the bytes.
    Scramble {
        input: PathBuf,
        /// Output path (default: <input-stem>.scram.woff2).
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = Scheme::Xor)]
        scheme: Scheme,
        /// Keystream seed for --scheme xor (hex `0x..` or decimal).
        #[arg(long, default_value = "0x9e3779b9")]
        key: String,
        /// Also scramble the `wOF2` signature (default keeps it in the clear
        /// so the artefact still reports as WOFF2).
        #[arg(long)]
        no_preserve_magic: bool,
    },
    /// Undo a scramble, writing a valid WOFF2 (for cross-checking the JS).
    Descramble {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = Scheme::Xor)]
        scheme: Scheme,
        #[arg(long, default_value = "0x9e3779b9")]
        key: String,
        #[arg(long)]
        no_preserve_magic: bool,
    },
    /// Scramble then descramble in memory and assert the font survives.
    Verify {
        input: PathBuf,
        #[arg(long, value_enum, default_value_t = Scheme::Xor)]
        scheme: Scheme,
        #[arg(long, default_value = "0x9e3779b9")]
        key: String,
        #[arg(long)]
        no_preserve_magic: bool,
    },
}

/// Parse `--key` as decimal or `0x` hex at the CLI boundary.
fn parse_key(s: &str) -> Result<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).with_context(|| format!("invalid hex key {s:?}"))
    } else {
        s.parse::<u32>().with_context(|| format!("invalid key {s:?}"))
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Scramble {
            input,
            output,
            scheme,
            key,
            no_preserve_magic,
        } => cmd_scramble(&input, output, scheme, parse_key(&key)?, !no_preserve_magic),
        Command::Descramble {
            input,
            output,
            scheme,
            key,
            no_preserve_magic,
        } => cmd_descramble(&input, &output, scheme, parse_key(&key)?, !no_preserve_magic),
        Command::Verify {
            input,
            scheme,
            key,
            no_preserve_magic,
        } => cmd_verify(&input, scheme, parse_key(&key)?, !no_preserve_magic),
    }
}

/// Read the source font, convert to WOFF2 and detect the format via the SFNT
/// magic so a bogus input fails with a clear message.
fn load_woff2(input: &Path) -> Result<Vec<u8>> {
    let raw = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    ensure!(raw.len() > MAGIC_LEN, "{} is too small to be a font", input.display());
    let format = match &raw[..4] {
        [0x00, 0x01, 0x00, 0x00] | b"true" => "TrueType",
        b"OTTO" => "OpenType/CFF",
        other => bail!(
            "{} is not a TTF/OTF (magic {:?}); this tool scrambles WOFF2 produced from a source font",
            input.display(),
            other
        ),
    };
    let woff2 = to_woff2(&raw).with_context(|| format!("converting {} to WOFF2", input.display()))?;
    eprintln!(
        "  {} → WOFF2 ({format}, {} → {} bytes)",
        input.display(),
        raw.len(),
        woff2.len()
    );
    Ok(woff2)
}

fn cmd_scramble(
    input: &Path,
    output: Option<PathBuf>,
    scheme: Scheme,
    key: u32,
    preserve_magic: bool,
) -> Result<()> {
    let mut buf = load_woff2(input)?;
    scramble(&mut buf, scheme, key, preserve_magic);

    let out = output.unwrap_or_else(|| default_output(input));
    std::fs::write(&out, &buf).with_context(|| format!("writing {}", out.display()))?;

    // Print the reconstruction params so they can be typed into the demo UI;
    // the key is never written next to the font file.
    println!("  scrambled → {}", out.display());
    println!(
        "  demo params: scheme={} key=0x{key:08x} preserveMagic={preserve_magic}",
        scheme.as_str()
    );
    Ok(())
}

fn cmd_descramble(
    input: &Path,
    output: &Path,
    scheme: Scheme,
    key: u32,
    preserve_magic: bool,
) -> Result<()> {
    let mut buf = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    descramble(&mut buf, scheme, key, preserve_magic);
    ensure!(
        buf[..MAGIC_LEN] == *b"wOF2",
        "descrambled output is not a WOFF2: wrong --scheme/--key/--no-preserve-magic?"
    );
    // Prove it's a real WOFF2 by decoding it to an SFNT the font engine would see.
    let sfnt = woofwoof::decompress(&buf).context("decoded WOFF2 is malformed")?;
    ttf_parser::Face::parse(&sfnt, 0).context("descrambled font does not parse as SFNT")?;
    std::fs::write(output, &buf).with_context(|| format!("writing {}", output.display()))?;
    println!("  descrambled → {} (valid WOFF2)", output.display());
    Ok(())
}

fn cmd_verify(input: &Path, scheme: Scheme, key: u32, preserve_magic: bool) -> Result<()> {
    let woff2 = load_woff2(input)?;

    let mut scrambled = woff2.clone();
    scramble(&mut scrambled, scheme, key, preserve_magic);
    let mut restored = scrambled.clone();
    descramble(&mut restored, scheme, key, preserve_magic);
    ensure!(restored == woff2, "round-trip FAILED: descramble != original");

    // And the round-tripped bytes must be a genuine WOFF2 that decodes.
    let sfnt = woofwoof::decompress(&restored).context("decoded WOFF2 is malformed")?;
    let face = ttf_parser::Face::parse(&sfnt, 0).context("descrambled font does not parse")?;
    let family = face
        .names()
        .into_iter()
        .find(|n| n.name_id == 1)
        .and_then(|n| n.to_string())
        .unwrap_or_else(|| "?".into());

    let changed = scrambled
        .iter()
        .zip(&woff2)
        .filter(|(a, b)| a != b)
        .count();
    println!("  round-trip OK (scheme={}, key=0x{key:08x})", scheme.as_str());
    println!("  family: {family}");
    println!("  bytes differing after scramble: {changed}/{} ({}%)", woff2.len(),
        changed as f64 / woff2.len() as f64 * 100.0);
    Ok(())
}

fn default_output(input: &Path) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    input.with_file_name(format!("{stem}.scram.woff2"))
}
