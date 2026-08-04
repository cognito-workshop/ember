use serde::Deserialize;
use std::path::Path;

use crate::error::WispError;
use crate::wisp::buffer::BufferConfig as RuntimeBufferConfig;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub tls: TlsConfig,
    pub buffer: BufferConfig,
    pub extensions: ExtensionsConfig,
    pub logging: LoggingConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerConfig,
    #[serde(default)]
    pub pool: PoolConfig,
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
    #[serde(default = "default_metrics_port")]
    pub metrics_port: u16,
}

#[derive(Clone, Debug, Default, Deserialize)]
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
    #[serde(default = "default_max_buffer_bytes")]
    pub max_buffer_bytes: usize,
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
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PoolConfig {
    #[serde(default = "default_pool_max_per_target")]
    pub max_per_target: usize,
    #[serde(default = "default_pool_max_total")]
    pub max_total: usize,
    #[serde(default = "default_pool_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
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

    /// Launch interactive TUI dashboard (requires `tui` feature)
    #[arg(long)]
    pub tui: bool,
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
fn default_metrics_port() -> u16 {
    9090
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
fn default_max_size_mb() -> u64 {
    100
}
fn default_max_buffer_bytes() -> usize {
    10 * 1024 * 1024
}
fn default_pool_max_per_target() -> usize {
    16
}
fn default_pool_max_total() -> usize {
    256
}
fn default_pool_idle_timeout_secs() -> u64 {
    60
}



impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_per_target: default_pool_max_per_target(),
            max_total: default_pool_max_total(),
            idle_timeout_secs: default_pool_idle_timeout_secs(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            max_connections: default_max_connections(),
            metrics_port: default_metrics_port(),
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
            max_buffer_bytes: default_max_buffer_bytes(),
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

#[derive(Clone, Debug, Deserialize)]
pub struct CircuitBreakerConfig {
    #[serde(default = "default_cb_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cb_failure_threshold")]
    pub failure_threshold: u64,
    #[serde(default = "default_cb_recovery_timeout_secs")]
    pub recovery_timeout_secs: u64,
    #[serde(default = "default_cb_half_open_max")]
    pub half_open_max: u64,
}

fn default_cb_enabled() -> bool {
    true
}
fn default_cb_failure_threshold() -> u64 {
    5
}
fn default_cb_recovery_timeout_secs() -> u64 {
    30
}
fn default_cb_half_open_max() -> u64 {
    3
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            enabled: default_cb_enabled(),
            failure_threshold: default_cb_failure_threshold(),
            recovery_timeout_secs: default_cb_recovery_timeout_secs(),
            half_open_max: default_cb_half_open_max(),
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
            file: None,
            max_size_mb: default_max_size_mb(),
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
            max_buffer_bytes: cfg.max_buffer_bytes,
        }
    }
}

impl Config {
    pub fn load_from_path(path: &str) -> Result<Self, WispError> {
        let contents = std::fs::read_to_string(Path::new(path)).map_err(|e| {
            WispError::Config(format!("failed to read config file '{}': {}", path, e))
        })?;
        let config: Config = toml::from_str(&contents).map_err(|e| {
            WispError::Config(format!("failed to parse config file '{}': {}", path, e))
        })?;
        config.validate()?;
        Ok(config)
    }

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
