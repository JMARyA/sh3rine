use regex::Regex;
use std::collections::HashMap;

use crate::resolve;

pub struct HostPattern {
    pub regex: Regex,
    pub bucket_template: String,
    /// Pre-computed (placeholder, capture_name) pairs, e.g. ("{bucket}", "bucket")
    pub replacements: Vec<(String, String)>,
}

pub struct Config {
    pub endpoint: String,
    pub listen: String,
    /// Separate listen address for the /metrics endpoint. None = metrics disabled.
    pub metrics_listen: Option<String>,
    /// Exact hostname → bucket
    pub hosts: HashMap<String, String>,
    /// Regex patterns with named groups interpolated into bucket template
    pub host_patterns: Vec<HostPattern>,
    pub cache_max_mb: u64,
    pub cache_ttl_secs: u64,
    pub cache_max_bytes: u64,
    pub s3_timeout_secs: u64,
    pub cache_control: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let endpoint = std::env::var("ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:9000".to_string());
        let listen = std::env::var("LISTEN")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string());

        let mut hosts = HashMap::new();
        if let Ok(s) = std::env::var("HOSTS") {
            for entry in s.split('|') {
                if let Some((host, bucket)) = entry.split_once(':') {
                    let bucket = bucket.trim();
                    if !resolve::is_valid_bucket_name(bucket) {
                        return Err(format!("invalid bucket name in HOSTS: {bucket:?}").into());
                    }
                    hosts.insert(host.trim().to_string(), bucket.to_string());
                }
            }
        }

        let mut host_patterns = Vec::new();
        if let Ok(s) = std::env::var("HOST_PATTERNS") {
            for entry in s.split('|') {
                if let Some((pattern, template)) = entry.split_once(':') {
                    // template may contain colons (unlikely but safe via split_once)
                    let regex = Regex::new(pattern.trim())?;
                    let replacements = regex
                        .capture_names()
                        .flatten()
                        .map(|n| (format!("{{{n}}}"), n.to_string()))
                        .collect();
                    host_patterns.push(HostPattern {
                        regex,
                        bucket_template: template.trim().to_string(),
                        replacements,
                    });
                }
            }
        }

        Ok(Config {
            endpoint,
            listen,
            metrics_listen: std::env::var("METRICS_LISTEN").ok(),
            hosts,
            host_patterns,
            cache_max_mb: env_parse("CACHE_MAX_MB", 128)?,
            cache_ttl_secs: env_parse("CACHE_TTL_SECS", 60)?,
            cache_max_bytes: env_parse::<u64>("CACHE_MAX_FILE_KB", 512)? * 1024,
            s3_timeout_secs: env_parse("S3_TIMEOUT_SECS", 15)?,
            cache_control: std::env::var("CACHE_CONTROL").ok(),
        })
    }

    pub fn resolve_bucket(&self, host: &str) -> Option<String> {
        let host = host.split(':').next().unwrap_or(host);

        if let Some(bucket) = self.hosts.get(host) {
            return Some(bucket.clone());
        }

        for pat in &self.host_patterns {
            if let Some(caps) = pat.regex.captures(host) {
                let mut bucket = pat.bucket_template.clone();
                for (placeholder, group) in &pat.replacements {
                    if let Some(m) = caps.name(group) {
                        bucket = bucket.replace(placeholder.as_str(), m.as_str());
                    }
                }
                if !resolve::is_valid_bucket_name(&bucket) {
                    return None;
                }
                return Some(bucket);
            }
        }

        None
    }
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> Result<T, Box<dyn std::error::Error>>
where
    T::Err: std::error::Error + 'static,
{
    match std::env::var(key) {
        Ok(v) => Ok(v.parse()?),
        Err(_) => Ok(default),
    }
}
