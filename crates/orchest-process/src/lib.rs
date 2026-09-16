//! Managed child process execution and supervision.
mod supervisor;
use std::{
    ffi::OsString,
    path::Path,
    process::{Command, ExitStatus, Output},
};
pub use supervisor::{ProcessState, ServiceStatus, Supervisor};

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("cannot run managed binary: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("the child process terminated without a status code")]
    NoExitCode,
    #[error("service {0} is already running")]
    AlreadyRunning(String),
    #[error("process identity mismatch; refusing to stop PID {0}")]
    IdentityMismatch(u32),
    #[error("service state is invalid: {0}")]
    InvalidState(String),
    #[error("process did not stop after termination request")]
    StopTimeout,
    #[error(transparent)]
    Platform(#[from] orchest_platform::PlatformError),
}

pub fn run(
    binary: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    managed_dirs: &[&Path],
) -> Result<ExitStatus, ProcessError> {
    run_with_env(binary, args, cwd, managed_dirs, &[])
}

pub fn run_with_env(
    binary: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    managed_dirs: &[&Path],
    envs: &[(&str, &Path)],
) -> Result<ExitStatus, ProcessError> {
    Ok(command(binary, args, cwd, managed_dirs, envs)?.status()?)
}

pub fn run_capture(
    binary: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    managed_dirs: &[&Path],
) -> Result<Output, ProcessError> {
    run_capture_with_env(binary, args, cwd, managed_dirs, &[])
}

pub fn run_capture_with_env(
    binary: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    managed_dirs: &[&Path],
    envs: &[(&str, &Path)],
) -> Result<Output, ProcessError> {
    Ok(command(binary, args, cwd, managed_dirs, envs)?.output()?)
}

fn command(
    binary: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    managed_dirs: &[&Path],
    envs: &[(&str, &Path)],
) -> Result<Command, ProcessError> {
    let mut command = Command::new(binary);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut paths: Vec<_> = managed_dirs.iter().map(|p| p.to_path_buf()).collect();
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    command.env(
        "PATH",
        std::env::join_paths(paths).map_err(std::io::Error::other)?,
    );
    for (name, value) in envs {
        command.env(name, value);
    }
    Ok(command)
}
