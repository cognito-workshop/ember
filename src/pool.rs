use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tokio::net::TcpStream;

/// Statistics about the connection pool.
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub total_idle: usize,
    pub targets: usize,
    pub hits: u64,
    pub misses: u64,
}

/// A thread-safe connection pool for upstream TCP connections.
///
/// Connections are keyed by "host:port" strings. When a connection is returned
/// to the pool, it is marked idle. When retrieved, it is health-checked with
/// a 1-byte peek to ensure the remote end is still connected.
pub struct ConnectionPool {
    pools: DashMap<String, Vec<IdleConnection>>,
    max_per_target: usize,
    max_total: usize,
    hits: AtomicU64,
    misses: AtomicU64,
}

struct IdleConnection {
    stream: TcpStream,
    created: std::time::Instant,
}

impl ConnectionPool {
    pub fn new(max_per_target: usize, max_total: usize) -> Arc<Self> {
        Arc::new(Self {
            pools: DashMap::new(),
            max_per_target,
            max_total,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        })
    }

    /// Get an idle connection for the given target ("host:port").
    /// Returns `None` if no healthy idle connection is available.
    pub async fn get(&self, target: &str) -> Option<TcpStream> {
        let mut entry = self.pools.get_mut(target)?;

        while let Some(idle) = entry.pop() {
            if idle.created.elapsed().as_secs() > 120 {
                continue;
            }

            if is_alive(&idle.stream).await {
                self.hits.fetch_add(1, Ordering::Relaxed);
                tracing::trace!("pool hit for {}", target);
                return Some(idle.stream);
            }
        }

        None
    }

    /// Return a connection to the pool for later reuse.
    /// If the pool is full or the target is at its per-target limit, the stream
    /// is dropped (closed).
    pub async fn put(&self, target: &str, stream: TcpStream) {
        if !is_alive(&stream).await {
            return;
        }

        let total: usize = self.pools.iter().map(|e| e.value().len()).sum();
        if total >= self.max_total {
            return;
        }

        let mut entry = self.pools.entry(target.to_string()).or_default();
        if entry.len() < self.max_per_target {
            entry.push(IdleConnection {
                stream,
                created: std::time::Instant::now(),
            });
        }
    }

    /// Remove all pooled connections for a given target (e.g. on error).
    pub fn remove(&self, target: &str) {
        self.pools.remove(target);
    }

    /// Return pool statistics.
    pub fn stats(&self) -> PoolStats {
        let total_idle: usize = self.pools.iter().map(|e| e.value().len()).sum();
        PoolStats {
            total_idle,
            targets: self.pools.len(),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    pub fn miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }
}

/// Check whether a TCP connection is still alive by attempting a non-blocking
/// 1-byte read. Returns `false` if the read fails or returns 0 (EOF).
async fn is_alive(stream: &TcpStream) -> bool {
    let mut buf = [0u8; 1];
    match stream.peek(&mut buf).await {
        Ok(0) => false,
        Ok(_) => true,
        Err(_) => false,
    }
}
