use clap::{Parser, Subcommand};
use std::path::Path;

mod config;
mod installer;
mod motd;

use config::{expand_tilde, load_config, merge_config};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    cmd: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Install,
    Uninstall,
    Status,
}

fn main() {
    let cli = Cli::parse();

    match cli.cmd {
        Some(Commands::Install) => {
            if let Err(e) = installer::do_install() {
                eprintln!("Install failed: {}", e);
                std::process::exit(1);
            }
            println!("Install successful!");
        }
        Some(Commands::Uninstall) => {
            if let Err(e) = installer::do_uninstall() {
                eprintln!("Uninstall failed: {}", e);
                std::process::exit(1);
            }
            println!("Uninstall successful!");
        }
        Some(Commands::Status) => {
            if let Err(e) = installer::do_status() {
                eprintln!("Status check failed: {}", e);
                std::process::exit(1);
            }
        }
        None => run_motd(cli.verbose),
    }
}

fn run_motd(verbose: bool) {
    let sys_cfg = load_config(Path::new("/etc/motdyn/config.toml"));
    let usr_cfg = load_config(&expand_tilde("~/.config/motdyn/config.toml"));
    let merged_cfg = merge_config(sys_cfg, usr_cfg);

    motd::render(verbose, &merged_cfg);
}
