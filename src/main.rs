use std::process;

use anyhow::Context;
use clap::{Parser, Subcommand};
use wimage::tilehistory::DateHours;

mod merge;
mod validate;

#[derive(Parser)]
#[command(name = "wplace-daily-archives")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Merge tiles from z=11 down to z=0 for a given datehour.
    Merge {
        /// Datehour: hours since 2025-01-01T00:00:00Z
        #[arg(short, long)]
        t: u32,
        /// Path to the SQLite database
        input_db: String,
    },
    /// Validate (and fix) every tile's TileHistory in the database.
    Validate {
        /// Path to the SQLite database
        input_db: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Merge { t, input_db } => {
            let date_hours = DateHours(t);
            eprintln!("Merging for datehour={t} ({}) from {}", date_hours.to_datetime(), input_db);
            merge::merge(&input_db, date_hours).with_context(|| "merge failed")
        }
        Command::Validate { input_db } => {
            eprintln!("Validating tiles in {}", input_db);
            validate::validate(&input_db).with_context(|| "validate failed")
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e:?}");
        process::exit(1);
    }
}
