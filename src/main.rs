use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    what_sorry::run(what_sorry::Cli::parse())
}
