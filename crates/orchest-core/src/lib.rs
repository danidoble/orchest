//! Reusable Orchest application core.
use chrono::{DateTime, Utc};
use orchest_packages::{Artifact, Catalog, PackageError};
use orchest_platform::{atomic_write, ensure_layout, Platform};
use orchest_process::{ProcessState, ServiceStatus, Supervisor};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
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
    #[serde(default = "default_meilisearch_http_port")]
    pub meilisearch_http: u16,
}
fn default_mailpit_http_port() -> u16 {
    8025
}
fn default_mailpit_smtp_port() -> u16 {
    1025
}
fn default_meilisearch_http_port() -> u16 {
    7700
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
                meilisearch_http: default_meilisearch_http_port(),
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
    pub owner: Option<String>,
    pub state: PortState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortState {
    Available,
    Managed,
    Reserved,
    StaleClaim,
    Unavailable,
}

pub struct Orchest {
    root: PathBuf,
    catalog: Catalog,
}
struct ManagedLaunch<'a> {
    ports: &'a [u16],
    args: &'a [OsString],
    data_dir: &'a Path,
    envs: &'a [(&'a str, &'a Path)],
}
const BUILTIN_PHP_MANIFEST: &str = include_str!("../../../manifests/php.toml");
const BUILTIN_MAILPIT_MANIFEST: &str = include_str!("../../../manifests/mailpit.toml");
const BUILTIN_MEILISEARCH_MANIFEST: &str = include_str!("../../../manifests/meilisearch.toml");
const BUILTIN_NGINX_MANIFEST: &str = include_str!("../../../manifests/nginx.toml");
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
        let meilisearch_path = root.join("config/packages/meilisearch.toml");
        if !meilisearch_path.exists() {
            atomic_write(&meilisearch_path, BUILTIN_MEILISEARCH_MANIFEST.as_bytes())?;
        }
        let nginx_path = root.join("config/packages/nginx.toml");
        if !nginx_path.exists() {
            atomic_write(&nginx_path, BUILTIN_NGINX_MANIFEST.as_bytes())?;
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
            CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, path TEXT NOT NULL UNIQUE, php_version TEXT, node_version TEXT, domain TEXT, web_server TEXT, database_binding TEXT, ssl_enabled INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS port_claims (port INTEGER PRIMARY KEY, service_id TEXT NOT NULL, instance_id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS php_web_ports (version TEXT PRIMARY KEY, port INTEGER NOT NULL UNIQUE);")?;
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
                    "ports.meilisearch_http" => config.ports.meilisearch_http = port,
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
        if self
            .supervisor()
            .instances(package)?
            .iter()
            .any(|instance| {
                instance.status == ServiceStatus::Running
                    && installation.executable.canonicalize().ok()
                        == Some(instance.process.executable.clone())
            })
        {
            return Err(OrchestError::PackageInUse(format!(
                "{package} service is running"
            )));
        }
        if package == "php"
            && self
                .supervisor()
                .status_instance("php-web", &php_web_instance_id(version))?
                == ServiceStatus::Running
        {
            return Err(OrchestError::PackageInUse(format!(
                "php@{version} FastCGI backend is running"
            )));
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
        if package == "php" {
            self.connect()?
                .execute("DELETE FROM php_web_ports WHERE version=?1", [version])?;
        }
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
    fn start_managed_service(
        &self,
        service_id: &str,
        instance_id: &str,
        installation: &Installation,
        ports: &[u16],
        args: &[OsString],
        data_dir: &Path,
    ) -> Result<ProcessState> {
        self.start_managed_service_with_env(
            service_id,
            instance_id,
            installation,
            ManagedLaunch {
                ports,
                args,
                data_dir,
                envs: &[],
            },
        )
    }
    fn start_managed_service_with_env(
        &self,
        service_id: &str,
        instance_id: &str,
        installation: &Installation,
        launch: ManagedLaunch<'_>,
    ) -> Result<ProcessState> {
        let supervisor = self.supervisor();
        if supervisor.status_instance(service_id, instance_id)? == ServiceStatus::Running {
            return Err(OrchestError::PackageInUse(format!(
                "{service_id}/{instance_id} is running"
            )));
        }
        let configured = configured_ports(&self.config()?.ports);
        let mut unique = std::collections::BTreeSet::new();
        for port in launch.ports {
            if !unique.insert(*port) {
                return Err(OrchestError::InvalidInput(format!(
                    "port {port} is assigned twice to {service_id}"
                )));
            }
            let assignments: Vec<_> = configured
                .iter()
                .filter(|(_, value)| value == port)
                .collect();
            if assignments.len() > 1 {
                return Err(OrchestError::InvalidInput(format!(
                    "port {port} is configured for multiple services"
                )));
            }
        }
        let mut db = self.connect()?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for port in launch.ports {
            let prior: Option<(String, String)> = transaction
                .query_row(
                    "SELECT service_id,instance_id FROM port_claims WHERE port=?1",
                    [port],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((owner_service, owner_instance)) = prior {
                if supervisor.status_instance(&owner_service, &owner_instance)?
                    == ServiceStatus::Running
                {
                    return Err(OrchestError::PackageInUse(format!(
                        "port {port} belongs to {owner_service}/{owner_instance}"
                    )));
                }
                transaction.execute("DELETE FROM port_claims WHERE port=?1", [port])?;
            }
            if !port_available(*port) {
                return Err(OrchestError::InvalidInput(format!(
                    "port {port} is unavailable"
                )));
            }
        }
        fs::create_dir_all(launch.data_dir)?;
        let process = supervisor.start_instance_with_env(
            service_id,
            instance_id,
            &installation.executable,
            launch.args,
            Some(launch.data_dir),
            launch.envs,
        )?;
        for port in launch.ports {
            if let Err(error) = transaction.execute(
                "INSERT INTO port_claims (port,service_id,instance_id) VALUES (?1,?2,?3)",
                params![port, service_id, instance_id],
            ) {
                let _ = supervisor.stop_instance(service_id, instance_id);
                return Err(error.into());
            }
        }
        if let Err(error) = transaction.commit() {
            let _ = supervisor.stop_instance(service_id, instance_id);
            return Err(error.into());
        }
        Ok(process)
    }
    fn stop_managed_service(&self, service_id: &str, instance_id: &str) -> Result<ServiceStatus> {
        self.supervisor().stop_instance(service_id, instance_id)?;
        self.connect()?.execute(
            "DELETE FROM port_claims WHERE service_id=?1 AND instance_id=?2",
            params![service_id, instance_id],
        )?;
        Ok(self.supervisor().status_instance(service_id, instance_id)?)
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
        let data_dir = self.root.join("data/mailpit");
        let args: Vec<OsString> = vec![
            "--listen".into(),
            format!("127.0.0.1:{}", ports.mailpit_http).into(),
            "--smtp".into(),
            format!("127.0.0.1:{}", ports.mailpit_smtp).into(),
            "--database".into(),
            data_dir.join("mailpit.db").into_os_string(),
        ];
        self.start_managed_service(
            "mailpit",
            "default",
            &installation,
            &[ports.mailpit_http, ports.mailpit_smtp],
            &args,
            &data_dir,
        )
    }
    pub fn stop_mailpit(&self) -> Result<ServiceStatus> {
        self.stop_managed_service("mailpit", "default")
    }
    pub fn meilisearch_status(&self) -> Result<ServiceStatus> {
        Ok(self.supervisor().status("meilisearch")?)
    }
    pub fn start_meilisearch(&self) -> Result<ProcessState> {
        let installation = self
            .installed(Some("meilisearch"))?
            .into_iter()
            .max_by_key(|entry| numeric_version(&entry.version))
            .ok_or_else(|| OrchestError::RuntimeNotInstalled("meilisearch".into()))?;
        let port = self.config()?.ports.meilisearch_http;
        let data_dir = self.root.join("data/meilisearch");
        let args: Vec<OsString> = vec![
            "--http-addr".into(),
            format!("127.0.0.1:{port}").into(),
            "--db-path".into(),
            data_dir.join("data.ms").into_os_string(),
            "--env".into(),
            "development".into(),
        ];
        self.start_managed_service(
            "meilisearch",
            "default",
            &installation,
            &[port],
            &args,
            &data_dir,
        )
    }
    pub fn stop_meilisearch(&self) -> Result<ServiceStatus> {
        self.stop_managed_service("meilisearch", "default")
    }
    fn php_web_port(&self, version: &str) -> Result<u16> {
        let mut db = self.connect()?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(port) = transaction
            .query_row(
                "SELECT port FROM php_web_ports WHERE version=?1",
                [version],
                |row| row.get::<_, u16>(0),
            )
            .optional()?
        {
            return Ok(port);
        }
        let configured: std::collections::BTreeSet<u16> = configured_ports(&self.config()?.ports)
            .into_iter()
            .map(|(_, port)| port)
            .collect();
        let mut selected = None;
        for port in 19000..=19999 {
            if configured.contains(&port) {
                continue;
            }
            let assigned: Option<u8> = transaction
                .query_row(
                    "SELECT 1 FROM php_web_ports WHERE port=?1 UNION SELECT 1 FROM port_claims WHERE port=?1",
                    [port],
                    |row| row.get(0),
                )
                .optional()?;
            if assigned.is_none() && port_available(port) {
                selected = Some(port);
                break;
            }
        }
        let port = selected.ok_or_else(|| {
            OrchestError::Config("no free PHP FastCGI port in 19000..19999".into())
        })?;
        transaction.execute(
            "INSERT INTO php_web_ports (version,port) VALUES (?1,?2)",
            params![version, port],
        )?;
        transaction.commit()?;
        Ok(port)
    }
    fn nginx_php_versions(&self) -> Result<std::collections::BTreeSet<String>> {
        let default = self.config()?.defaults.php;
        let mut versions = std::collections::BTreeSet::new();
        for project in self.projects()? {
            if project
                .web_server
                .as_deref()
                .is_some_and(|server| server != "nginx")
            {
                continue;
            }
            if let Some(version) = project.php_version.as_ref().or(default.as_ref()) {
                let installation = self.resolve_installed("php", version)?;
                versions.insert(installation.version);
            }
        }
        Ok(versions)
    }
    pub fn php_web_status(&self, version: &str) -> Result<ServiceStatus> {
        let installed = self.resolve_installed("php", version)?;
        Ok(self
            .supervisor()
            .status_instance("php-web", &php_web_instance_id(&installed.version))?)
    }
    pub fn start_php_web(&self, version: &str) -> Result<ProcessState> {
        let mut installation = self.resolve_installed("php", version)?;
        let package_root = self.root.join("bin/php").join(&installation.version);
        installation.executable = if cfg!(windows) {
            package_root.join("php-cgi.exe")
        } else {
            package_root.join("sbin/php-fpm")
        };
        if !installation.executable.is_file() {
            return Err(OrchestError::RuntimeNotInstalled(format!(
                "php@{version}: FastCGI executable missing at {}",
                installation.executable.display()
            )));
        }
        let instance_id = php_web_instance_id(&installation.version);
        if self.supervisor().status_instance("php-web", &instance_id)? == ServiceStatus::Running {
            return Err(OrchestError::PackageInUse(format!(
                "php-web/{version} is running"
            )));
        }
        let port = self.php_web_port(&installation.version)?;
        if configured_ports(&self.config()?.ports)
            .iter()
            .any(|(_, configured)| *configured == port)
        {
            return Err(OrchestError::Config(format!(
                "FastCGI port {port} for PHP {version} is also configured for another service"
            )));
        }
        let data_dir = self
            .root
            .join("runtime/generated/php-web")
            .join(&instance_id);
        fs::create_dir_all(&data_dir)?;
        let scan_dir = package_root.join("conf.d");
        fs::create_dir_all(&scan_dir)?;
        let ini = package_root.join("php.ini");
        let args: Vec<OsString> = if cfg!(windows) {
            vec![
                "-b".into(),
                format!("127.0.0.1:{port}").into(),
                "-c".into(),
                package_root.as_os_str().into(),
            ]
        } else {
            let user = std::env::var("USER").unwrap_or_else(|_| "nobody".into());
            if user.is_empty()
                || !user
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            {
                return Err(OrchestError::Config(
                    "invalid current user for PHP-FPM".into(),
                ));
            }
            let fpm_config = format!(
                "[global]\ndaemonize = no\nerror_log = \"{}\"\n[orchest]\nuser = {user}\nlisten = 127.0.0.1:{port}\nlisten.allowed_clients = 127.0.0.1\npm = ondemand\npm.max_children = 4\npm.process_idle_timeout = 10s\ncatch_workers_output = yes\n",
                nginx_path(&data_dir.join("php-fpm.log"))?
            );
            let path = data_dir.join("php-fpm.conf");
            atomic_write(&path, fpm_config.as_bytes())?;
            vec![
                "-F".into(),
                "-R".into(),
                "-y".into(),
                path.as_os_str().into(),
                "-c".into(),
                ini.as_os_str().into(),
            ]
        };
        let envs = if cfg!(windows) {
            vec![
                ("PHPRC", package_root.as_path()),
                ("PHP_INI_SCAN_DIR", scan_dir.as_path()),
                ("PHP_FCGI_MAX_REQUESTS", Path::new("0")),
            ]
        } else {
            vec![
                ("PHPRC", package_root.as_path()),
                ("PHP_INI_SCAN_DIR", scan_dir.as_path()),
            ]
        };
        let process = self.start_managed_service_with_env(
            "php-web",
            &instance_id,
            &installation,
            ManagedLaunch {
                ports: &[port],
                args: &args,
                data_dir: &data_dir,
                envs: &envs,
            },
        )?;
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        for _ in 0..30 {
            if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                return Ok(process);
            }
            if self.supervisor().status_instance("php-web", &instance_id)? != ServiceStatus::Running
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.stop_managed_service("php-web", &instance_id);
        Err(OrchestError::Config(format!(
            "PHP FastCGI {version} did not listen on 127.0.0.1:{port}; inspect logs/services/php-web/{instance_id}"
        )))
    }
    pub fn stop_php_web(&self, version: &str) -> Result<ServiceStatus> {
        let installed = self.resolve_installed("php", version)?;
        self.stop_managed_service("php-web", &php_web_instance_id(&installed.version))
    }
    pub fn nginx_status(&self) -> Result<ServiceStatus> {
        Ok(self.supervisor().status("nginx")?)
    }
    pub fn nginx_config(&self) -> Result<String> {
        let config = self.config()?;
        let default_php = config.defaults.php.as_deref();
        let suffix = &config.defaults.domain_suffix;
        if suffix.is_empty()
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
        {
            return Err(OrchestError::Config("invalid domain suffix".into()));
        }
        let mut output = format!(
            "worker_processes 1;\npid logs/nginx.pid;\nerror_log logs/error.log;\nevents {{ worker_connections 256; }}\nhttp {{\n    access_log logs/access.log;\n    server {{ listen 127.0.0.1:{} default_server; server_name _; return 404; }}\n",
            config.ports.nginx_http
        );
        let mut domains = std::collections::BTreeSet::new();
        for project in self.projects()? {
            if project
                .web_server
                .as_deref()
                .is_some_and(|server| server != "nginx")
            {
                continue;
            }
            let domain = project
                .domain
                .unwrap_or_else(|| format!("{}.{}", project.name, suffix))
                .to_ascii_lowercase();
            if domain.is_empty()
                || !domain
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
                || !domains.insert(domain.clone())
            {
                return Err(OrchestError::Config(format!(
                    "invalid or duplicate project domain: {domain}"
                )));
            }
            let root = nginx_path(&project.path)?;
            output.push_str(&format!(
                "    server {{\n        listen 127.0.0.1:{};\n        server_name {domain};\n        root \"{root}\";\n        location ~ /\\. {{ return 404; }}\n",
                config.ports.nginx_http
            ));
            if let Some(version) = project.php_version.as_deref().or(default_php) {
                let php = self.resolve_installed("php", version)?;
                let port = self.php_web_port(&php.version)?;
                output.push_str(&format!(
                    "        index index.php index.html index.htm;\n        location / {{ try_files $uri $uri/ /index.php?$query_string; }}\n        location ~* \\.php$ {{\n            try_files $uri =404;\n            fastcgi_pass 127.0.0.1:{port};\n            fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n            fastcgi_param SCRIPT_NAME $fastcgi_script_name;\n            fastcgi_param DOCUMENT_ROOT $document_root;\n            fastcgi_param QUERY_STRING $query_string;\n            fastcgi_param REQUEST_METHOD $request_method;\n            fastcgi_param CONTENT_TYPE $content_type;\n            fastcgi_param CONTENT_LENGTH $content_length;\n            fastcgi_param REQUEST_URI $request_uri;\n            fastcgi_param SERVER_PROTOCOL $server_protocol;\n            fastcgi_param SERVER_NAME $server_name;\n            fastcgi_param SERVER_PORT $server_port;\n            fastcgi_param HTTPS off;\n            fastcgi_param REDIRECT_STATUS 200;\n            fastcgi_param HTTP_AUTHORIZATION $http_authorization;\n        }}\n"
                ));
            } else {
                output.push_str("        index index.html index.htm;\n        location / { try_files $uri $uri/ =404; }\n");
            }
            output.push_str(
                "        location ~* \\.(?:phtml|phar|php[0-9]?|inc)(?:$|[./]) { return 404; }\n",
            );
            output.push_str("    }\n");
        }
        output.push_str("}\n");
        Ok(output)
    }
    pub fn start_nginx(&self) -> Result<ProcessState> {
        if self.nginx_status()? == ServiceStatus::Running {
            return Err(OrchestError::PackageInUse("nginx is running".into()));
        }
        let installation = self
            .installed(Some("nginx"))?
            .into_iter()
            .max_by_key(|entry| numeric_version(&entry.version))
            .ok_or_else(|| OrchestError::RuntimeNotInstalled("nginx".into()))?;
        let prefix = self.root.join("runtime/generated/nginx");
        fs::create_dir_all(prefix.join("logs"))?;
        let configuration = prefix.join("nginx.conf");
        atomic_write(&configuration, self.nginx_config()?.as_bytes())?;
        let prefix_arg = format!("{}/", prefix.display());
        let base_args: Vec<OsString> = vec![
            "-p".into(),
            prefix_arg.into(),
            "-c".into(),
            configuration.as_os_str().into(),
        ];
        let result = Command::new(&installation.executable)
            .args(&base_args)
            .arg("-t")
            .output()?;
        if !result.status.success() {
            return Err(OrchestError::Config(format!(
                "nginx configuration failed validation: {}",
                String::from_utf8_lossy(&result.stderr).trim()
            )));
        }
        let mut started_php: Vec<String> = Vec::new();
        for version in self.nginx_php_versions()? {
            if self.php_web_status(&version)? != ServiceStatus::Running {
                if let Err(error) = self.start_php_web(&version) {
                    for started in started_php {
                        let _ = self.stop_php_web(&started);
                    }
                    return Err(error);
                }
                started_php.push(version);
            }
        }
        let mut args = base_args;
        args.extend(["-g".into(), "daemon off; master_process off;".into()]);
        let result = self.start_managed_service(
            "nginx",
            "default",
            &installation,
            &[self.config()?.ports.nginx_http],
            &args,
            &prefix,
        );
        if result.is_err() {
            for started in started_php {
                let _ = self.stop_php_web(&started);
            }
        }
        result
    }
    pub fn stop_nginx(&self) -> Result<ServiceStatus> {
        let status = self.stop_managed_service("nginx", "default")?;
        for instance in self.supervisor().instances("php-web")? {
            if instance.status == ServiceStatus::Running {
                self.stop_managed_service("php-web", &instance.instance_id)?;
            }
        }
        Ok(status)
    }
    pub fn service_status(&self, name: &str) -> Result<ServiceStatus> {
        if let Some(version) = name.strip_prefix("php@") {
            return self.php_web_status(version);
        }
        match name {
            "mailpit" => self.mailpit_status(),
            "meilisearch" => self.meilisearch_status(),
            "nginx" => self.nginx_status(),
            _ => Err(OrchestError::InvalidInput(format!(
                "unknown service: {name}"
            ))),
        }
    }
    pub fn start_service(&self, name: &str) -> Result<ProcessState> {
        if let Some(version) = name.strip_prefix("php@") {
            return self.start_php_web(version);
        }
        match name {
            "mailpit" => self.start_mailpit(),
            "meilisearch" => self.start_meilisearch(),
            "nginx" => self.start_nginx(),
            _ => Err(OrchestError::InvalidInput(format!(
                "unknown service: {name}"
            ))),
        }
    }
    pub fn stop_service(&self, name: &str) -> Result<ServiceStatus> {
        if let Some(version) = name.strip_prefix("php@") {
            return self.stop_php_web(version);
        }
        match name {
            "mailpit" => self.stop_mailpit(),
            "meilisearch" => self.stop_meilisearch(),
            "nginx" => self.stop_nginx(),
            _ => Err(OrchestError::InvalidInput(format!(
                "unknown service: {name}"
            ))),
        }
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
                for port in ports {
                    let duplicate = configured
                        .get(&port.port)
                        .is_some_and(|owners| owners.len() > 1);
                    checks.push(Check {
                        name: format!("port {} ({})", port.port, port.configured_for),
                        level: if duplicate
                            || !matches!(port.state, PortState::Available | PortState::Managed)
                        {
                            "warn"
                        } else {
                            "ok"
                        },
                        detail: if duplicate {
                            format!(
                                "configured for multiple services: {}",
                                configured[&port.port].join(", ")
                            )
                        } else {
                            match port.state {
                                PortState::Managed => format!(
                                    "occupied by managed {}",
                                    port.owner.as_deref().unwrap_or("service")
                                ),
                                PortState::Reserved => format!(
                                    "reserved by {}, but not listening",
                                    port.owner.as_deref().unwrap_or("service")
                                ),
                                PortState::StaleClaim => format!(
                                    "stale claim by {}",
                                    port.owner.as_deref().unwrap_or("service")
                                ),
                                PortState::Unavailable => {
                                    "unavailable (external process or permission denied)".into()
                                }
                                PortState::Available => "available".into(),
                            }
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
        let db = self.connect()?;
        let configured = configured_ports(&ports);
        let mut result: Vec<_> = configured
            .iter()
            .map(|(name, port)| self.port_info(&db, *port, name))
            .collect::<Result<_>>()?;
        let known: std::collections::BTreeSet<_> =
            configured.iter().map(|(_, port)| *port).collect();
        let mut statement = db.prepare("SELECT port FROM port_claims ORDER BY port")?;
        for claimed in statement.query_map([], |row| row.get::<_, u16>(0))? {
            let port = claimed?;
            if !known.contains(&port) {
                result.push(self.port_info(&db, port, "unassigned")?);
            }
        }
        Ok(result)
    }
    pub fn check_port(&self, port: u16) -> Result<PortInfo> {
        if port == 0 {
            return Err(OrchestError::InvalidInput("port must be 1..65535".into()));
        }
        let configured_for = configured_ports(&self.config()?.ports)
            .into_iter()
            .find(|(_, configured_port)| *configured_port == port)
            .map(|(name, _)| name)
            .unwrap_or("unassigned");
        self.port_info(&self.connect()?, port, configured_for)
    }
    fn port_info(&self, db: &Connection, port: u16, configured_for: &str) -> Result<PortInfo> {
        let claim: Option<(String, String)> = db
            .query_row(
                "SELECT service_id,instance_id FROM port_claims WHERE port=?1",
                [port],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let available = port_available(port);
        let (owner, state) = if let Some((service, instance)) = claim {
            let status = self.supervisor().status_instance(&service, &instance)?;
            let state = match status {
                ServiceStatus::Running if available => PortState::Reserved,
                ServiceStatus::Running => PortState::Managed,
                _ => PortState::StaleClaim,
            };
            (Some(format!("{service}/{instance}")), state)
        } else if available {
            (None, PortState::Available)
        } else {
            (None, PortState::Unavailable)
        };
        Ok(PortInfo {
            port,
            configured_for: configured_for.into(),
            available,
            owner,
            state,
        })
    }
}

fn nginx_path(path: &Path) -> Result<String> {
    let path = path.to_string_lossy();
    if path.chars().any(|character| {
        character == '"'
            || character == '$'
            || character.is_control()
            || (cfg!(unix) && character == '\\')
    }) {
        return Err(OrchestError::InvalidInput(format!(
            "project path cannot be represented in nginx configuration: {path}"
        )));
    }
    Ok(path.replace('\\', "/"))
}

fn php_web_instance_id(version: &str) -> String {
    let mut id = String::from("v");
    for byte in version.bytes() {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

fn configured_ports(ports: &Ports) -> Vec<(&'static str, u16)> {
    vec![
        ("nginx_http", ports.nginx_http),
        ("nginx_https", ports.nginx_https),
        ("mysql", ports.mysql),
        ("mariadb", ports.mariadb),
        ("mongodb", ports.mongodb),
        ("redis", ports.redis),
        ("mailpit_http", ports.mailpit_http),
        ("mailpit_smtp", ports.mailpit_smtp),
        ("meilisearch_http", ports.meilisearch_http),
    ]
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
    fn nginx_config_serves_registered_static_projects_only() {
        let root = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), &root.path().join("missing")).unwrap();
        let project = app.add_project_default("site").unwrap();
        let config = app.nginx_config().unwrap();
        assert!(config.contains("server_name site.test;"));
        assert!(config.contains(&format!("root \"{}\";", project.path.display())));
        assert!(config
            .contains("location ~* \\.(?:phtml|phar|php[0-9]?|inc)(?:$|[./]) { return 404; }"));
        assert!(config.contains("listen 127.0.0.1:80 default_server"));
    }
    #[test]
    fn nginx_uses_distinct_fastcgi_backends_for_project_php_versions() {
        let root = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), &root.path().join("missing")).unwrap();
        for version in ["8.4.15", "8.5.10"] {
            let binary = root.path().join("bin/php").join(version).join("bin/php");
            fs::create_dir_all(binary.parent().unwrap()).unwrap();
            fs::write(&binary, b"fixture").unwrap();
            app.register_installation("php", version, "linux-x86_64", "fixture".into(), "bin/php")
                .unwrap();
        }
        app.add_project_default("first").unwrap();
        app.add_project_default("second").unwrap();
        app.set_project_php("first", "8.4.15").unwrap();
        app.set_project_php("second", "8.5.10").unwrap();
        let first_port = app.php_web_port("8.4.15").unwrap();
        let second_port = app.php_web_port("8.5.10").unwrap();
        assert_ne!(first_port, second_port);
        let generated = app.nginx_config().unwrap();
        assert!(generated.contains(&format!("fastcgi_pass 127.0.0.1:{first_port};")));
        assert!(generated.contains(&format!("fastcgi_pass 127.0.0.1:{second_port};")));
        assert!(
            generated.contains("fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;")
        );
        assert!(generated.contains("try_files $uri =404;"));
        assert_eq!(app.php_web_port("8.4.15").unwrap(), first_port);
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
        assert!(app.catalog().get("meilisearch").is_ok());
        assert!(app.catalog().get("nginx").is_ok());
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
        assert_eq!(config.ports.meilisearch_http, 7700);
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
    #[cfg(unix)]
    #[test]
    fn port_claims_track_instances_and_recover_after_exit() {
        let root = tempfile::tempdir().unwrap();
        let app = Orchest::init(root.path().to_path_buf(), &root.path().join("missing")).unwrap();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        app.config_set("ports.redis", &port.to_string()).unwrap();
        let installation = Installation {
            package: "worker".into(),
            version: "1".into(),
            platform: "linux-x86_64".into(),
            installed_at: Utc::now(),
            source_url: "fixture".into(),
            executable: PathBuf::from("/bin/sleep"),
        };
        let first = app
            .start_managed_service(
                "worker",
                "one",
                &installation,
                &[port],
                &["30".into()],
                root.path(),
            )
            .unwrap();
        let info = app.check_port(port).unwrap();
        assert_eq!(info.owner.as_deref(), Some("worker/one"));
        assert_eq!(info.state, PortState::Reserved);
        let other_port = if port == u16::MAX { port - 1 } else { port + 1 };
        app.config_set("ports.redis", &other_port.to_string())
            .unwrap();
        assert!(app
            .ports()
            .unwrap()
            .iter()
            .any(|item| item.port == port && item.owner.as_deref() == Some("worker/one")));
        assert!(app
            .start_managed_service(
                "worker",
                "two",
                &installation,
                &[port],
                &["30".into()],
                root.path()
            )
            .is_err());
        app.supervisor().stop_instance("worker", "one").unwrap();
        assert_eq!(app.check_port(port).unwrap().state, PortState::StaleClaim);
        let second = app
            .start_managed_service(
                "worker",
                "two",
                &installation,
                &[port],
                &["30".into()],
                root.path(),
            )
            .unwrap();
        assert_ne!(first.pid, second.pid);
        assert_eq!(
            app.check_port(port).unwrap().owner.as_deref(),
            Some("worker/two")
        );
        app.stop_managed_service("worker", "two").unwrap();
        assert_eq!(app.check_port(port).unwrap().state, PortState::Available);
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
