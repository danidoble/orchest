use crate::{OrchestError, Result};
use orchest_platform::atomic_write;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use std::{fs, path::Path, time::Duration};
use time::OffsetDateTime;

fn certificate_error(error: impl std::fmt::Display) -> OrchestError {
    OrchestError::Config(format!("certificate generation failed: {error}"))
}

fn ca_params() -> Result<CertificateParams> {
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(certificate_error)?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, "Orchest Local CA");
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(3650);
    Ok(params)
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn ensure_site_certificate(root: &Path, domain: &str) -> Result<bool> {
    if domain.is_empty()
        || !domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err(OrchestError::Config("invalid certificate domain".into()));
    }
    let ca_dir = root.join("certificates/ca");
    let ca_cert = ca_dir.join("cert.pem");
    let ca_key = ca_dir.join("key.pem");
    fs::create_dir_all(&ca_dir)?;
    let ca_key_exists = ca_key.is_file();
    let key = if ca_key_exists {
        KeyPair::from_pem(&fs::read_to_string(&ca_key)?).map_err(certificate_error)?
    } else {
        let key = KeyPair::generate().map_err(certificate_error)?;
        private_write(&ca_key, key.serialize_pem().as_bytes())?;
        key
    };
    let params = ca_params()?;
    if !ca_cert.is_file() || !ca_key_exists {
        let certificate = params.self_signed(&key).map_err(certificate_error)?;
        atomic_write(&ca_cert, certificate.pem().as_bytes())?;
    }
    let issuer = Issuer::new(params, key);
    let site_dir = root.join("certificates/sites").join(domain);
    fs::create_dir_all(&site_dir)?;
    let cert = site_dir.join("cert.pem");
    let key = site_dir.join("key.pem");
    let fresh = cert.is_file()
        && key.is_file()
        && fs::metadata(&cert)?
            .modified()?
            .elapsed()
            .unwrap_or(Duration::MAX)
            < Duration::from_secs(60 * 24 * 60 * 60);
    if !fresh {
        let mut params =
            CertificateParams::new(vec![domain.to_owned()]).map_err(certificate_error)?;
        params.distinguished_name.push(DnType::CommonName, domain);
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth);
        params.not_before = OffsetDateTime::now_utc() - time::Duration::days(1);
        params.not_after = OffsetDateTime::now_utc() + time::Duration::days(90);
        let site_key = KeyPair::generate().map_err(certificate_error)?;
        let site_cert = params
            .signed_by(&site_key, &issuer)
            .map_err(certificate_error)?;
        private_write(&key, site_key.serialize_pem().as_bytes())?;
        atomic_write(&cert, site_cert.pem().as_bytes())?;
    }
    Ok(!fresh || !ca_key_exists)
}
