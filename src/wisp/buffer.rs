#[derive(Clone, Debug)]
pub struct BufferConfig {
    pub initial_size: u32,
    pub min_size: u32,
    pub max_size: u32,
    pub high_watermark: f64,
    pub low_watermark: f64,
    pub max_buffer_bytes: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            initial_size: 128,
            min_size: 32,
            max_size: 1024,
            high_watermark: 0.8,
            low_watermark: 0.2,
            max_buffer_bytes: 10 * 1024 * 1024,
        }
    }
}

pub struct AdaptiveBuffer {
    capacity: u32,
    queued: u32,
    config: BufferConfig,
}

impl AdaptiveBuffer {
    #[inline]
    pub fn new(config: BufferConfig) -> Self {
        let initial = config.initial_size;
        Self {
            capacity: initial,
            queued: 0,
            config,
        }
    }

    #[inline]
    pub fn can_accept(&self) -> bool {
        self.queued < self.capacity
    }

    #[inline]
    pub fn on_send(&mut self) {
        self.queued = self.queued.saturating_add(1);
    }

    #[inline]
    pub fn on_drain(&mut self) {
        self.queued = self.queued.saturating_sub(1);
        self.adapt_down();
    }

    #[inline]
    pub fn adapt(&mut self) {
        // Estimate bytes: capacity * avg message size ~1KB
        let estimated_bytes = self.capacity as usize * 1024;
        if estimated_bytes >= self.config.max_buffer_bytes {
            return;
        }
        let usage = self.queued as f64 / self.capacity as f64;
        if usage > self.config.high_watermark {
            self.capacity = (self.capacity + 64).min(self.config.max_size);
        }
    }

    #[inline]
    fn adapt_down(&mut self) {
        let usage = self.queued as f64 / self.capacity as f64;
        if usage < self.config.low_watermark {
            let new_capacity = (self.capacity.saturating_sub(32)).max(self.config.min_size);
            if self.queued < new_capacity {
                self.capacity = new_capacity;
            }
        }
    }

    #[inline]
    pub fn remaining(&self) -> u32 {
        self.capacity.saturating_sub(self.queued)
    }

    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    #[inline]
    pub fn reset(&mut self) {
        self.queued = 0;
        self.capacity = self.config.initial_size;
    }
}
