use osarg::{Arg, Error, Parser, help};
use std::ffi::OsString;
#[cfg(feature = "template")]
use std::path::PathBuf;

#[cfg(feature = "config-toml")]
mod config;
#[cfg(feature = "install-hooks")]
mod installer;
#[cfg(feature = "full")]
mod motd;
#[cfg(feature = "template")]
mod template;

#[cfg(feature = "full")]
use config::{expand_tilde, load_config, merge_config};
#[cfg(feature = "full")]
use std::path::Path;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_TEMPLATE_OUTPUT: &str = "Welcome!";

#[cfg(feature = "full")]
const ROOT_HELP_SECTIONS: &[help::Section<'static>] = &[
    help::Section::new(
        "commands:",
        "  install      install motdyn into login startup hooks\n  uninstall    remove motdyn from login startup hooks\n  status       show whether motdyn is installed for login shells",
    ),
    help::Section::new(
        "template options:",
        "      --text TEXT          render TEXT as an environment template\n      --file PATH          render a local template file\n      --env-prefix PREFIX  read ${NAME} from PREFIXNAME",
    ),
    help::Section::new(
        "motd options:",
        "  -v, --verbose              show diagnostics\n      --profile auto|full|basic\n      --plain                disable ANSI color\n      --compact              use dense output\n      --section-headers      group output by section",
    ),
    help::Section::new(
        "standard options:",
        "  -h, --help                 show help\n  -V, --version              show version",
    ),
];

#[cfg(all(not(feature = "full"), feature = "template"))]
const ROOT_HELP_SECTIONS: &[help::Section<'static>] = &[
    help::Section::new(
        "template options:",
        "      --text TEXT          render TEXT as an environment template\n      --file PATH          render a local template file\n      --env-prefix PREFIX  read ${NAME} from PREFIXNAME",
    ),
    help::Section::new(
        "standard options:",
        "  -h, --help       show help\n  -V, --version    show version",
    ),
];

#[cfg(all(not(feature = "full"), not(feature = "template")))]
const ROOT_HELP_SECTIONS: &[help::Section<'static>] = &[help::Section::new(
    "standard options:",
    "  -h, --help       show help\n  -V, --version    show version",
)];

const ROOT_HELP: help::Help<'static> =
    help::Help::new("motdyn [OPTIONS] [COMMAND]", ROOT_HELP_SECTIONS);

#[cfg(feature = "install-hooks")]
const INSTALL_HELP_SECTIONS: &[help::Section<'static>] = &[help::Section::new(
    "options:",
    "  -h, --help             show help\n  -V, --version          show version\n      --user             install only for the current user\n      --target TARGET    profile, bash_profile, bash_login, or zprofile",
)];
#[cfg(feature = "install-hooks")]
const INSTALL_HELP: help::Help<'static> =
    help::Help::new("motdyn install [OPTIONS]", INSTALL_HELP_SECTIONS);

#[cfg(feature = "install-hooks")]
const UNINSTALL_HELP_SECTIONS: &[help::Section<'static>] = &[help::Section::new(
    "options:",
    "  -h, --help             show help\n  -V, --version          show version\n      --user             remove only the current user's install hook\n      --target TARGET    profile, bash_profile, bash_login, or zprofile",
)];
#[cfg(feature = "install-hooks")]
const UNINSTALL_HELP: help::Help<'static> =
    help::Help::new("motdyn uninstall [OPTIONS]", UNINSTALL_HELP_SECTIONS);

#[cfg(feature = "install-hooks")]
const STATUS_HELP_SECTIONS: &[help::Section<'static>] = &[help::Section::new(
    "options:",
    "  -h, --help             show help\n  -V, --version          show version\n      --user             check only the current user's install hook\n      --target TARGET    profile, bash_profile, bash_login, or zprofile",
)];
#[cfg(feature = "install-hooks")]
const STATUS_HELP: help::Help<'static> =
    help::Help::new("motdyn status [OPTIONS]", STATUS_HELP_SECTIONS);

#[derive(Debug, Default)]
struct Cli {
    #[cfg(feature = "full")]
    verbose: bool,
    #[cfg(feature = "full")]
    profile: ProfileArg,
    #[cfg(feature = "full")]
    plain: bool,
    #[cfg(feature = "full")]
    compact: bool,
    #[cfg(feature = "full")]
    section_headers: bool,
    #[cfg(feature = "install-hooks")]
    cmd: Option<Commands>,
    #[cfg(feature = "template")]
    template: TemplateInput,
}

#[cfg(feature = "template")]
#[derive(Debug, Default)]
struct TemplateInput {
    source: Option<TemplateSource>,
    env_prefix: Option<String>,
}

#[cfg(feature = "template")]
#[derive(Debug)]
enum TemplateSource {
    Text(String),
    File(PathBuf),
}

#[cfg(feature = "full")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ProfileArg {
    #[default]
    Auto,
    Full,
    Basic,
}

#[cfg(feature = "full")]
impl ProfileArg {
    fn parse(value: String) -> Result<Self, Error> {
        match value.as_str() {
            "auto" => Ok(Self::Auto),
            "full" => Ok(Self::Full),
            "basic" => Ok(Self::Basic),
            _ => Err(Error::invalid_value_for(value.into())),
        }
    }
}

#[cfg(feature = "full")]
impl From<ProfileArg> for motd::ModuleProfile {
    fn from(value: ProfileArg) -> Self {
        match value {
            ProfileArg::Auto => Self::Auto,
            ProfileArg::Full => Self::Full,
            ProfileArg::Basic => Self::Basic,
        }
    }
}

#[cfg(feature = "install-hooks")]
#[derive(Debug)]
enum Commands {
    Install(CommandArgs),
    Uninstall(CommandArgs),
    Status(CommandArgs),
}

#[cfg(feature = "install-hooks")]
#[derive(Debug, Default)]
struct CommandArgs {
    user: bool,
    target: Option<installer::UserProfileTarget>,
}

#[cfg(feature = "install-hooks")]
#[derive(Clone, Copy)]
enum CommandKind {
    Install,
    Uninstall,
    Status,
}

enum CliAction {
    Run(Cli),
    Help(help::Help<'static>),
    Version,
}

fn main() {
    match parse_cli_from_env() {
        Ok(CliAction::Run(cli)) => run_cli(cli),
        Ok(CliAction::Help(help_doc)) => {
            if let Err(err) = help_doc.write(&mut std::io::stdout().lock()) {
                eprintln!("Failed to write help: {}", err);
                std::process::exit(1);
            }
        }
        Ok(CliAction::Version) => {
            println!("{}", VERSION);
        }
        Err(err) => {
            eprintln!("error: {}", err);
            eprintln!("try 'motdyn --help' for usage");
            std::process::exit(2);
        }
    }
}

fn parse_cli_from_env() -> Result<CliAction, Error> {
    parse_cli(Parser::from_env())
}

#[cfg(all(
    test,
    any(feature = "full", feature = "template", feature = "install-hooks")
))]
fn parse_cli_from_args<T, S>(args: T) -> Result<CliAction, Error>
where
    T: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    parse_cli(Parser::from_args(args))
}

#[cfg(any(feature = "full", feature = "template", feature = "install-hooks"))]
fn parse_cli<I>(mut parser: Parser<I>) -> Result<CliAction, Error>
where
    I: Iterator<Item = OsString>,
{
    #[cfg(any(feature = "full", feature = "template", feature = "install-hooks"))]
    let mut cli = Cli::default();
    #[cfg(not(any(feature = "full", feature = "template", feature = "install-hooks")))]
    let cli = Cli::default();

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Short('h') | Arg::Long("help") => return Ok(CliAction::Help(ROOT_HELP)),
            Arg::Short('V') | Arg::Long("version") => return Ok(CliAction::Version),
            #[cfg(feature = "full")]
            Arg::Short('v') | Arg::Long("verbose") => {
                cli.verbose = true;
            }
            #[cfg(feature = "full")]
            Arg::Long("profile") => {
                cli.profile = ProfileArg::parse(parser.string_owned()?)?;
            }
            #[cfg(feature = "full")]
            Arg::Long("plain") => {
                cli.plain = true;
            }
            #[cfg(feature = "full")]
            Arg::Long("compact") => {
                cli.compact = true;
            }
            #[cfg(feature = "full")]
            Arg::Long("section-headers") => {
                cli.section_headers = true;
            }
            #[cfg(feature = "template")]
            Arg::Long("text") => {
                cli.template.source = Some(TemplateSource::Text(parser.string_owned()?));
            }
            #[cfg(feature = "template")]
            Arg::Long("file") => {
                cli.template.source = Some(TemplateSource::File(parser.value()?.to_path_buf()));
            }
            #[cfg(feature = "template")]
            Arg::Long("env-prefix") => {
                cli.template.env_prefix = Some(parser.string_owned()?);
            }
            Arg::Value(value) => match value.to_str()? {
                #[cfg(feature = "install-hooks")]
                "install" => return parse_command(parser, cli, CommandKind::Install),
                #[cfg(feature = "install-hooks")]
                "uninstall" => return parse_command(parser, cli, CommandKind::Uninstall),
                #[cfg(feature = "install-hooks")]
                "status" => return parse_command(parser, cli, CommandKind::Status),
                _ => return Err(Error::unexpected_argument(value.to_os_string())),
            },
            other => return Err(other.unexpected()),
        }
    }

    Ok(CliAction::Run(cli))
}

#[cfg(not(any(feature = "full", feature = "template", feature = "install-hooks")))]
fn parse_cli<I>(mut parser: Parser<I>) -> Result<CliAction, Error>
where
    I: Iterator<Item = OsString>,
{
    match parser.next()? {
        Some(Arg::Short('h')) | Some(Arg::Long("help")) => Ok(CliAction::Help(ROOT_HELP)),
        Some(Arg::Short('V')) | Some(Arg::Long("version")) => Ok(CliAction::Version),
        Some(Arg::Value(value)) => Err(Error::unexpected_argument(value.to_os_string())),
        Some(other) => Err(other.unexpected()),
        None => Ok(CliAction::Run(Cli::default())),
    }
}

#[cfg(feature = "install-hooks")]
fn parse_command<I>(
    mut parser: Parser<I>,
    mut cli: Cli,
    kind: CommandKind,
) -> Result<CliAction, Error>
where
    I: Iterator<Item = OsString>,
{
    let mut args = CommandArgs::default();
    let help_doc = command_help(kind);

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Short('h') | Arg::Long("help") => return Ok(CliAction::Help(help_doc)),
            Arg::Short('V') | Arg::Long("version") => return Ok(CliAction::Version),
            #[cfg(feature = "full")]
            Arg::Short('v') | Arg::Long("verbose") => {
                cli.verbose = true;
            }
            #[cfg(feature = "full")]
            Arg::Long("profile") => {
                cli.profile = ProfileArg::parse(parser.string_owned()?)?;
            }
            #[cfg(feature = "full")]
            Arg::Long("plain") => {
                cli.plain = true;
            }
            #[cfg(feature = "full")]
            Arg::Long("compact") => {
                cli.compact = true;
            }
            #[cfg(feature = "full")]
            Arg::Long("section-headers") => {
                cli.section_headers = true;
            }
            #[cfg(feature = "template")]
            Arg::Long("text") => {
                cli.template.source = Some(TemplateSource::Text(parser.string_owned()?));
            }
            #[cfg(feature = "template")]
            Arg::Long("file") => {
                cli.template.source = Some(TemplateSource::File(parser.value()?.to_path_buf()));
            }
            #[cfg(feature = "template")]
            Arg::Long("env-prefix") => {
                cli.template.env_prefix = Some(parser.string_owned()?);
            }
            Arg::Long("user") => {
                args.user = true;
            }
            Arg::Long("target") => {
                let value = parser.string_owned()?;
                args.target = Some(parse_user_profile_target(value)?);
            }
            Arg::Value(value) => return Err(value.unexpected()),
            other => return Err(other.unexpected()),
        }
    }

    cli.cmd = Some(match kind {
        CommandKind::Install => Commands::Install(args),
        CommandKind::Uninstall => Commands::Uninstall(args),
        CommandKind::Status => Commands::Status(args),
    });

    Ok(CliAction::Run(cli))
}

#[cfg(feature = "install-hooks")]
fn command_help(kind: CommandKind) -> help::Help<'static> {
    match kind {
        CommandKind::Install => INSTALL_HELP,
        CommandKind::Uninstall => UNINSTALL_HELP,
        CommandKind::Status => STATUS_HELP,
    }
}

#[cfg(feature = "install-hooks")]
fn parse_user_profile_target(value: String) -> Result<installer::UserProfileTarget, Error> {
    installer::UserProfileTarget::parse_name(&value)
        .ok_or_else(|| Error::invalid_value_for(value.into()))
}

fn run_cli(cli: Cli) {
    #[cfg(not(any(feature = "full", feature = "template", feature = "install-hooks")))]
    let _ = cli;

    #[cfg(feature = "template")]
    if cli.template.source.is_some() {
        render_template_or_exit(&cli.template);
        return;
    }

    #[cfg(feature = "install-hooks")]
    match cli.cmd {
        Some(Commands::Install(args)) => {
            if let Err(e) = installer::do_install(args.user, args.target) {
                eprintln!("Install failed: {}", e);
                std::process::exit(1);
            }
            println!("Install successful!");
            return;
        }
        Some(Commands::Uninstall(args)) => {
            if let Err(e) = installer::do_uninstall(args.user, args.target) {
                eprintln!("Uninstall failed: {}", e);
                std::process::exit(1);
            }
            println!("Uninstall successful!");
            return;
        }
        Some(Commands::Status(args)) => {
            if let Err(e) = installer::do_status(args.user, args.target) {
                eprintln!("Status check failed: {}", e);
                std::process::exit(1);
            }
            return;
        }
        None => {}
    }

    #[cfg(feature = "full")]
    run_motd_safely(&cli);

    #[cfg(not(feature = "full"))]
    println!("{}", DEFAULT_TEMPLATE_OUTPUT);
}

#[cfg(feature = "template")]
fn render_template_or_exit(input: &TemplateInput) {
    let source = match input.source.as_ref() {
        Some(TemplateSource::Text(text)) => text.clone(),
        Some(TemplateSource::File(path)) => match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) => {
                eprintln!("Failed to read template '{}': {}", path.display(), err);
                std::process::exit(1);
            }
        },
        None => DEFAULT_TEMPLATE_OUTPUT.to_string(),
    };

    match template::render_template(&source, input.env_prefix.as_deref()) {
        Ok(rendered) => print!("{}", rendered),
        Err(err) => {
            eprintln!("Failed to render template: {}", err);
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "full")]
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

#[cfg(feature = "full")]
fn run_motd_safely(cli: &Cli) {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(|| run_motd(cli));
    std::panic::set_hook(previous_hook);

    let _ = result;
}

#[cfg(all(
    test,
    any(feature = "full", feature = "template", feature = "install-hooks")
))]
mod tests {
    use super::*;

    fn parse_run(args: &[&str]) -> Cli {
        match parse_cli_from_args(args).expect("cli should parse") {
            CliAction::Run(cli) => cli,
            CliAction::Help(_) | CliAction::Version => panic!("expected runnable cli"),
        }
    }

    #[cfg(feature = "install-hooks")]
    #[test]
    fn osarg_parses_user_profile_target_values() {
        let cli = parse_run(&["install", "--user", "--target", "bash_profile"]);

        match cli.cmd {
            Some(Commands::Install(args)) => {
                assert_eq!(args.target, Some(installer::UserProfileTarget::BashProfile));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[cfg(feature = "full")]
    #[test]
    fn osarg_parses_profile_values() {
        let cli = parse_run(&["--profile", "basic"]);
        assert_eq!(cli.profile, ProfileArg::Basic);

        let cli = parse_run(&["--profile", "full"]);
        assert_eq!(cli.profile, ProfileArg::Full);
    }

    #[cfg(all(feature = "full", feature = "install-hooks"))]
    #[test]
    fn osarg_parses_global_options_after_subcommand() {
        let cli = parse_run(&[
            "status",
            "--user",
            "--target=bash_login",
            "--profile=full",
            "-v",
        ]);

        assert!(cli.verbose);
        assert_eq!(cli.profile, ProfileArg::Full);
        match cli.cmd {
            Some(Commands::Status(args)) => {
                assert!(args.user);
                assert_eq!(args.target, Some(installer::UserProfileTarget::BashLogin));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[cfg(feature = "install-hooks")]
    #[test]
    fn osarg_returns_command_help() {
        let action = parse_cli_from_args(["status", "--help"]).expect("cli should parse");
        match action {
            CliAction::Help(help_doc) => assert_eq!(help_doc.usage(), "motdyn status [OPTIONS]"),
            CliAction::Run(_) | CliAction::Version => panic!("expected status help"),
        }
    }

    #[cfg(feature = "template")]
    #[test]
    fn osarg_parses_template_options() {
        let cli = parse_run(&["--text", "hello ${NAME}", "--env-prefix", "APP_"]);

        match cli.template.source {
            Some(TemplateSource::Text(text)) => assert_eq!(text, "hello ${NAME}"),
            Some(TemplateSource::File(_)) | None => panic!("expected text template"),
        }
        assert_eq!(cli.template.env_prefix.as_deref(), Some("APP_"));
    }
}
