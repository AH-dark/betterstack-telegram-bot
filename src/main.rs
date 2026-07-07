mod config;

use clap::Parser;
use config::Config;

fn main() {
    let _config = Config::parse();
    println!("betterstack-telegram-bot starting");
}
