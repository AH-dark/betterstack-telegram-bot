mod config;
mod logging;

use clap::Parser;
use config::Config;

fn main() {
    let config = Config::parse();
    logging::init_logging(&config.log_format, &config.log_level);
    tracing::info!("betterstack-telegram-bot starting");
}
