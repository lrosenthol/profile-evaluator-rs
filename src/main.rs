use std::fs;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use profile_evaluator_rs::{OutputFormat, evaluate_files, serialize_report};

#[derive(Debug, Parser)]
#[command(name = "profile-evaluator")]
#[command(about = "Evaluate an asset profile (YAML) against indicators JSON")]
struct Cli {
    #[arg(short, long)]
    profile: PathBuf,

    #[arg(short, long)]
    indicators: PathBuf,

    #[arg(short, long, value_enum, default_value_t = FormatArg::Json)]
    format: FormatArg,

    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    Json,
    Yaml,
}

fn main() {
    let cli = Cli::parse();

    let format = match cli.format {
        FormatArg::Json => OutputFormat::Json,
        FormatArg::Yaml => OutputFormat::Yaml,
    };

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let report = evaluate_files(&cli.profile, &cli.indicators)?;
        let serialized = serialize_report(&report, format)?;

        if let Some(out_path) = &cli.output {
            fs::write(out_path, serialized)?;
        } else {
            println!("{serialized}");
        }

        Ok(())
    })();

    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
