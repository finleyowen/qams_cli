mod init;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "qams", about = "Quality Assurance Management System")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Set up a new QAMS instance in the current directory.
    Init(InitUpdateArgs),
    /// Update the scorecard, agents, or review metadata for an existing instance.
    Update(InitUpdateArgs),
    /// Generate a report from reviews within a date range.
    Report(ReportArgs),
}

/// Arguments shared by `init` and `update`.
#[derive(clap::Args)]
struct InitUpdateArgs {
    /// Path to the scorecard CSV.
    #[arg(short = 's', long)]
    path_to_scorecard: PathBuf,

    /// Path to the agents CSV (first column = unique identifier).
    #[arg(short = 'a', long)]
    path_to_agents: PathBuf,

    /// Path to the review metadata CSV (optional; one field name per row).
    #[arg(short = 'r', long)]
    path_to_metadata: Option<PathBuf>,
}

#[derive(clap::Args)]
struct ReportArgs {
    /// Start date (inclusive), format: YYYY-MM-DD.
    #[arg(short = 's', long)]
    start_date: String,

    /// End date (inclusive), format: YYYY-MM-DD.
    #[arg(short = 'e', long)]
    end_date: String,

    /// Path to a directory of previous reports to include.
    #[arg(short = 'p', long)]
    path_to_previous_reports: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();
    let result: Result<(), String> = match cli.command {
        Commands::Init(args) | Commands::Update(args) => init::run(
            &args.path_to_scorecard,
            &args.path_to_agents,
            args.path_to_metadata.as_deref(),
        ),
        Commands::Report(_) => {
            eprintln!("The `report` command is not yet implemented.");
            std::process::exit(1);
        }
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}