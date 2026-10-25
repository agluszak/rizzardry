use bevy::app::AppExit;
use clap::Parser;

fn main() -> AppExit {
    rizzardry::app::run(rizzardry::app::Options::parse())
}
