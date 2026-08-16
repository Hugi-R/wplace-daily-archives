use std::process;

use anyhow::Context;
use clap::{Parser, Subcommand};
use wimage::tilehistory::DateHours;

mod merge;
mod validate;
mod increment;
mod makebase;
mod common;

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
    /// Add PNG tiles from an increment db into the latest archive.
    Increment {
        /// Folder containing the archive .db files
        #[arg(long)]
        archives: String,
        /// Path to the increment SQLite db (PNG tiles at z=11)
        #[arg(long)]
        increment: String,
    },
    /// Run Increment then Merge
    Ingest {
        /// Folder containing the archive .db files
        #[arg(long)]
        archives: String,
        /// Path to the increment SQLite db (PNG tiles at z=11)
        #[arg(long)]
        increment: String,
    },
    /// Create a new base archive from PNG tiles.
    Makebase {
        /// Path to the SQLite base db (PNG tiles at z=11)
        #[arg(long)]
        base: String,
        /// Path to the new archive SQLite db to create
        #[arg(long)]
        output: String,
        /// Datehour: hours since 2025-01-01T00:00:00Z (default 0)
        #[arg(long, default_value_t = 0)]
        datehours: u32,
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
        Command::Increment { archives, increment } => {
            eprintln!(
                "Incrementing from {increment} into latest archive in {archives}"
            );
            match increment::increment(&archives, &increment)
                .with_context(|| "increment failed")
            {
                Ok((new_archive_path, date_hours)) => {
                    eprintln!("Increment completed successfully for datehour={}. New archive path: {}", date_hours.to_datetime(), new_archive_path.display());
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Command::Ingest { archives, increment } => {
            eprintln!(
                "Ingesting from {increment} into latest archive in {archives}"
            );
            match increment::increment(&archives, &increment)
                .with_context(|| "increment failed")
            {
                Ok((new_archive_path, date_hours)) => {
                    eprintln!("Increment completed successfully for datehour={}. New archive path: {}", date_hours.to_datetime(), new_archive_path.display());
                    eprintln!("Merging for datehour={} ({}) from {}", date_hours.0, date_hours.to_datetime(), new_archive_path.display());
                    merge::merge(new_archive_path.to_str().unwrap(), date_hours)
                        .with_context(|| "merge failed")
                }
                Err(e) => Err(e),
            }
        }
        Command::Makebase { base, output, datehours } => {
            let date_hours = DateHours(datehours);
            makebase::makebase(&base, &output, date_hours).with_context(|| "makebase failed")
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e:?}");
        process::exit(1);
    }
}
