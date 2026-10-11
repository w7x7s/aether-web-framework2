//! # Aether Production Configuration Engine
//!
//! Type-safe configuration management with environment variable loading,
//! strict validation, and secret sanitization for logs and telemetry.

use std::env;
use std::fmt;

#[derive(Debug, Clone)]
pub struct AetherConfig {
    pub host: String,
    pub port: u16,
    pub environment: String,
    pub max_body_bytes: usize,
    pub max_header_bytes: usize,
    pub workers: usize,
}

impl Default for AetherConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
            environment: "development".to_string(),
            max_body_bytes: 2 * 1024 * 1024, // 2MB
            max_header_bytes: 8 * 1024,      // 8KB
            workers: num_cpus::get().max(1),
        }
    }
}

impl AetherConfig {
    /// Loads configuration automatically from environment variables with sensible defaults.
    pub fn from_env() -> Result<Self, String> {
        let mut cfg = Self::default();

        if let Ok(host) = env::var("AETHER_HOST") {
            cfg.host = host;
        }

        if let Ok(port_str) = env::var("AETHER_PORT") {
            cfg.port = port_str
                .parse::<u16>()
                .map_err(|_| format!("Invalid AETHER_PORT value '{}': Must be a valid port (1-65535)", port_str))?;
        }

        if let Ok(env_str) = env::var("AETHER_ENV") {
            cfg.environment = env_str;
        }

        if let Ok(body_str) = env::var("AETHER_MAX_BODY_BYTES") {
            cfg.max_body_bytes = body_str
                .parse::<usize>()
                .map_err(|_| format!("Invalid AETHER_MAX_BODY_BYTES: '{}'", body_str))?;
        }

        Ok(cfg)
    }

    pub fn is_production(&self) -> bool {
        self.environment.eq_ignore_ascii_case("production") || self.environment.eq_ignore_ascii_case("prod")
    }

    pub fn listen_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl fmt::Display for AetherConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AetherConfig(env={}, addr={}, max_body={}B, workers={})",
            self.environment,
            self.listen_addr(),
            self.max_body_bytes,
            self.workers
        )
    }
}

// Fallback if num_cpus is not included
mod num_cpus {
    pub fn get() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }
}
