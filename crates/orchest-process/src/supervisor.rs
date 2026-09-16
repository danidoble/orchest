use crate::ProcessError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use sysinfo::{Pid, ProcessesToUpdate, Signal, System};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessState {
    pub service_id: String,
    pub pid: u32,
    pub executable: PathBuf,
    pub started_at: DateTime<Utc>,
    pub process_start_time: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Running,
    Stopped,
    Stale,
}

pub struct Supervisor {
    state_dir: PathBuf,
    log_dir: PathBuf,
}
impl Supervisor {
    pub fn new(state_dir: PathBuf, log_dir: PathBuf) -> Self {
        Self { state_dir, log_dir }
    }
    fn state_path(&self, service_id: &str) -> Result<PathBuf, ProcessError> {
        if service_id.is_empty()
            || !service_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ProcessError::InvalidState("invalid service id".into()));
        }
        Ok(self.state_dir.join(format!("{service_id}.json")))
    }
    pub fn state(&self, service_id: &str) -> Result<Option<ProcessState>, ProcessError> {
        let path = self.state_path(service_id)?;
        if !path.exists() {
            return Ok(None);
        }
        serde_json::from_slice(&fs::read(path)?)
            .map(Some)
            .map_err(|e| ProcessError::InvalidState(e.to_string()))
    }
    pub fn status(&self, service_id: &str) -> Result<ServiceStatus, ProcessError> {
        let Some(state) = self.state(service_id)? else {
            return Ok(ServiceStatus::Stopped);
        };
        let system = System::new_all();
        let Some(process) = system.process(Pid::from_u32(state.pid)) else {
            return Ok(ServiceStatus::Stale);
        };
        if process.start_time() == state.process_start_time
            && process.exe() == Some(state.executable.as_path())
        {
            Ok(ServiceStatus::Running)
        } else {
            Ok(ServiceStatus::Stale)
        }
    }
    pub fn start(
        &self,
        service_id: &str,
        binary: &Path,
        args: &[OsString],
        cwd: Option<&Path>,
    ) -> Result<ProcessState, ProcessError> {
        if self.status(service_id)? == ServiceStatus::Running {
            return Err(ProcessError::AlreadyRunning(service_id.into()));
        }
        if !binary.is_file() {
            return Err(ProcessError::InvalidState(format!(
                "missing executable: {}",
                binary.display()
            )));
        }
        let binary = binary.canonicalize()?;
        fs::create_dir_all(&self.state_dir)?;
        let log_dir = self.log_dir.join(service_id);
        fs::create_dir_all(&log_dir)?;
        let mut command = Command::new(&binary);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                File::options()
                    .create(true)
                    .append(true)
                    .open(log_dir.join("stdout.log"))?,
            ))
            .stderr(Stdio::from(
                File::options()
                    .create(true)
                    .append(true)
                    .open(log_dir.join("stderr.log"))?,
            ));
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let child = command.spawn()?;
        let pid = child.id();
        let mut system = System::new();
        let process_start_time = (0..10)
            .find_map(|_| {
                system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
                let found = system.process(Pid::from_u32(pid)).map(|p| p.start_time());
                if found.is_none() {
                    thread::sleep(Duration::from_millis(50));
                }
                found
            })
            .ok_or_else(|| {
                ProcessError::InvalidState(
                    "service exited before its identity could be recorded".into(),
                )
            })?;
        let state = ProcessState {
            service_id: service_id.into(),
            pid,
            executable: binary,
            started_at: Utc::now(),
            process_start_time,
        };
        orchest_platform::atomic_write(
            &self.state_path(service_id)?,
            &serde_json::to_vec_pretty(&state)
                .map_err(|e| ProcessError::InvalidState(e.to_string()))?,
        )?;
        Ok(state)
    }
    pub fn stop(&self, service_id: &str) -> Result<(), ProcessError> {
        let Some(state) = self.state(service_id)? else {
            return Ok(());
        };
        let system = System::new_all();
        let Some(process) = system.process(Pid::from_u32(state.pid)) else {
            fs::remove_file(self.state_path(service_id)?)?;
            return Ok(());
        };
        if process.start_time() != state.process_start_time
            || process.exe() != Some(state.executable.as_path())
        {
            return Err(ProcessError::IdentityMismatch(state.pid));
        }
        if process.kill_with(Signal::Term).is_none() {
            process.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.status(service_id)? != ServiceStatus::Running {
                fs::remove_file(self.state_path(service_id)?)?;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        let system = System::new_all();
        if let Some(process) = system.process(Pid::from_u32(state.pid)) {
            if process.start_time() == state.process_start_time
                && process.exe() == Some(state.executable.as_path())
            {
                process.kill();
            }
        }
        if self.status(service_id)? == ServiceStatus::Running {
            return Err(ProcessError::StopTimeout);
        }
        fs::remove_file(self.state_path(service_id)?)?;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn starts_and_stops_managed_process() {
        let root = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(root.path().join("state"), root.path().join("logs"));
        let state = supervisor
            .start("fixture", Path::new("/bin/sleep"), &["30".into()], None)
            .unwrap();
        assert!(state.pid > 0);
        assert_eq!(
            supervisor.status("fixture").unwrap(),
            ServiceStatus::Running
        );
        supervisor.stop("fixture").unwrap();
        assert_eq!(
            supervisor.status("fixture").unwrap(),
            ServiceStatus::Stopped
        );
    }
}
