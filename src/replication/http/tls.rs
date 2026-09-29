use super::protocol::ReplicationHttpError;
use rustls::{ClientConfig, pki_types::CertificateDer};
use rustls_platform_verifier::BuilderVerifierExt;
use std::{fs::File, io::BufReader, path::Path, sync::Arc};

pub(super) fn client_config(
    identity: Option<(&Path, &Path)>,
) -> Result<Arc<ClientConfig>, ReplicationHttpError> {
    let builder = ClientConfig::builder()
        .with_platform_verifier()
        .map_err(|error| ReplicationHttpError::Tls(error.to_string()))?;
    let config = if let Some((certificate_path, key_path)) = identity {
        let certificates = read_certificates(certificate_path)?;
        let private_key = rustls_pemfile::private_key(&mut BufReader::new(
            File::open(key_path).map_err(|error| {
                ReplicationHttpError::InvalidConfig(format!(
                    "cannot read TLS client private key {}: {error}",
                    key_path.display()
                ))
            })?,
        ))
        .map_err(|error| {
            ReplicationHttpError::InvalidConfig(format!(
                "cannot parse TLS client private key: {error}"
            ))
        })?
        .ok_or_else(|| {
            ReplicationHttpError::InvalidConfig(
                "TLS client private key file contains no private key".into(),
            )
        })?;
        builder
            .with_client_auth_cert(certificates, private_key)
            .map_err(|error| ReplicationHttpError::Tls(error.to_string()))?
    } else {
        builder.with_no_client_auth()
    };
    Ok(Arc::new(config))
}

fn read_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, ReplicationHttpError> {
    let file = File::open(path).map_err(|error| {
        ReplicationHttpError::InvalidConfig(format!(
            "cannot read TLS client certificate {}: {error}",
            path.display()
        ))
    })?;
    let certificates = rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            ReplicationHttpError::InvalidConfig(format!(
                "cannot parse TLS client certificate: {error}"
            ))
        })?;
    if certificates.is_empty() {
        return Err(ReplicationHttpError::InvalidConfig(
            "TLS client certificate file contains no certificates".into(),
        ));
    }
    Ok(certificates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::generate_simple_self_signed;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_CERTIFICATE_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn client_identity_files_must_exist_and_match() {
        let directory = std::env::temp_dir().join(format!(
            "txbase-http-client-tls-{}-{}",
            std::process::id(),
            NEXT_CERTIFICATE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let identity = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let certificate_path = directory.join("client.pem");
        let key_path = directory.join("client-key.pem");
        fs::write(&certificate_path, identity.cert.pem()).unwrap();
        fs::write(&key_path, identity.key_pair.serialize_pem()).unwrap();

        assert!(client_config(Some((&certificate_path, &key_path))).is_ok());

        let other = generate_simple_self_signed(vec!["other".into()]).unwrap();
        let mismatched_key_path = directory.join("mismatched-key.pem");
        fs::write(&mismatched_key_path, other.key_pair.serialize_pem()).unwrap();
        assert!(client_config(Some((&certificate_path, &mismatched_key_path))).is_err());
        assert!(client_config(Some((&certificate_path, &directory.join("missing.pem")))).is_err());

        fs::remove_dir_all(directory).unwrap();
    }
}
