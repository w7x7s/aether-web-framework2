use std::time::Instant;

/// High-performance logging middleware recording request timing with microsecond precision.
#[derive(Debug, Clone)]
pub struct LoggerMiddleware {
    pub enabled: bool,
    pub show_timing: bool,
}

impl Default for LoggerMiddleware {
    fn default() -> Self {
        Self {
            // Disabled on hot path by default in high-throughput benchmarks, configurable for dev
            enabled: false,
            show_timing: false,
        }
    }
}

impl LoggerMiddleware {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Record request start
    #[inline(always)]
    pub fn start(&self) -> Instant {
        Instant::now()
    }

    /// Log request outcome with elapsed time (non-blocking when disabled)
    #[inline(always)]
    pub fn log(&self, method: &str, path: &str, status: u16, start: Instant) {
        if self.enabled {
            let elapsed = start.elapsed();
            let elapsed_us = elapsed.as_micros();
            println!(
                "⚡ [Aether] {} {} -> {} ({}µs)",
                method, path, status, elapsed_us
            );
        }
    }
}

