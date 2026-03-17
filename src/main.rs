/*
Copyright 2026 Adobe. All rights reserved.
This file is licensed to you under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License. You may obtain a copy
of the License at http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed under
the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR REPRESENTATIONS
OF ANY KIND, either express or implied. See the License for the specific language
governing permissions and limitations under the License.
*/

#[cfg(not(target_arch = "wasm32"))]
fn main() {
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

#[cfg(target_arch = "wasm32")]
fn main() {
    // Binary is not used when built for WASM; entry point is the library.
}
