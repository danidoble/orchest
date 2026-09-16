//! Reusable Orchest application core.
use chrono::{DateTime, Utc};
use orchest_packages::{Artifact, Catalog, PackageError};
use orchest_platform::{atomic_write, ensure_layout, Platform};
use orchest_process::{ProcessState, ServiceStatus, Supervisor};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum OrchestError {
    #[error("Orchest is not initialized at {0}; run `orchest init`")]
    NotInitialized(PathBuf),
    #[error("project {0} was not found")]
    ProjectNotFound(String),
    #[error("project {0} already exists")]
    ProjectExists(String),
    #[error("runtime {0} is not installed")]
    RuntimeNotInstalled(String),
    #[error("no global PHP version is configured")]
    NoDefaultPhp,
    #[error("package version is in use: {0}")]
    PackageInUse(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Platform(#[from] orchest_platform::PlatformError),
    #[error(transparent)]
    Process(#[from] orchest_process::ProcessError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
pub type Result<T> = std::result::Result<T, OrchestError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub defaults: Defaults,
    pub ports: Ports,
    #[serde(default)]
    pub sources: Sources,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sources {
    pub github_repository: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Defaults {
    pub php: Option<String>,
    pub web_server: String,
    pub domain_suffix: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ports {
    pub nginx_http: u16,
    pub nginx_https: u16,
    pub mysql: u16,
    pub mariadb: u16,
    pub mongodb: u16,
    pub redis: u16,
    #[serde(default = "default_mailpit_http_port")]
    pub mailpit_http: u16,
    #[serde(default = "default_mailpit_smtp_port")]
    pub mailpit_smtp: u16,
}
fn default_mailpit_http_port() -> u16 {
    8025
}
fn default_mailpit_smtp_port() -> u16 {
    1025
}
impl Default for Config {
    fn default() -> Self {
        Self {
            defaults: Defaults {
                php: None,
                web_server: "nginx".into(),
                domain_suffix: "test".into(),
            },
            ports: Ports {
                nginx_http: 80,
                nginx_https: 443,
                mysql: 3306,
                mariadb: 3307,
                mongodb: 27017,
                redis: 6379,
                mailpit_http: default_mailpit_http_port(),
                mailpit_smtp: default_mailpit_smtp_port(),
            },
            sources: Sources::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installation {
    pub package: String,
    pub version: String,
    pub platform: String,
    pub installed_at: DateTime<Utc>,
    pub source_url: String,
    pub executable: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub php_version: Option<String>,
    pub node_version: Option<String>,
    pub domain: Option<String>,
    pub web_server: Option<String>,
    pub database: Option<String>,
    pub ssl_enabled: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub level: &'static str,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct PortInfo {
    pub port: u16,
    pub configured_for: String,
    pub available: bool,
}

pub struct Orchest {
    root: PathBuf,
    catalog: Catalog,
}
const BUILTIN_PHP_MANIFEST: &str = include_str!("../../../manifests/php.toml");
const BUILTIN_MAILPIT_MANIFEST: &str = include_str!("../../../manifests/mailpit.toml");
impl Orchest {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn open(root: PathBuf, manifest_dir: &Path) -> Result<Self> {
        if !root.join("config/orchest.toml").is_file() {
            return Err(OrchestError::NotInitialized(root));
        }
        let catalog = Catalog::load(manifest_dir)?;
        Ok(Self { root, catalog })
    }
    pub fn init(root: PathBuf, manifest_dir: &Path) -> Result<Self> {
        ensure_layout(&root)?;
        if manifest_dir.is_dir() {
            for entry in fs::read_dir(manifest_dir)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|ext| ext == "toml") {
                    let target = root.join("config/packages").join(entry.file_name());
                    if !target.exists() {
                        fs::copy(entry.path(), target)?;
                    }
                }
            }
        }
        let builtin_path = root.join("config/packages/php.toml");
        if !builtin_path.exists() {
            atomic_write(&builtin_path, BUILTIN_PHP_MANIFEST.as_bytes())?;
        } else {
            let mut current: orchest_packages::Manifest =
                toml::from_str(&fs::read_to_string(&builtin_path)?)
                    .map_err(|e| OrchestError::Config(e.to_string()))?;
            if current.package.id == "php" {
                let builtin: orchest_packages::Manifest = toml::from_str(BUILTIN_PHP_MANIFEST)
                    .map_err(|e| OrchestError::Config(e.to_string()))?;
                let mut changed = false;
                for bundled in builtin.versions {
                    if let Some(existing) = current
                        .versions
                        .iter_mut()
                        .find(|version| version.version == bundled.version)
                    {
                        if let (Some(installed), Some(released)) = (
                            existing.platforms.get_mut("linux-x86_64"),
                            bundled.platforms.get("linux-x86_64"),
                        ) {
                            let old_default = format!(
                                "https://github.com/{{github_repository}}/releases/download/php-{}-linux-x86_64-r1/php-{}-linux-x86_64.tar.gz",
                                bundled.version, bundled.version
                            );
                            if installed.url == old_default {
                                installed.url = released.url.clone();
                                changed = true;
                            }
                        }
                    } else {
                        current.versions.push(bundled);
                        changed = true;
                    }
                }
                if changed {
                    atomic_write(
                        &builtin_path,
                        toml::to_string_pretty(&current)
                            .map_err(|e| OrchestError::Config(e.to_string()))?
                            .as_bytes(),
                    )?;
                }
            }
        }
        let mailpit_path = root.join("config/packages/mailpit.toml");
        if !mailpit_path.exists() {
            atomic_write(&mailpit_path, BUILTIN_MAILPIT_MANIFEST.as_bytes())?;
        }
        let config_path = root.join("config/orchest.toml");
        if !config_path.exists() {
            atomic_write(
                &config_path,
                toml::to_string_pretty(&Config::default())
                    .map_err(|e| OrchestError::Config(e.to_string()))?
                    .as_bytes(),
            )?;
        }
        let catalog = Catalog::load(&root.join("config/packages"))?;
        let orchest = Self { root, catalog };
        orchest.connect()?;
        Ok(orchest)
    }
    fn connect(&self) -> Result<Connection> {
        let db = self.root.join("config/orchest.db");
        let connection = Connection::open(db)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS installations (package TEXT NOT NULL, version TEXT NOT NULL, platform TEXT NOT NULL, installed_at TEXT NOT NULL, source_url TEXT NOT NULL, executable TEXT NOT NULL, PRIMARY KEY(package,version));
            CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, path TEXT NOT NULL UNIQUE, php_version TEXT, node_version TEXT, domain TEXT, web_server TEXT, database_binding TEXT, ssl_enabled INTEGER NOT NULL DEFAULT 0);")?;
        Ok(connection)
    }
    pub fn config(&self) -> Result<Config> {
        toml::from_str(&fs::read_to_string(self.root.join("config/orchest.toml"))?)
            .map_err(|e| OrchestError::Config(e.to_string()))
    }
    pub fn config_set(&self, key: &str, value: &str) -> Result<Config> {
        let mut config = self.config()?;
        match key {
            "defaults.php" => {
                config.defaults.php = Some(self.resolve_installed("php", value)?.version);
            }
            "defaults.web_server" if ["nginx", "apache"].contains(&value) => {
                config.defaults.web_server = value.into()
            }
            "defaults.domain_suffix"
                if !value.is_empty()
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-') =>
            {
                config.defaults.domain_suffix = value.into()
            }
            "sources.github_repository" if valid_github_repository(value) => {
                config.sources.github_repository = Some(value.into());
            }
            key if key.starts_with("ports.") => {
                let port: u16 = value
                    .parse()
                    .map_err(|_| OrchestError::InvalidInput("port must be 1..65535".into()))?;
                if port == 0 {
                    return Err(OrchestError::InvalidInput("port must be 1..65535".into()));
                }
                match key {
                    "ports.nginx_http" => config.ports.nginx_http = port,
                    "ports.nginx_https" => config.ports.nginx_https = port,
                    "ports.mysql" => config.ports.mysql = port,
                    "ports.mariadb" => config.ports.mariadb = port,
                    "ports.mongodb" => config.ports.mongodb = port,
                    "ports.redis" => config.ports.redis = port,
                    "ports.mailpit_http" => config.ports.mailpit_http = port,
                    "ports.mailpit_smtp" => config.ports.mailpit_smtp = port,
                    _ => {
                        return Err(OrchestError::InvalidInput(format!(
                            "unknown port key: {key}"
                        )))
                    }
                }
            }
            _ => {
                return Err(OrchestError::InvalidInput(format!(
                    "unsupported config key or value: {key}"
                )))
            }
        }
        atomic_write(
            &self.root.join("config/orchest.toml"),
            toml::to_string_pretty(&config)
                .map_err(|e| OrchestError::Config(e.to_string()))?
                .as_bytes(),
        )?;
        Ok(config)
    }
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
    pub fn installed(&self, package: Option<&str>) -> Result<Vec<Installation>> {
        let db = self.connect()?;
        let mut statement = db.prepare("SELECT package,version,platform,installed_at,source_url,executable FROM installations ORDER BY package,version")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (name, version, platform, date, url, executable) = row?;
            if package.is_some_and(|wanted| wanted != name) {
                continue;
            }
            let installed_at = DateTime::parse_from_rfc3339(&date)
                .map_err(|e| OrchestError::Config(e.to_string()))?
                .with_timezone(&Utc);
            result.push(Installation {
                package: name,
                version,
                platform,
                installed_at,
                source_url: url,
                executable: executable.into(),
            });
        }
        Ok(result)
    }
    fn resolve_installed(&self, package: &str, requested: &str) -> Result<Installation> {
        let installations = self.installed(Some(package))?;
        let mut matches: Vec<_> = installations
            .into_iter()
            .filter(|i| i.version == requested || i.version.starts_with(&format!("{requested}.")))
            .collect();
        matches.sort_by_key(|a| numeric_version(&a.version));
        matches
            .pop()
            .ok_or_else(|| OrchestError::RuntimeNotInstalled(format!("{package}@{requested}")))
    }
    pub fn install(&self, package: &str, version: &str) -> Result<Installation> {
        if let Ok(existing) = self.resolve_installed(package, version) {
            if existing.version == version && existing.executable.is_file() {
                return Ok(existing);
            }
        }
        let target = Platform::current()?.target()?;
        let artifact = self.resolved_artifact(package, version, target)?;
        let destination = self.root.join("bin").join(package).join(version);
        if destination.exists() && !destination.join(&artifact.executable).is_file() {
            return Err(OrchestError::Config(format!(
                "incomplete installation at {}; inspect it before retrying",
                destination.display()
            )));
        }
        orchest_packages::install(&artifact, &destination, &self.root.join("cache/downloads"))?;
        self.register_installation(package, version, target, artifact.url, &artifact.executable)
    }
    pub fn install_from_archive(
        &self,
        package: &str,
        version: &str,
        archive: &Path,
    ) -> Result<Installation> {
        let target = Platform::current()?.target()?;
        let artifact = self.catalog.artifact(package, version, target)?;
        let archive = archive.canonicalize()?;
        let destination = self.root.join("bin").join(package).join(version);
        if destination.exists() && !destination.join(&artifact.executable).is_file() {
            return Err(OrchestError::Config(format!(
                "incomplete installation at {}; inspect it before retrying",
                destination.display()
            )));
        }
        orchest_packages::install_archive(artifact, &archive, &destination)?;
        self.register_installation(
            package,
            version,
            target,
            format!("local:{}", archive.display()),
            &artifact.executable,
        )
    }
    fn register_installation(
        &self,
        package: &str,
        version: &str,
        target: &str,
        source_url: String,
        executable_relative: &str,
    ) -> Result<Installation> {
        if package == "php" {
            let package_root = self.root.join("bin/php").join(version);
            let ini = package_root.join("php.ini");
            if !ini.exists() {
                let development = package_root.join("php.ini-development");
                if development.is_file() {
                    atomic_write(&ini, &fs::read(development)?)?;
                }
            }
            fs::create_dir_all(package_root.join("conf.d"))?;
        }
        let installation = Installation {
            package: package.into(),
            version: version.into(),
            platform: target.into(),
            installed_at: Utc::now(),
            source_url,
            executable: self
                .root
                .join("bin")
                .join(package)
                .join(version)
                .join(executable_relative),
        };
        self.connect()?.execute("INSERT OR REPLACE INTO installations (package,version,platform,installed_at,source_url,executable) VALUES (?1,?2,?3,?4,?5,?6)", params![installation.package, installation.version, installation.platform, installation.installed_at.to_rfc3339(), installation.source_url, installation.executable.to_string_lossy()])?;
        Ok(installation)
    }
    fn resolved_artifact(&self, package: &str, version: &str, target: &str) -> Result<Artifact> {
        let mut artifact = self.catalog.artifact(package, version, target)?.clone();
        if artifact.url.contains("{github_repository}") {
            let repository = self.config()?.sources.github_repository.ok_or_else(|| OrchestError::Config("set sources.github_repository to owner/repo before installing Linux release artifacts".into()))?;
            artifact.url = artifact.url.replace("{github_repository}", &repository);
        }
        Ok(artifact)
    }
    pub fn remove(&self, package: &str, version: &str, force: bool) -> Result<()> {
        let installation = self.resolve_installed(package, version)?;
        if installation.version != version {
            return Err(OrchestError::InvalidInput(
                "removal requires an exact version".into(),
            ));
        }
        if package == "mailpit"
            && self.mailpit_status()? == ServiceStatus::Running
            && self.supervisor().state("mailpit")?.is_some_and(|state| {
                installation.executable.canonicalize().ok() == Some(state.executable)
            })
        {
            return Err(OrchestError::PackageInUse(
                "mailpit service is running".into(),
            ));
        }
        if package == "php" {
            let config = self.config()?;
            let users: Vec<_> = self
                .projects()?
                .into_iter()
                .filter(|p| p.php_version.as_deref() == Some(version))
                .map(|p| p.name)
                .collect();
            if !force && (config.defaults.php.as_deref() == Some(version) || !users.is_empty()) {
                return Err(OrchestError::PackageInUse(format!(
                    "global default: {}; projects: {}",
                    config.defaults.php.as_deref() == Some(version),
                    users.join(", ")
                )));
            }
            if force && config.defaults.php.as_deref() == Some(version) {
                let mut updated = config;
                updated.defaults.php = None;
                atomic_write(
                    &self.root.join("config/orchest.toml"),
                    toml::to_string_pretty(&updated)
                        .map_err(|e| OrchestError::Config(e.to_string()))?
                        .as_bytes(),
                )?;
            }
            if force {
                self.connect()?.execute(
                    "UPDATE projects SET php_version=NULL WHERE php_version=?1",
                    [version],
                )?;
            }
        }
        fs::remove_dir_all(self.root.join("bin").join(package).join(version))?;
        self.connect()?.execute(
            "DELETE FROM installations WHERE package=?1 AND version=?2",
            params![package, version],
        )?;
        Ok(())
    }
    pub fn projects(&self) -> Result<Vec<Project>> {
        let db = self.connect()?;
        let mut statement = db.prepare("SELECT id,name,path,php_version,node_version,domain,web_server,database_binding,ssl_enabled FROM projects ORDER BY name")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, i64>(8)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (
                id,
                name,
                path,
                php_version,
                node_version,
                domain,
                web_server,
                database,
                ssl_enabled,
            ) = row?;
            result.push(Project {
                id: Uuid::parse_str(&id).map_err(|e| OrchestError::Config(e.to_string()))?,
                name,
                path: path.into(),
                php_version,
                node_version,
                domain,
                web_server,
                database,
                ssl_enabled: ssl_enabled != 0,
            });
        }
        Ok(result)
    }
    pub fn project(&self, name: &str) -> Result<Project> {
        self.projects()?
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| OrchestError::ProjectNotFound(name.into()))
    }
    pub fn add_project(&self, path: &Path, name: &str) -> Result<Project> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(OrchestError::InvalidInput(
                "project name must contain only letters, digits, _ or -".into(),
            ));
        }
        let path = path.canonicalize()?;
        if !path.is_dir() {
            return Err(OrchestError::InvalidInput(
                "project path must be a directory".into(),
            ));
        }
        let project = Project {
            id: Uuid::new_v4(),
            name: name.into(),
            path,
            php_version: None,
            node_version: None,
            domain: None,
            web_server: None,
            database: None,
            ssl_enabled: false,
        };
        let changed = self.connect()?.execute(
            "INSERT OR IGNORE INTO projects (id,name,path,ssl_enabled) VALUES (?1,?2,?3,0)",
            params![
                project.id.to_string(),
                project.name,
                project.path.to_string_lossy()
            ],
        )?;
        if changed == 0 {
            return Err(OrchestError::ProjectExists(name.into()));
        }
        Ok(project)
    }
    pub fn add_project_default(&self, name: &str) -> Result<Project> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(OrchestError::InvalidInput(
                "project name must contain only letters, digits, _ or -".into(),
            ));
        }
        if self.projects()?.iter().any(|project| project.name == name) {
            return Err(OrchestError::ProjectExists(name.into()));
        }
        let path = self.root.join("www").join(name);
        fs::create_dir_all(&path)?;
        self.add_project(&path, name)
    }
    pub fn set_project_php(&self, name: &str, requested: &str) -> Result<Project> {
        self.project(name)?;
        let installed = self.resolve_installed("php", requested)?;
        self.connect()?.execute(
            "UPDATE projects SET php_version=?1 WHERE name=?2",
            params![installed.version, name],
        )?;
        self.project(name)
    }
    pub fn resolve_php(&self, project: Option<&Project>) -> Result<Installation> {
        let requested = project
            .and_then(|p| p.php_version.as_deref())
            .map(str::to_owned)
            .or(self.config()?.defaults.php)
            .ok_or(OrchestError::NoDefaultPhp)?;
        let installed = self.resolve_installed("php", &requested)?;
        if !installed.executable.is_file() {
            return Err(OrchestError::RuntimeNotInstalled(format!(
                "php@{}: executable missing",
                installed.version
            )));
        }
        Ok(installed)
    }
    pub fn exec_php(&self, project: Option<&Project>, args: &[OsString]) -> Result<i32> {
        let php = self.resolve_php(project)?;
        let package_root = self.root.join("bin/php").join(&php.version);
        let scan_dir = package_root.join("conf.d");
        let parent = php
            .executable
            .parent()
            .ok_or_else(|| OrchestError::InvalidInput("invalid PHP binary path".into()))?;
        let status = orchest_process::run_with_env(
            &php.executable,
            args,
            project.map(|p| p.path.as_path()),
            &[parent],
            &[("PHPRC", &package_root), ("PHP_INI_SCAN_DIR", &scan_dir)],
        )?;
        Ok(status.code().unwrap_or(1))
    }
    pub fn exec_php_capture(
        &self,
        project: Option<&Project>,
        args: &[OsString],
    ) -> Result<(i32, String, String)> {
        let php = self.resolve_php(project)?;
        let package_root = self.root.join("bin/php").join(&php.version);
        let scan_dir = package_root.join("conf.d");
        let parent = php
            .executable
            .parent()
            .ok_or_else(|| OrchestError::InvalidInput("invalid PHP binary path".into()))?;
        let output = orchest_process::run_capture_with_env(
            &php.executable,
            args,
            project.map(|p| p.path.as_path()),
            &[parent],
            &[("PHPRC", &package_root), ("PHP_INI_SCAN_DIR", &scan_dir)],
        )?;
        Ok((
            output.status.code().unwrap_or(1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
    fn supervisor(&self) -> Supervisor {
        Supervisor::new(
            self.root.join("runtime/state/services"),
            self.root.join("logs/services"),
        )
    }
    pub fn mailpit_status(&self) -> Result<ServiceStatus> {
        Ok(self.supervisor().status("mailpit")?)
    }
    pub fn start_mailpit(&self) -> Result<ProcessState> {
        let supervisor = self.supervisor();
        if supervisor.status("mailpit")? == ServiceStatus::Running {
            return Err(OrchestError::PackageInUse(
                "mailpit service is running".into(),
            ));
        }
        let installation = self
            .installed(Some("mailpit"))?
            .into_iter()
            .max_by_key(|entry| numeric_version(&entry.version))
            .ok_or_else(|| OrchestError::RuntimeNotInstalled("mailpit".into()))?;
        let ports = self.config()?.ports;
        for port in [ports.mailpit_http, ports.mailpit_smtp] {
            if !port_available(port) {
                return Err(OrchestError::InvalidInput(format!(
                    "port {port} is unavailable for Mailpit"
                )));
            }
        }
        let data_dir = self.root.join("data/mailpit");
        fs::create_dir_all(&data_dir)?;
        let args: Vec<OsString> = vec![
            "--listen".into(),
            format!("127.0.0.1:{}", ports.mailpit_http).into(),
            "--smtp".into(),
            format!("127.0.0.1:{}", ports.mailpit_smtp).into(),
            "--database".into(),
            data_dir.join("mailpit.db").into_os_string(),
        ];
        Ok(supervisor.start("mailpit", &installation.executable, &args, Some(&data_dir))?)
    }
    pub fn stop_mailpit(&self) -> Result<ServiceStatus> {
        self.supervisor().stop("mailpit")?;
        self.mailpit_status()
    }
    pub fn doctor(&self) -> Vec<Check> {
        let mut checks = Vec::new();
        checks.push(match check_root_writable(&self.root) {
            Ok(()) => Check {
                name: "root".into(),
                level: "ok",
                detail: format!("writable: {}", self.root.display()),
            },
            Err(error) => Check {
                name: "root".into(),
                level: "error",
                detail: format!("{}: {error}", self.root.display()),
            },
        });
        checks.push(match self.config() {
            Ok(_) => Check {
                name: "configuration".into(),
                level: "ok",
                detail: "readable".into(),
            },
            Err(e) => Check {
                name: "configuration".into(),
                level: "error",
                detail: e.to_string(),
            },
        });
        checks.push(
            match check_database_integrity(&self.root.join("config/orchest.db")) {
                Ok(()) => Check {
                    name: "database".into(),
                    level: "ok",
                    detail: "SQLite quick_check passed".into(),
                },
                Err(error) => Check {
                    name: "database".into(),
                    level: "error",
                    detail: error,
                },
            },
        );
        match self.installed(None) {
            Ok(installs) => {
                for install in installs {
                    checks.push(Check {
                        name: format!("{}@{}", install.package, install.version),
                        level: if install.executable.is_file() {
                            "ok"
                        } else {
                            "error"
                        },
                        detail: install.executable.display().to_string(),
                    });
                }
            }
            Err(e) => checks.push(Check {
                name: "package state".into(),
                level: "error",
                detail: e.to_string(),
            }),
        }
        let supervisor = self.supervisor();
        match supervisor.services() {
            Ok(services) => {
                for service in services {
                    match supervisor.instances(&service) {
                        Ok(instances) => {
                            for instance in instances {
                                checks.push(Check {
                                    name: format!("service {service}/{}", instance.instance_id),
                                    level: if instance.status == ServiceStatus::Stale {
                                        "warn"
                                    } else {
                                        "ok"
                                    },
                                    detail: format!(
                                        "{:?} (PID {})",
                                        instance.status, instance.process.pid
                                    )
                                    .to_lowercase(),
                                });
                            }
                        }
                        Err(error) => checks.push(Check {
                            name: format!("service {service}"),
                            level: "error",
                            detail: error.to_string(),
                        }),
                    }
                }
            }
            Err(error) => checks.push(Check {
                name: "service state".into(),
                level: "error",
                detail: error.to_string(),
            }),
        }
        match self.ports() {
            Ok(ports) => {
                let mut configured = std::collections::BTreeMap::<u16, Vec<String>>::new();
                for port in &ports {
                    configured
                        .entry(port.port)
                        .or_default()
                        .push(port.configured_for.clone());
                }
                let mailpit_running = self
                    .mailpit_status()
                    .is_ok_and(|status| status == ServiceStatus::Running);
                for port in ports {
                    let duplicate = configured
                        .get(&port.port)
                        .is_some_and(|owners| owners.len() > 1);
                    let managed = mailpit_running
                        && matches!(
                            port.configured_for.as_str(),
                            "mailpit_http" | "mailpit_smtp"
                        );
                    checks.push(Check {
                        name: format!("port {} ({})", port.port, port.configured_for),
                        level: if duplicate || (!port.available && !managed) {
                            "warn"
                        } else {
                            "ok"
                        },
                        detail: if duplicate {
                            format!(
                                "configured for multiple services: {}",
                                configured[&port.port].join(", ")
                            )
                        } else if managed && !port.available {
                            "occupied by managed Mailpit".into()
                        } else if !port.available {
                            "unavailable (occupied or permission denied)".into()
                        } else {
                            "available".into()
                        },
                    });
                }
            }
            Err(error) => checks.push(Check {
                name: "ports".into(),
                level: "error",
                detail: error.to_string(),
            }),
        }
        match self.projects() {
            Ok(projects) => {
                for project in projects {
                    checks.push(Check {
                        name: format!("project {}", project.name),
                        level: if project.path.is_dir() { "ok" } else { "warn" },
                        detail: project.path.display().to_string(),
                    });
                }
            }
            Err(error) => checks.push(Check {
                name: "project state".into(),
                level: "error",
                detail: error.to_string(),
            }),
        }
        if let Ok(target) = Platform::current().and_then(Platform::target) {
            for manifest in self.catalog.list() {
                if manifest.package.id == "php"
                    && manifest
                        .versions
                        .iter()
                        .all(|v| !v.platforms.contains_key(target))
                {
                    checks.push(Check {
                        name: "PHP catalog".into(),
                        level: "warn",
                        detail: format!("no managed PHP artifacts for {target}"),
                    });
                }
            }
            if target == "linux-x86_64"
                && self.catalog.list().iter().any(|manifest| {
                    manifest.versions.iter().any(|version| {
                        version
                            .platforms
                            .get(target)
                            .is_some_and(|artifact| artifact.url.contains("{github_repository}"))
                    })
                })
                && self
                    .config()
                    .is_ok_and(|c| c.sources.github_repository.is_none())
            {
                checks.push(Check {
                    name: "release repository".into(),
                    level: "warn",
                    detail: "set sources.github_repository to owner/repo".into(),
                });
            }
        }
        checks
    }

    pub fn ports(&self) -> Result<Vec<PortInfo>> {
        let ports = self.config()?.ports;
        Ok([
            ("nginx_http", ports.nginx_http),
            ("nginx_https", ports.nginx_https),
            ("mysql", ports.mysql),
            ("mariadb", ports.mariadb),
            ("mongodb", ports.mongodb),
            ("redis", ports.redis),
            ("mailpit_http", ports.mailpit_http),
            ("mailpit_smtp", ports.mailpit_smtp),
        ]
        .into_iter()
        .map(|(name, port)| PortInfo {
            port,
            configured_for: name.into(),
            available: port_available(port),
        })
        .collect())
    }
    pub fn check_port(&self, port: u16) -> Result<PortInfo> {
        if port == 0 {
            return Err(OrchestError::InvalidInput("port must be 1..65535".into()));
        }
        let owner = self
            .ports()?
            .into_iter()
            .find(|item| item.port == port)
            .map(|item| item.configured_for)
            .unwrap_or_else(|| "unassigned".into());
        Ok(PortInfo {
            port,
            configured_for: owner,
            available: port_available(port),
        })
    }
}

fn check_root_writable(root: &Path) -> std::io::Result<()> {
    let path = root.join(format!(".orchest-doctor-{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let result = file.write_all(b"orchest doctor");
    drop(file);
    let cleanup = fs::remove_file(path);
    result.and(cleanup)
}

fn check_database_integrity(path: &Path) -> std::result::Result<(), String> {
    if !path.is_file() {
        return Err(format!("database is missing: {}", path.display()));
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let result: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if result == "ok" {
        Ok(())
    } else {
        Err(format!("SQLite quick_check: {result}"))
    }
}

fn port_available(port: u16) -> bool {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok()
}

fn valid_github_repository(value: &str) -> bool {
    let Some((owner, repo)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && !repo.is_empty()
        && !repo.contains('/')
        && [owner, repo].iter().all(|part| {
            part.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphanumeric())
                && part
                    .bytes()
                    .last()
                    .is_some_and(|b| b.is_ascii_alphanumeric())
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
                && !part.contains("..")
        })
}

fn numeric_version(value: &str) -> Vec<u64> {
    value.split('.').map(|s| s.parse().unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let manifest = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), manifest.path()).unwrap();
        let project = app.add_project(root.path(), "example").unwrap();
        assert_eq!(app.project("example").unwrap().id, project.id);
    }
    #[test]
    fn default_project_lives_in_www_and_external_paths_still_work() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), &root.path().join("missing")).unwrap();
        let default = app.add_project_default("site").unwrap();
        assert_eq!(
            default.path,
            root.path().join("www/site").canonicalize().unwrap()
        );
        assert!(default.path.is_dir());
        let external_project = app.add_project(external.path(), "external").unwrap();
        assert_eq!(
            external_project.path,
            external.path().canonicalize().unwrap()
        );
    }
    #[test]
    fn initialization_uses_embedded_catalog_without_source_tree() {
        let root = tempfile::tempdir().unwrap();
        let app = Orchest::init(
            root.path().to_path_buf(),
            &root.path().join("missing-manifests"),
        )
        .unwrap();
        assert!(app.catalog().get("php").is_ok());
        assert!(app.catalog().get("mailpit").is_ok());
    }
    #[test]
    fn init_adds_new_php_versions_without_replacing_custom_versions() {
        let root = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), &root.path().join("missing")).unwrap();
        let manifest_path = root.path().join("config/packages/php.toml");
        let mut manifest: orchest_packages::Manifest =
            toml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest.versions.retain(|v| v.version != "8.2.33");
        manifest.versions[0]
            .platforms
            .get_mut("linux-x86_64")
            .unwrap()
            .url = "https://example.test/custom.tar.gz".into();
        fs::write(&manifest_path, toml::to_string(&manifest).unwrap()).unwrap();
        drop(app);
        let app = Orchest::init(root.path().to_path_buf(), &root.path().join("missing")).unwrap();
        assert!(app
            .catalog()
            .artifact("php", "8.2.33", "linux-x86_64")
            .is_ok());
        assert_eq!(
            app.catalog()
                .artifact("php", "8.3.33", "linux-x86_64")
                .unwrap()
                .url,
            "https://example.test/custom.tar.gz"
        );
    }
    #[test]
    fn old_config_receives_mailpit_port_defaults() {
        let old = "[defaults]\nweb_server='nginx'\ndomain_suffix='test'\n[ports]\nnginx_http=80\nnginx_https=443\nmysql=3306\nmariadb=3307\nmongodb=27017\nredis=6379\n";
        let config: Config = toml::from_str(old).unwrap();
        assert_eq!(config.ports.mailpit_http, 8025);
        assert_eq!(config.ports.mailpit_smtp, 1025);
    }
    #[cfg(unix)]
    #[test]
    fn global_and_project_php_resolution() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let manifest = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), manifest.path()).unwrap();
        for version in ["8.4.15", "8.5.10"] {
            let executable = root.path().join("bin/php").join(version).join("php");
            fs::create_dir_all(executable.parent().unwrap()).unwrap();
            fs::write(&executable, format!("#!/bin/sh\necho {version}\n")).unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
            app.connect().unwrap().execute("INSERT INTO installations (package,version,platform,installed_at,source_url,executable) VALUES ('php',?1,'linux-x86_64',?2,'https://example.test/php',?3)", params![version,Utc::now().to_rfc3339(),executable.to_str().unwrap()]).unwrap();
        }
        app.config_set("defaults.php", "8.5").unwrap();
        let project = app.add_project(root.path(), "legacy").unwrap();
        let project = app.set_project_php(&project.name, "8.4").unwrap();
        assert_eq!(app.resolve_php(None).unwrap().version, "8.5.10");
        assert_eq!(app.resolve_php(Some(&project)).unwrap().version, "8.4.15");
        assert_eq!(
            app.exec_php_capture(Some(&project), &[]).unwrap().1.trim(),
            "8.4.15"
        );
        assert!(matches!(
            app.remove("php", "8.4.15", false),
            Err(OrchestError::PackageInUse(_))
        ));
    }
    #[test]
    fn port_configuration_and_lookup() {
        let root = tempfile::tempdir().unwrap();
        let manifest = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), manifest.path()).unwrap();
        app.config_set("ports.redis", "16379").unwrap();
        assert_eq!(app.check_port(16379).unwrap().configured_for, "redis");
        assert!(app.config_set("ports.redis", "0").is_err());
    }
    #[test]
    fn doctor_reports_database_corruption_without_failing_to_open() {
        let root = tempfile::tempdir().unwrap();
        Orchest::init(root.path().to_path_buf(), &root.path().join("missing")).unwrap();
        fs::write(
            root.path().join("config/orchest.db"),
            b"not a sqlite database",
        )
        .unwrap();
        let app = Orchest::open(
            root.path().to_path_buf(),
            &root.path().join("config/packages"),
        )
        .unwrap();
        let checks = app.doctor();
        assert!(checks
            .iter()
            .any(|check| check.name == "database" && check.level == "error"));
        assert!(checks
            .iter()
            .any(|check| check.name == "package state" && check.level == "error"));
    }
    #[test]
    fn doctor_reports_occupied_ports_and_stale_instances() {
        let root = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), &root.path().join("missing")).unwrap();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        app.config_set("ports.redis", &port.to_string()).unwrap();
        let state_path = root.path().join("runtime/state/services/worker/one.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let state = ProcessState {
            service_id: "worker".into(),
            instance_id: "one".into(),
            pid: u32::MAX,
            executable: root.path().join("bin/worker"),
            started_at: Utc::now(),
            process_start_time: 0,
        };
        fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
        let checks = app.doctor();
        assert!(checks
            .iter()
            .any(|check| check.name == "root" && check.level == "ok"));
        assert!(checks
            .iter()
            .any(|check| check.name == "database" && check.level == "ok"));
        assert!(checks
            .iter()
            .any(|check| check.name == format!("port {port} (redis)") && check.level == "warn"));
        assert!(checks
            .iter()
            .any(|check| check.name == "service worker/one" && check.level == "warn"));
        assert!(state_path.exists());
    }
    #[test]
    fn github_release_url_requires_configured_repository() {
        let root = tempfile::tempdir().unwrap();
        let manifests = tempfile::tempdir().unwrap();
        fs::write(manifests.path().join("php.toml"), "[package]\nid='php'\nname='PHP'\ntype='runtime'\n[[versions]]\nversion='8.4.15'\n[versions.linux-x86_64]\nurl='https://github.com/{github_repository}/releases/download/php-8.4.15-linux-x86_64-r1/php.tar.gz'\narchive='tar.gz'\nexecutable='bin/php'").unwrap();
        let app = Orchest::init(root.path().to_path_buf(), manifests.path()).unwrap();
        assert!(app
            .resolved_artifact("php", "8.4.15", "linux-x86_64")
            .is_err());
        assert!(app
            .config_set("sources.github_repository", "../other")
            .is_err());
        app.config_set("sources.github_repository", "example/orchest")
            .unwrap();
        let artifact = app
            .resolved_artifact("php", "8.4.15", "linux-x86_64")
            .unwrap();
        assert!(artifact
            .url
            .starts_with("https://github.com/example/orchest/releases/"));
    }
}
