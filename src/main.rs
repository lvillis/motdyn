use clap::{Args, Parser, Subcommand, ValueEnum};
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

    #[arg(long, value_enum, default_value_t = ProfileArg::Auto, global = true)]
    profile: ProfileArg,

    #[arg(long, global = true)]
    plain: bool,

    #[arg(long, global = true)]
    compact: bool,

    #[arg(long, global = true)]
    section_headers: bool,

    #[command(subcommand)]
    cmd: Option<Commands>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ProfileArg {
    Auto,
    Full,
    Basic,
}

impl From<ProfileArg> for motd::ModuleProfile {
    fn from(value: ProfileArg) -> Self {
        match value {
            ProfileArg::Auto => Self::Auto,
            ProfileArg::Full => Self::Full,
            ProfileArg::Basic => Self::Basic,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Install motdyn into login startup hooks.
    Install(InstallArgs),
    /// Remove motdyn from login startup hooks.
    Uninstall(UninstallArgs),
    /// Show whether motdyn is installed for login shells.
    Status(StatusArgs),
}

#[derive(Args, Debug)]
struct InstallArgs {
    /// Install only for the current user instead of system-wide.
    #[arg(long)]
    user: bool,

    /// Explicit user profile target: profile, bash_profile, bash_login, or zprofile.
    #[arg(long)]
    target: Option<installer::UserProfileTarget>,
}

#[derive(Args, Debug)]
struct UninstallArgs {
    /// Remove only the current user's install hook.
    #[arg(long)]
    user: bool,

    /// Explicit user profile target: profile, bash_profile, bash_login, or zprofile.
    #[arg(long)]
    target: Option<installer::UserProfileTarget>,
}

#[derive(Args, Debug)]
struct StatusArgs {
    /// Check only the current user's install hook.
    #[arg(long)]
    user: bool,

    /// Explicit user profile target: profile, bash_profile, bash_login, or zprofile.
    #[arg(long)]
    target: Option<installer::UserProfileTarget>,
}

fn main() {
    let cli = Cli::parse();

    match cli.cmd {
        Some(Commands::Install(args)) => {
            if let Err(e) = installer::do_install(args.user, args.target) {
                eprintln!("Install failed: {}", e);
                std::process::exit(1);
            }
            println!("Install successful!");
        }
        Some(Commands::Uninstall(args)) => {
            if let Err(e) = installer::do_uninstall(args.user, args.target) {
                eprintln!("Uninstall failed: {}", e);
                std::process::exit(1);
            }
            println!("Uninstall successful!");
        }
        Some(Commands::Status(args)) => {
            if let Err(e) = installer::do_status(args.user, args.target) {
                eprintln!("Status check failed: {}", e);
                std::process::exit(1);
            }
        }
        None => run_motd_safely(&cli),
    }
}

fn run_motd(cli: &Cli) {
    let sys_cfg_path = Path::new("/etc/motdyn/config.toml");
    let usr_cfg_path = expand_tilde("~/.config/motdyn/config.toml");

    let sys_cfg = load_config(sys_cfg_path);
    let usr_cfg = load_config(&usr_cfg_path);
    let render_ctx = motd::RenderContext {
        system_config_path: sys_cfg_path.display().to_string(),
        system_config_status: sys_cfg.status_label().to_string(),
        user_config_path: usr_cfg_path.display().to_string(),
        user_config_status: usr_cfg.status_label().to_string(),
        config_notes: [sys_cfg.note(), usr_cfg.note()]
            .into_iter()
            .flatten()
            .collect(),
    };
    let mut merged_cfg = merge_config(sys_cfg.config, usr_cfg.config);

    if cli.plain {
        merged_cfg.output.plain = Some(true);
    }
    if cli.compact {
        merged_cfg.output.compact = Some(true);
    }
    if cli.section_headers {
        merged_cfg.output.section_headers = Some(true);
    }

    motd::render(cli.verbose, cli.profile.into(), &merged_cfg, &render_ctx);
}

fn run_motd_safely(cli: &Cli) {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(|| run_motd(cli));
    std::panic::set_hook(previous_hook);

    let _ = result;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_parses_user_profile_target_values() {
        let cli = Cli::try_parse_from(["motdyn", "install", "--user", "--target", "bash_profile"])
            .expect("cli should parse");

        match cli.cmd {
            Some(Commands::Install(args)) => {
                assert_eq!(args.target, Some(installer::UserProfileTarget::BashProfile));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn clap_parses_profile_values() {
        let cli = Cli::try_parse_from(["motdyn", "--profile", "basic"]).expect("cli should parse");
        assert_eq!(cli.profile, ProfileArg::Basic);

        let cli = Cli::try_parse_from(["motdyn", "--profile", "full"]).expect("cli should parse");
        assert_eq!(cli.profile, ProfileArg::Full);
    }
}
