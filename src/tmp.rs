use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tmp")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Target {
        target: String,
    }
}