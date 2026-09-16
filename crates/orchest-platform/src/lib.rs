//! Operating-system paths and safe file operations.
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Result<Self, PlatformError> {
        if cfg!(windows) {
            Ok(Self::Windows)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else {
            Err(PlatformError::Unsupported)
        }
    }

    pub fn target(self) -> Result<&'static str, PlatformError> {
        if std::env::consts::ARCH != "x86_64" {
            return Err(PlatformError::Unsupported);
        }
        Ok(match self {
            Self::Windows => "windows-x86_64",
            Self::Linux => "linux-x86_64",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("unsupported operating system or architecture")]
    Unsupported,
    #[error("a default Orchest root could not be determined")]
    NoHome,
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub fn default_root() -> Result<PathBuf, PlatformError> {
    match Platform::current()? {
        Platform::Windows => Ok(env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or(PlatformError::NoHome)?
            .join("Orchest")),
        Platform::Linux => {
            if let Some(data) = env::var_os("XDG_DATA_HOME") {
                return Ok(PathBuf::from(data).join("orchest"));
            }
            Ok(env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or(PlatformError::NoHome)?
                .join(".local/share/orchest"))
        }
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PlatformError> {
    let parent = path.parent().ok_or(PlatformError::NoHome)?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| PlatformError::Io(e.error))?;
    Ok(())
}

pub fn ensure_layout(root: &Path) -> Result<(), PlatformError> {
    for name in [
        "bin",
        "config",
        "config/projects",
        "config/packages",
        "data",
        "www",
        "logs",
        "logs/orchest",
        "runtime",
        "runtime/pid",
        "runtime/generated",
        "runtime/state",
        "cache/downloads",
        "certificates/ca",
        "certificates/sites",
        "backups",
    ] {
        fs::create_dir_all(root.join(name))?;
    }
    Ok(())
}
