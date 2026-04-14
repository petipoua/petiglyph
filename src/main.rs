use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "petiglyph",
    version,
    about = "Convert SVG icon sets into terminal-usable font glyphs."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a starter manifest for an icon set.
    Init {
        /// Output manifest path.
        #[arg(short, long, default_value = "petiglyph.toml")]
        output: PathBuf,
    },
    /// Build a font from SVG sources and a manifest.
    BuildFont {
        /// Path to the manifest file.
        #[arg(short, long, default_value = "petiglyph.toml")]
        manifest: PathBuf,
        /// Output directory for generated artifacts.
        #[arg(short, long, default_value = "dist")]
        out_dir: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { output } => init_manifest(output),
        Command::BuildFont { manifest, out_dir } => build_font(manifest, out_dir),
    }
}

fn init_manifest(output: PathBuf) -> Result<()> {
    println!("petiglyph init");
    println!("  output: {}", output.display());
    println!("  status: scaffolding only (manifest writing not implemented yet)");
    Ok(())
}

fn build_font(manifest: PathBuf, out_dir: PathBuf) -> Result<()> {
    println!("petiglyph build-font");
    println!("  manifest: {}", manifest.display());
    println!("  out-dir: {}", out_dir.display());
    println!("  status: scaffolding only (SVG->glyph pipeline not implemented yet)");
    Ok(())
}
