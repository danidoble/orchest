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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessState {
    pub service_id: String,
    #[serde(default = "default_instance_id")]
    pub instance_id: String,
    pub pid: u32,
    pub executable: PathBuf,
    pub started_at: DateTime<Utc>,
    pub process_start_time: u64,
}

fn default_instance_id() -> String {
    "default".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceState {
    pub instance_id: String,
    pub status: ServiceStatus,
    pub process: ProcessState,
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
    fn validate_id(id: &str) -> Result<(), ProcessError> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ProcessError::InvalidState(
                "invalid service or instance id".into(),
            ));
        }
        Ok(())
    }
    fn state_path(&self, service_id: &str, instance_id: &str) -> Result<PathBuf, ProcessError> {
        Self::validate_id(service_id)?;
        Self::validate_id(instance_id)?;
        if instance_id == "default" {
            Ok(self.state_dir.join(format!("{service_id}.json")))
        } else {
            Ok(self
                .state_dir
                .join(service_id)
                .join(format!("{instance_id}.json")))
        }
    }
    fn log_path(&self, service_id: &str, instance_id: &str) -> PathBuf {
        let service = self.log_dir.join(service_id);
        if instance_id == "default" {
            service
        } else {
            service.join(instance_id)
        }
    }
    pub fn state(&self, service_id: &str) -> Result<Option<ProcessState>, ProcessError> {
        self.state_instance(service_id, "default")
    }
    /// Reads one instance record. The default instance also reads preexisting service records.
    pub fn state_instance(
        &self,
        service_id: &str,
        instance_id: &str,
    ) -> Result<Option<ProcessState>, ProcessError> {
        let path = self.state_path(service_id, instance_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let state: ProcessState = serde_json::from_slice(&fs::read(path)?)
            .map_err(|e| ProcessError::InvalidState(e.to_string()))?;
        if state.service_id != service_id || state.instance_id != instance_id {
            return Err(ProcessError::InvalidState(
                "state identity does not match its path".into(),
            ));
        }
        Ok(Some(state))
    }
    pub fn status(&self, service_id: &str) -> Result<ServiceStatus, ProcessError> {
        self.status_instance(service_id, "default")
    }
    /// Checks the recorded PID, start time, and executable before reporting it as running.
    pub fn status_instance(
        &self,
        service_id: &str,
        instance_id: &str,
    ) -> Result<ServiceStatus, ProcessError> {
        let Some(state) = self.state_instance(service_id, instance_id)? else {
            return Ok(ServiceStatus::Stopped);
        };
        let system = System::new_all();
        let Some(process) = system.process(Pid::from_u32(state.pid)) else {
            return Ok(ServiceStatus::Stale);
        };
        if process.start_time() == state.process_start_time
            && same_executable(process.exe(), &state.executable)
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
        self.start_instance(service_id, "default", binary, args, cwd)
    }
    /// Starts a service instance with its own state file and log directory.
    pub fn start_instance(
        &self,
        service_id: &str,
        instance_id: &str,
        binary: &Path,
        args: &[OsString],
        cwd: Option<&Path>,
    ) -> Result<ProcessState, ProcessError> {
        self.start_instance_with_env(service_id, instance_id, binary, args, cwd, &[])
    }
    pub fn start_instance_with_env(
        &self,
        service_id: &str,
        instance_id: &str,
        binary: &Path,
        args: &[OsString],
        cwd: Option<&Path>,
        envs: &[(&str, &Path)],
    ) -> Result<ProcessState, ProcessError> {
        let state_path = self.state_path(service_id, instance_id)?;
        if self.status_instance(service_id, instance_id)? == ServiceStatus::Running {
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
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let log_dir = self.log_path(service_id, instance_id);
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
        for (name, value) in envs {
            command.env(name, value);
        }
        let mut child = command.spawn()?;
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
                let _ = child.kill();
                let _ = child.wait();
                ProcessError::InvalidState(
                    "service exited before its identity could be recorded".into(),
                )
            })?;
        let state = ProcessState {
            service_id: service_id.into(),
            instance_id: instance_id.into(),
            pid,
            executable: binary,
            started_at: Utc::now(),
            process_start_time,
        };
        if let Err(error) = orchest_platform::atomic_write(
            &state_path,
            &serde_json::to_vec_pretty(&state)
                .map_err(|e| ProcessError::InvalidState(e.to_string()))?,
        ) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        Ok(state)
    }
    pub fn stop(&self, service_id: &str) -> Result<(), ProcessError> {
        self.stop_instance(service_id, "default")
    }
    /// Stops only the matching instance after validating its process identity.
    pub fn stop_instance(&self, service_id: &str, instance_id: &str) -> Result<(), ProcessError> {
        let state_path = self.state_path(service_id, instance_id)?;
        let Some(state) = self.state_instance(service_id, instance_id)? else {
            return Ok(());
        };
        let system = System::new_all();
        let Some(process) = system.process(Pid::from_u32(state.pid)) else {
            fs::remove_file(&state_path)?;
            return Ok(());
        };
        if process.start_time() != state.process_start_time
            || !same_executable(process.exe(), &state.executable)
        {
            return Err(ProcessError::IdentityMismatch(state.pid));
        }
        if process.kill_with(Signal::Term).is_none() {
            process.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.status_instance(service_id, instance_id)? != ServiceStatus::Running {
                fs::remove_file(&state_path)?;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        let system = System::new_all();
        if let Some(process) = system.process(Pid::from_u32(state.pid)) {
            if process.start_time() == state.process_start_time
                && same_executable(process.exe(), &state.executable)
            {
                process.kill();
            }
        }
        if self.status_instance(service_id, instance_id)? == ServiceStatus::Running {
            return Err(ProcessError::StopTimeout);
        }
        fs::remove_file(&state_path)?;
        Ok(())
    }
    /// Lists the persisted instances of a service and their current status.
    pub fn instances(&self, service_id: &str) -> Result<Vec<InstanceState>, ProcessError> {
        Self::validate_id(service_id)?;
        let mut ids = Vec::new();
        if self.state_path(service_id, "default")?.exists() {
            ids.push("default".to_string());
        }
        let dir = self.state_dir.join(service_id);
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    if let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) {
                        if id == "default" {
                            continue;
                        }
                        Self::validate_id(id)?;
                        ids.push(id.to_string());
                    }
                }
            }
        }
        ids.sort();
        ids.into_iter()
            .map(|instance_id| {
                let process = self
                    .state_instance(service_id, &instance_id)?
                    .ok_or_else(|| {
                        ProcessError::InvalidState("state disappeared during listing".into())
                    })?;
                let status = self.status_instance(service_id, &instance_id)?;
                Ok(InstanceState {
                    instance_id,
                    status,
                    process,
                })
            })
            .collect()
    }
    /// Lists services with persisted instance records, including legacy default records.
    pub fn services(&self) -> Result<Vec<String>, ProcessError> {
        let mut services = std::collections::BTreeSet::new();
        if !self.state_dir.exists() {
            return Ok(Vec::new());
        }
        for entry in fs::read_dir(&self.state_dir)? {
            let path = entry?.path();
            let name = if path.is_dir() {
                path.file_name().and_then(|name| name.to_str())
            } else if path.extension().is_some_and(|ext| ext == "json") {
                path.file_stem().and_then(|name| name.to_str())
            } else {
                None
            };
            if let Some(name) = name {
                Self::validate_id(name)?;
                services.insert(name.to_string());
            }
        }
        Ok(services.into_iter().collect())
    }
    /// Removes records for exited or mismatched processes without touching running instances.
    pub fn recover_stale(&self, service_id: &str) -> Result<Vec<String>, ProcessError> {
        let mut recovered = Vec::new();
        for instance in self.instances(service_id)? {
            if instance.status == ServiceStatus::Stale
                && self.state_instance(service_id, &instance.instance_id)? == Some(instance.process)
                && self.status_instance(service_id, &instance.instance_id)? == ServiceStatus::Stale
            {
                fs::remove_file(self.state_path(service_id, &instance.instance_id)?)?;
                recovered.push(instance.instance_id);
            }
        }
        Ok(recovered)
    }
}

fn same_executable(actual: Option<&Path>, recorded: &Path) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    actual == recorded || actual.canonicalize().is_ok_and(|path| path == recorded)
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn executable_identity_accepts_equivalent_paths() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("php-cgi.exe");
        fs::write(&executable, b"fixture").unwrap();
        let canonical = executable.canonicalize().unwrap();
        assert!(same_executable(Some(&executable), &canonical));
        assert!(!same_executable(None, &canonical));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn tracks_and_stops_a_managed_windows_process() {
        let root = tempfile::tempdir().unwrap();
        let binary = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let supervisor = Supervisor::new(root.path().join("state"), root.path().join("logs"));
        let state = supervisor
            .start(
                "fixture",
                &binary,
                &[
                    "-NoProfile".into(),
                    "-Command".into(),
                    "Start-Sleep -Seconds 30".into(),
                ],
                None,
            )
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

    #[test]
    fn isolates_instances_and_recovers_only_stale_state() {
        let root = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(root.path().join("state"), root.path().join("logs"));
        let first = supervisor
            .start_instance(
                "fixture",
                "one",
                Path::new("/bin/sleep"),
                &["30".into()],
                None,
            )
            .unwrap();
        let second = supervisor
            .start_instance(
                "fixture",
                "two",
                Path::new("/bin/sleep"),
                &["30".into()],
                None,
            )
            .unwrap();
        assert_ne!(first.pid, second.pid);
        assert_eq!(supervisor.instances("fixture").unwrap().len(), 2);
        assert!(root.path().join("logs/fixture/one/stdout.log").exists());
        assert!(root.path().join("logs/fixture/two/stdout.log").exists());
        supervisor.stop_instance("fixture", "one").unwrap();
        assert_eq!(
            supervisor.status_instance("fixture", "two").unwrap(),
            ServiceStatus::Running
        );

        let path = supervisor.state_path("fixture", "one").unwrap();
        orchest_platform::atomic_write(&path, &serde_json::to_vec(&first).unwrap()).unwrap();
        assert_eq!(
            supervisor.status_instance("fixture", "one").unwrap(),
            ServiceStatus::Stale
        );
        assert_eq!(supervisor.recover_stale("fixture").unwrap(), vec!["one"]);
        assert_eq!(
            supervisor.status_instance("fixture", "one").unwrap(),
            ServiceStatus::Stopped
        );
        assert_eq!(
            supervisor.status_instance("fixture", "two").unwrap(),
            ServiceStatus::Running
        );
        supervisor.stop_instance("fixture", "two").unwrap();
    }

    #[test]
    fn reads_legacy_default_state_and_rejects_unsafe_ids() {
        let root = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(root.path().join("state"), root.path().join("logs"));
        let state = supervisor
            .start("fixture", Path::new("/bin/sleep"), &["30".into()], None)
            .unwrap();
        let path = supervisor.state_path("fixture", "default").unwrap();
        let mut value = serde_json::to_value(&state).unwrap();
        value.as_object_mut().unwrap().remove("instance_id");
        orchest_platform::atomic_write(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            supervisor.state("fixture").unwrap().unwrap().instance_id,
            "default"
        );
        assert!(supervisor.status_instance("fixture", "../bad").is_err());
        supervisor.stop("fixture").unwrap();
    }
}
