use clap::{Parser, Subcommand};
use orchest_core::{Orchest, OrchestError};
use serde_json::{json, Value};
use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "orchest",
    version,
    about = "Local development environments managed by Orchest"
)]
struct Cli {
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    quiet: bool,
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Init,
    Status,
    Doctor,
    Port {
        #[command(subcommand)]
        command: PortCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Package {
        #[command(subcommand)]
        command: PackageCommand,
    },
    Php {
        #[command(subcommand)]
        command: PhpCommand,
    },
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Exec {
        #[arg(long)]
        project: Option<String>,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
}
#[derive(Subcommand)]
enum ConfigCommand {
    Show,
    Get { key: String },
    Set { key: String, value: String },
}
#[derive(Subcommand)]
enum PortCommand {
    List,
    Check { port: u16 },
}
#[derive(Subcommand)]
enum PackageCommand {
    List,
    Search {
        query: String,
    },
    Installed,
    Install {
        spec: String,
        #[arg(long)]
        archive: Option<PathBuf>,
    },
    Remove {
        spec: String,
        #[arg(long)]
        force: bool,
    },
}
#[derive(Subcommand)]
enum PhpCommand {
    List,
    Install {
        version: String,
        #[arg(long)]
        archive: Option<PathBuf>,
    },
    Remove {
        version: String,
        #[arg(long)]
        force: bool,
    },
    Default {
        version: String,
    },
}
#[derive(Subcommand)]
enum ProjectCommand {
    List,
    Add {
        path: PathBuf,
        #[arg(long)]
        name: String,
    },
    Show {
        name: String,
    },
    Php {
        name: String,
        version: String,
    },
    Exec {
        name: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.verbose {
        tracing_subscriber::fmt().with_env_filter("debug").init();
    }
    let result = run(&cli);
    match result {
        Ok((value, code)) => {
            if cli.json {
                println!("{}", json!({"ok":true,"data":value}));
            } else if !cli.quiet {
                print_human(&value);
            }
            ExitCode::from(code.clamp(0, 255) as u8)
        }
        Err(error) => {
            if cli.json {
                println!(
                    "{}",
                    json!({"ok":false,"error":{"code":error_code(&error),"message":error.to_string()}})
                );
            } else {
                eprintln!("Error: {error}");
            }
            ExitCode::FAILURE
        }
    }
}
fn run(cli: &Cli) -> Result<(Value, i32), OrchestError> {
    let root = cli
        .root
        .clone()
        .or_else(|| env::var_os("ORCHEST_ROOT").map(PathBuf::from))
        .map(Ok)
        .unwrap_or_else(orchest_platform::default_root)?;
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../manifests");
    let app = match cli.command {
        Command::Init => Orchest::init(root, &bundled)?,
        _ => Orchest::open(root.clone(), &root.join("config/packages"))?,
    };
    let output = match &cli.command {
        Command::Init => json!({"root":app.root(),"initialized":true}),
        Command::Status => {
            json!({"root":app.root(),"installed":app.installed(None)?.len(),"projects":app.projects()?.len()})
        }
        Command::Doctor => json!(app.doctor()),
        Command::Port { command } => match command {
            PortCommand::List => json!(app.ports()?),
            PortCommand::Check { port } => json!(app.check_port(*port)?),
        },
        Command::Config { command } => match command {
            ConfigCommand::Show => json!(app.config()?),
            ConfigCommand::Get { key } => {
                let data = serde_json::to_value(app.config()?)
                    .map_err(|e| OrchestError::Config(e.to_string()))?;
                let found = key
                    .split('.')
                    .try_fold(&data, |value, part| value.get(part))
                    .ok_or_else(|| {
                        OrchestError::InvalidInput(format!("unknown config key: {key}"))
                    })?;
                found.clone()
            }
            ConfigCommand::Set { key, value } => json!(app.config_set(key, value)?),
        },
        Command::Package { command } => match command {
            PackageCommand::List => json!(app.catalog().list()),
            PackageCommand::Search { query } => json!(app
                .catalog()
                .list()
                .into_iter()
                .filter(|m| m.package.id.contains(query)
                    || m.package
                        .name
                        .to_lowercase()
                        .contains(&query.to_lowercase()))
                .collect::<Vec<_>>()),
            PackageCommand::Installed => json!(app.installed(None)?),
            PackageCommand::Install { spec, archive } => {
                let (name, version) = spec
                    .split_once('@')
                    .ok_or_else(|| OrchestError::InvalidInput("use package@version".into()))?;
                json!(if let Some(archive) = archive {
                    app.install_from_archive(name, version, archive)?
                } else {
                    app.install(name, version)?
                })
            }
            PackageCommand::Remove { spec, force } => {
                let (name, version) = spec
                    .split_once('@')
                    .ok_or_else(|| OrchestError::InvalidInput("use package@version".into()))?;
                app.remove(name, version, *force)?;
                json!({"removed":spec})
            }
        },
        Command::Php { command } => match command {
            PhpCommand::List => json!(app.installed(Some("php"))?),
            PhpCommand::Install { version, archive } => json!(if let Some(archive) = archive {
                app.install_from_archive("php", version, archive)?
            } else {
                app.install("php", version)?
            }),
            PhpCommand::Remove { version, force } => {
                app.remove("php", version, *force)?;
                json!({"removed":version})
            }
            PhpCommand::Default { version } => json!(app.config_set("defaults.php", version)?),
        },
        Command::Project { command } => match command {
            ProjectCommand::List => json!(app.projects()?),
            ProjectCommand::Add { path, name } => json!(app.add_project(path, name)?),
            ProjectCommand::Show { name } => json!(app.project(name)?),
            ProjectCommand::Php { name, version } => json!(app.set_project_php(name, version)?),
            ProjectCommand::Exec { name, args } => return exec(&app, Some(name), args, cli.json),
        },
        Command::Exec { project, args } => return exec(&app, project.as_deref(), args, cli.json),
    };
    Ok((output, 0))
}
fn exec(
    app: &Orchest,
    project: Option<&str>,
    args: &[OsString],
    json_output: bool,
) -> Result<(Value, i32), OrchestError> {
    if args.first().is_none_or(|a| a != "php") {
        return Err(OrchestError::InvalidInput(
            "only the managed php command is available in this milestone".into(),
        ));
    }
    let project = project.map(|name| app.project(name)).transpose()?;
    if json_output {
        let (code, stdout, stderr) = app.exec_php_capture(project.as_ref(), &args[1..])?;
        Ok((
            json!({"exit_code":code,"stdout":stdout,"stderr":stderr}),
            code,
        ))
    } else {
        let code = app.exec_php(project.as_ref(), &args[1..])?;
        Ok((json!({"exit_code":code}), code))
    }
}
fn error_code(error: &OrchestError) -> &'static str {
    match error {
        OrchestError::NotInitialized(_) => "not_initialized",
        OrchestError::ProjectNotFound(_) => "project_not_found",
        OrchestError::ProjectExists(_) => "project_exists",
        OrchestError::RuntimeNotInstalled(_) => "runtime_not_installed",
        OrchestError::NoDefaultPhp => "no_default_php",
        OrchestError::PackageInUse(_) => "package_in_use",
        OrchestError::InvalidInput(_) => "invalid_input",
        OrchestError::Config(_) => "invalid_configuration",
        OrchestError::Package(_) => "package_error",
        OrchestError::Database(_) => "database_error",
        OrchestError::Platform(_) => "platform_error",
        OrchestError::Process(_) => "process_error",
        OrchestError::Io(_) => "io_error",
    }
}
fn print_human(value: &Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                println!("{}", serde_json::to_string_pretty(item).unwrap_or_default());
            }
        }
        _ => println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        ),
    }
}
