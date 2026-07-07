pub mod betterstack;
mod config;
pub mod domain;
pub mod error;
mod logging;
pub mod render;
pub mod storage;
pub mod telegram;

use clap::Parser;
use config::Config;

fn main() {
    let config = Config::parse();
    logging::init_logging(&config.log_format, &config.log_level);
    tracing::info!("betterstack-telegram-bot starting");
}
