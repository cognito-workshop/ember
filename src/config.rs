use clap::Parser;
use serde::Deserialize;
use std::path::Path;

use crate::error::WispError;
use crate::wisp::buffer::BufferConfig as RuntimeBufferConfig;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub tls: TlsConfig,
    pub buffer: BufferConfig,
    pub extensions: ExtensionsConfig,
    pub logging: LoggingConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PluginsConfig {
    #[serde(default)]
    pub rate_limiter: Option<RateLimiterConfig>,
    #[serde(default = "default_true")]
    pub logger: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RateLimiterConfig {
    #[serde(default = "default_rl_max")]
    pub max_connections_per_ip: u32,
    #[serde(default = "default_rl_window")]
    pub window_secs: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cert_path: String,
    #[serde(default)]
    pub key_path: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BufferConfig {
    #[serde(default = "default_buffer_initial")]
    pub initial_size: u32,
    #[serde(default = "default_buffer_min")]
    pub min_size: u32,
    #[serde(default = "default_buffer_max")]
    pub max_size: u32,
    #[serde(default = "default_high_watermark")]
    pub high_watermark: f64,
    #[serde(default = "default_low_watermark")]
    pub low_watermark: f64,
    #[serde(default = "default_tcp_read_size")]
    pub tcp_read_size: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExtensionsConfig {
    #[serde(default = "default_true")]
    pub udp: bool,
    #[serde(default = "default_motd")]
    pub motd: String,
    #[serde(default)]
    pub stream_open_confirmation: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

#[derive(clap::Parser, Debug)]
#[command(name = "ember", about = "Ember Wisp server")]
pub struct Cli {
    /// Path to TOML config file
    #[arg(short, long)]
    pub config: Option<String>,

    /// Listen address
    #[arg(short, long)]
    pub host: Option<String>,

    /// Listen port
    #[arg(short, long)]
    pub port: Option<u16>,

    /// Enable TLS
    #[arg(long)]
    pub tls: bool,

    /// TLS certificate file path
    #[arg(long)]
    pub cert: Option<String>,

    /// TLS private key file path
    #[arg(long)]
    pub key: Option<String>,

    /// Enable debug logging
    #[arg(short, long)]
    pub verbose: bool,

    /// Use thread-per-core runtime (Linux only, requires SO_REUSEPORT)
    #[arg(long)]
    pub thread_per_core: bool,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    443
}
fn default_max_connections() -> u32 {
    10000
}
fn default_buffer_initial() -> u32 {
    128
}
fn default_buffer_min() -> u32 {
    32
}
fn default_buffer_max() -> u32 {
    1024
}
fn default_high_watermark() -> f64 {
    0.8
}
fn default_low_watermark() -> f64 {
    0.2
}
fn default_tcp_read_size() -> usize {
    131072 // 128KB — matches epoxy-server
}
fn default_rl_max() -> u32 {
    100
}
fn default_rl_window() -> u64 {
    60
}
fn default_true() -> bool {
    true
}
fn default_motd() -> String {
    "Welcome to Ember".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            tls: TlsConfig::default(),
            buffer: BufferConfig::default(),
            extensions: ExtensionsConfig::default(),
            logging: LoggingConfig::default(),
            plugins: PluginsConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            max_connections: default_max_connections(),
        }
    }
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_path: String::new(),
            key_path: String::new(),
        }
    }
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            initial_size: default_buffer_initial(),
            min_size: default_buffer_min(),
            max_size: default_buffer_max(),
            high_watermark: default_high_watermark(),
            low_watermark: default_low_watermark(),
            tcp_read_size: default_tcp_read_size(),
        }
    }
}

impl Default for ExtensionsConfig {
    fn default() -> Self {
        Self {
            udp: true,
            motd: default_motd(),
            stream_open_confirmation: false,
        }
    }
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            rate_limiter: None,
            logger: true,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

impl From<BufferConfig> for RuntimeBufferConfig {
    fn from(cfg: BufferConfig) -> Self {
        Self {
            initial_size: cfg.initial_size,
            min_size: cfg.min_size,
            max_size: cfg.max_size,
            high_watermark: cfg.high_watermark,
            low_watermark: cfg.low_watermark,
        }
    }
}

impl Config {
    pub fn load(cli: &Cli) -> Result<Self, WispError> {
        let mut config = Config::default();

        if let Some(ref path) = cli.config {
            let contents = std::fs::read_to_string(Path::new(path)).map_err(|e| {
                WispError::Config(format!("failed to read config file '{}': {}", path, e))
            })?;
            config = toml::from_str(&contents).map_err(|e| {
                WispError::Config(format!("failed to parse config file '{}': {}", path, e))
            })?;
        }

        if let Some(ref host) = cli.host {
            config.server.host = host.clone();
        }
        if let Some(port) = cli.port {
            config.server.port = port;
        }
        if cli.tls {
            config.tls.enabled = true;
        }
        if let Some(ref cert) = cli.cert {
            config.tls.cert_path = cert.clone();
        }
        if let Some(ref key) = cli.key {
            config.tls.key_path = key.clone();
        }
        if cli.verbose {
            config.logging.level = "debug".to_string();
        }

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), WispError> {
        if self.tls.enabled {
            if self.tls.cert_path.is_empty() {
                return Err(WispError::Config(
                    "TLS enabled but no certificate path provided".to_string(),
                ));
            }
            if self.tls.key_path.is_empty() {
                return Err(WispError::Config(
                    "TLS enabled but no key path provided".to_string(),
                ));
            }
            if !Path::new(&self.tls.cert_path).exists() {
                return Err(WispError::Config(format!(
                    "TLS certificate file not found: {}",
                    self.tls.cert_path
                )));
            }
            if !Path::new(&self.tls.key_path).exists() {
                return Err(WispError::Config(format!(
                    "TLS key file not found: {}",
                    self.tls.key_path
                )));
            }
        }

        if self.buffer.min_size > self.buffer.initial_size {
            return Err(WispError::Config(
                "buffer min_size cannot be greater than initial_size".to_string(),
            ));
        }
        if self.buffer.initial_size > self.buffer.max_size {
            return Err(WispError::Config(
                "buffer initial_size cannot be greater than max_size".to_string(),
            ));
        }
        if self.buffer.high_watermark <= self.buffer.low_watermark {
            return Err(WispError::Config(
                "buffer high_watermark must be greater than low_watermark".to_string(),
            ));
        }

        Ok(())
    }
}
