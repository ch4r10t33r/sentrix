mod commands;
mod detect_lang;
mod logger;
mod templates;

use clap::{Parser, Subcommand};

/// Sentrix — scaffold, run, and connect P2P AI agents.
#[derive(Parser)]
#[command(
    name    = "sentrix",
    version = env!("SENTRIX_VERSION"),
    about   = "Scaffold P2P-discoverable, DID-native AI agents across any framework",
    long_about = None,
    propagate_version = true,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new Sentrix agent project
    Init(commands::init::InitArgs),

    /// Generate agent source files inside an existing project
    Create(commands::create::CreateArgs),

    /// Start an agent in development mode
    Run(commands::run::RunArgs),

    /// Discover agents on the local mesh
    Discover(commands::discover::DiscoverArgs),

    /// Run or scaffold agent tests
    Test(commands::test::TestArgs),

    /// Inspect ANR records, agents, and capabilities
    Inspect(commands::inspect::InspectArgs),

    /// Print detailed version and platform information
    Version,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init(args) => commands::init::run(args),
        Command::Create(args) => commands::create::run(args),
        Command::Run(args) => commands::run::run(args),
        Command::Discover(args) => commands::discover::run(args),
        Command::Test(args) => commands::test::run(args),
        Command::Inspect(args) => commands::inspect::run(args),
        Command::Version => commands::version::run(),
    };
    if let Err(e) = result {
        logger::error(&format!("{e:#}"));
        std::process::exit(1);
    }
}
