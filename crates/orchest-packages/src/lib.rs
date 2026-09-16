//! Manifest-based package catalog and safe, staged installation.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub package: Package,
    pub versions: Vec<Version>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
    #[serde(flatten)]
    pub platforms: BTreeMap<String, Artifact>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub archive: String,
    pub executable: String,
    #[serde(default)]
    pub strip_components: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("package {0} was not found in the catalog")]
    PackageNotFound(String),
    #[error("version {0} was not found")]
    VersionNotFound(String),
    #[error("no artifact for {0}")]
    PlatformUnavailable(String),
    #[error("invalid manifest: {0}")]
    Manifest(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("unsafe or invalid archive: {0}")]
    Archive(String),
    #[error("expected executable is absent: {0}")]
    MissingExecutable(PathBuf),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub struct Catalog {
    manifests: BTreeMap<String, Manifest>,
}
impl Catalog {
    pub fn load(dir: &Path) -> Result<Self, PackageError> {
        let mut manifests = BTreeMap::new();
        if !dir.exists() {
            return Ok(Self { manifests });
        }
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let manifest: Manifest = toml::from_str(&fs::read_to_string(&path)?)
                .map_err(|e| PackageError::Manifest(format!("{}: {e}", path.display())))?;
            validate_manifest(&manifest)?;
            if manifests
                .insert(manifest.package.id.clone(), manifest)
                .is_some()
            {
                return Err(PackageError::Manifest("duplicate package id".into()));
            }
        }
        Ok(Self { manifests })
    }
    pub fn list(&self) -> Vec<&Manifest> {
        self.manifests.values().collect()
    }
    pub fn get(&self, id: &str) -> Result<&Manifest, PackageError> {
        self.manifests
            .get(id)
            .ok_or_else(|| PackageError::PackageNotFound(id.into()))
    }
    pub fn artifact(
        &self,
        id: &str,
        version: &str,
        target: &str,
    ) -> Result<&Artifact, PackageError> {
        let release = self
            .get(id)?
            .versions
            .iter()
            .find(|v| v.version == version)
            .ok_or_else(|| PackageError::VersionNotFound(version.into()))?;
        release
            .platforms
            .get(target)
            .ok_or_else(|| PackageError::PlatformUnavailable(target.into()))
    }
}

fn safe_relative(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
        && !path.as_os_str().is_empty()
}
fn validate_manifest(m: &Manifest) -> Result<(), PackageError> {
    let mut seen = std::collections::BTreeSet::new();
    let valid_id = !m.package.id.is_empty()
        && m.package
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'-' || b.is_ascii_digit());
    if !valid_id || m.versions.is_empty() {
        return Err(PackageError::Manifest(
            "invalid package id or empty versions".into(),
        ));
    }
    for version in &m.versions {
        if !seen.insert(&version.version) {
            return Err(PackageError::Manifest(format!(
                "duplicate version: {}",
                version.version
            )));
        }
        if version.version.is_empty()
            || !version
                .version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            return Err(PackageError::Manifest("invalid version".into()));
        }
        for (target, artifact) in &version.platforms {
            if !["windows-x86_64", "linux-x86_64"].contains(&target.as_str()) {
                return Err(PackageError::Manifest(format!("unknown target: {target}")));
            }
            if !artifact.url.starts_with("https://")
                || !safe_relative(Path::new(&artifact.executable))
                || !["zip", "tar.gz", "binary"].contains(&artifact.archive.as_str())
                || (artifact.archive == "binary" && artifact.strip_components != 0)
            {
                return Err(PackageError::Manifest(format!(
                    "invalid artifact for {}",
                    version.version
                )));
            }
        }
    }
    Ok(())
}

/// Installs an artifact into a staging directory and publishes it by rename.
/// State registration is deliberately left to the caller after this returns.
pub fn install(artifact: &Artifact, destination: &Path, cache: &Path) -> Result<(), PackageError> {
    if destination.exists() {
        return Ok(());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| PackageError::Manifest("destination has no parent".into()))?;
    fs::create_dir_all(parent)?;
    fs::create_dir_all(cache)?;
    let download = tempfile::NamedTempFile::new_in(cache)?;
    let mut response = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| PackageError::Download(e.to_string()))?
        .get(&artifact.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| PackageError::Download(e.to_string()))?;
    if response.url().scheme() != "https" {
        return Err(PackageError::Download(
            "redirected to a non-HTTPS URL".into(),
        ));
    }
    let mut output = File::create(download.path())?;
    io::copy(&mut response, &mut output)?;
    output.sync_all()?;
    install_archive(artifact, download.path(), destination)
}

/// Validates and publishes a previously downloaded archive. Useful for offline fixtures.
pub fn install_archive(
    artifact: &Artifact,
    archive: &Path,
    destination: &Path,
) -> Result<(), PackageError> {
    if destination.exists() {
        return Ok(());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| PackageError::Manifest("destination has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    match artifact.archive.as_str() {
        "zip" => extract_zip(archive, staging.path(), artifact.strip_components)?,
        "tar.gz" => extract_tar_gz(archive, staging.path(), artifact.strip_components)?,
        "binary" => {
            let mut source = File::open(archive)?;
            let mut magic = [0u8; 4];
            source
                .read_exact(&mut magic)
                .map_err(|_| PackageError::Archive("binary is too short".into()))?;
            let expected = if artifact.executable.ends_with(".exe") {
                b"MZ".as_slice()
            } else {
                b"\x7fELF".as_slice()
            };
            if !magic.starts_with(expected) {
                return Err(PackageError::Archive("unexpected executable format".into()));
            }
            let output = staging.path().join(&artifact.executable);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(archive, output)?;
        }
        other => {
            return Err(PackageError::Archive(format!(
                "unsupported archive: {other}"
            )))
        }
    }
    let executable = staging.path().join(&artifact.executable);
    if !executable.is_file() {
        return Err(PackageError::MissingExecutable(executable));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&executable)?.permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        fs::set_permissions(&executable, permissions)?;
    }
    if destination.exists() {
        return Ok(());
    }
    fs::rename(staging.path(), destination)?;
    Ok(())
}

fn stripped(path: &Path, count: usize) -> Result<Option<PathBuf>, PackageError> {
    if !safe_relative(path) {
        return Err(PackageError::Archive(format!(
            "unsafe path: {}",
            path.display()
        )));
    }
    let result: PathBuf = path.components().skip(count).collect();
    if result.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

pub fn extract_zip(
    archive: &Path,
    destination: &Path,
    strip_components: usize,
) -> Result<(), PackageError> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)
        .map_err(|e| PackageError::Archive(e.to_string()))?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| PackageError::Archive(e.to_string()))?;
        let path = Path::new(entry.name());
        let Some(relative) = stripped(path, strip_components)? else {
            continue;
        };
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(PackageError::Archive("symbolic link".into()));
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(output)?;
        io::copy(&mut entry, &mut file)?;
        file.flush()?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}

pub fn extract_tar_gz(
    archive: &Path,
    destination: &Path,
    strip_components: usize,
) -> Result<(), PackageError> {
    let gzip = flate2::read::GzDecoder::new(File::open(archive)?);
    let mut tar = tar::Archive::new(gzip);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Some(relative) = stripped(&path, strip_components)? else {
            continue;
        };
        let output = destination.join(relative);
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(output)?;
        } else if kind.is_file() {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(output)?;
            io::copy(&mut entry, &mut file)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(entry.header().mode()? & 0o777))?;
            }
        } else {
            return Err(PackageError::Archive(
                "links and special entries are not supported".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal() {
        assert!(stripped(Path::new("../outside"), 0).is_err());
        assert!(stripped(Path::new("/absolute"), 0).is_err());
    }
    #[test]
    fn checks_manifest() {
        let manifest: Manifest = toml::from_str("[package]\nid='php'\nname='PHP'\ntype='runtime'\n[[versions]]\nversion='8.4.15'\n[versions.linux-x86_64]\nurl='https://example.test/php.tar.gz'\narchive='tar.gz'\nexecutable='bin/php'").unwrap();
        validate_manifest(&manifest).unwrap();
    }
    #[test]
    fn installs_raw_binary_and_rejects_wrong_format() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download");
        let artifact = Artifact {
            url: "https://example.test/meilisearch".into(),
            archive: "binary".into(),
            executable: "meilisearch".into(),
            strip_components: 0,
        };
        fs::write(&source, b"not a binary").unwrap();
        assert!(install_archive(&artifact, &source, &root.path().join("installed")).is_err());
        assert!(!root.path().join("installed").exists());
        fs::write(&source, b"\x7fELFfixture").unwrap();
        install_archive(&artifact, &source, &root.path().join("installed")).unwrap();
        assert_eq!(
            fs::read(root.path().join("installed/meilisearch")).unwrap(),
            b"\x7fELFfixture"
        );
    }
    #[test]
    fn extracts_zip_fixture() {
        use zip::write::SimpleFileOptions;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file("php/bin/php", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"fixture").unwrap();
        writer.finish().unwrap();
        let output = dir.path().join("output");
        extract_zip(&path, &output, 1).unwrap();
        assert_eq!(fs::read(output.join("bin/php")).unwrap(), b"fixture");
    }
    #[test]
    fn extracts_tar_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.tar.gz");
        let gzip = flate2::write::GzEncoder::new(
            File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(gzip);
        let bytes = b"fixture";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "php/bin/php", &bytes[..])
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        let output = dir.path().join("output");
        extract_tar_gz(&path, &output, 1).unwrap();
        assert_eq!(fs::read(output.join("bin/php")).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(output.join("bin/php"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
    }
    #[test]
    fn archive_install_is_atomic_and_idempotent() {
        use zip::write::SimpleFileOptions;
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("php.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        writer
            .start_file("php", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"php binary").unwrap();
        writer.finish().unwrap();
        let destination = dir.path().join("bin/php/8.4.15");
        let artifact = Artifact {
            url: "https://example.test/php.zip".into(),
            archive: "zip".into(),
            executable: "php".into(),
            strip_components: 0,
        };
        install_archive(&artifact, &archive, &destination).unwrap();
        install_archive(&artifact, &archive, &destination).unwrap();
        assert_eq!(fs::read(destination.join("php")).unwrap(), b"php binary");
    }
    #[test]
    fn missing_binary_does_not_publish() {
        use zip::write::SimpleFileOptions;
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("php.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        writer
            .start_file("readme", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"readme").unwrap();
        writer.finish().unwrap();
        let destination = dir.path().join("bin/php/8.4.15");
        let artifact = Artifact {
            url: "https://example.test/php.zip".into(),
            archive: "zip".into(),
            executable: "php".into(),
            strip_components: 0,
        };
        assert!(matches!(
            install_archive(&artifact, &archive, &destination),
            Err(PackageError::MissingExecutable(_))
        ));
        assert!(!destination.exists());
    }
}
