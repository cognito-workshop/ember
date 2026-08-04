use std::fs::File;
use std::io::BufReader;
use rustls::ServerConfig;
use rustls_pemfile::{certs, private_key};

use crate::error::WispError;

pub fn load_tls_config(cert_path: &str, key_path: &str) -> Result<ServerConfig, WispError> {
    let cert_file = File::open(cert_path).map_err(|e| {
        WispError::Config(format!("failed to open cert file '{}': {}", cert_path, e))
    })?;
    let key_file = File::open(key_path).map_err(|e| {
        WispError::Config(format!("failed to open key file '{}': {}", key_path, e))
    })?;

    let mut cert_reader = BufReader::new(cert_file);
    let mut key_reader = BufReader::new(key_file);

    let certs = certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            WispError::Config(format!("failed to parse certificates: {}", e))
        })?;

    let key = private_key(&mut key_reader)
        .map_err(|e| {
            WispError::Config(format!("failed to parse private key: {}", e))
        })?
        .ok_or_else(|| {
            WispError::Config("no private key found in key file".to_string())
        })?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| {
            WispError::Config(format!("failed to build TLS config: {}", e))
        })?;

    Ok(config)
}
