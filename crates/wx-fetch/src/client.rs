use crate::cache::DiskCache;
use anyhow::{Result, bail};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub timeout: Duration,
    pub max_retries: u32,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadClient {
    client: reqwest::blocking::Client,
    max_retries: u32,
    cache: Option<DiskCache>,
}

impl DownloadClient {
    pub fn new() -> Result<Self> {
        Self::new_with_config(DownloadConfig::default())
    }

    pub fn new_with_config(config: DownloadConfig) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .use_rustls_tls()
            .timeout(config.timeout)
            .user_agent("rustbox/0.1.0")
            .build()?;

        Ok(Self {
            client,
            max_retries: config.max_retries,
            cache: None,
        })
    }

    pub fn with_cache_dir(cache_dir: Option<std::path::PathBuf>) -> Result<Self> {
        let mut client = Self::new()?;
        client.cache = Some(match cache_dir {
            Some(path) => DiskCache::with_dir(path),
            None => DiskCache::new(),
        });
        Ok(client)
    }

    pub fn set_cache(&mut self, cache: DiskCache) {
        self.cache = Some(cache);
    }

    pub fn head_ok(&self, url: &str) -> bool {
        for attempt in 0..=self.max_retries {
            match self.client.head(url).send() {
                Ok(response) if response.status().is_success() => return true,
                Ok(response)
                    if !should_retry_status(response.status()) || attempt == self.max_retries =>
                {
                    return false;
                }
                Err(error) if !should_retry_error(&error) || attempt == self.max_retries => {
                    return false;
                }
                _ => std::thread::sleep(backoff_duration(attempt)),
            }
        }

        false
    }

    pub fn get_text(&self, url: &str) -> Result<String> {
        self.with_retry(url, || {
            self.client.get(url).send()?.error_for_status()?.text()
        })
    }

    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let key = DiskCache::cache_key(url, None);
        if let Some(cache) = &self.cache
            && let Some(data) = cache.get(&key)
        {
            return Ok(data);
        }

        let bytes = self.with_retry(url, || {
            Ok(self
                .client
                .get(url)
                .send()?
                .error_for_status()?
                .bytes()?
                .to_vec())
        })?;

        if let Some(cache) = &self.cache {
            cache.put(&key, &bytes);
        }

        Ok(bytes)
    }

    pub fn get_range(&self, url: &str, start: u64, end_exclusive: u64) -> Result<Vec<u8>> {
        if end_exclusive <= start {
            bail!("invalid exclusive byte range {start}..{end_exclusive}");
        }
        let key = DiskCache::cache_key(url, Some((start, end_exclusive)));
        if let Some(cache) = &self.cache
            && let Some(data) = cache.get(&key)
        {
            return Ok(data);
        }

        let end_inclusive = end_exclusive - 1;
        let header = format!("bytes={start}-{end_inclusive}");
        let bytes = self.with_retry(url, || {
            Ok(self
                .client
                .get(url)
                .header(reqwest::header::RANGE, header.clone())
                .send()?
                .error_for_status()?
                .bytes()?
                .to_vec())
        })?;

        if let Some(cache) = &self.cache {
            cache.put(&key, &bytes);
        }

        Ok(bytes)
    }

    pub fn get_ranges(&self, url: &str, ranges: &[(u64, u64)]) -> Result<Vec<u8>> {
        let key = DiskCache::cache_key_ranges(url, ranges);
        if let Some(cache) = &self.cache
            && let Some(data) = cache.get(&key)
        {
            return Ok(data);
        }

        let mut combined = Vec::new();
        for (start, end_exclusive) in ranges {
            combined.extend_from_slice(&self.get_range(url, *start, *end_exclusive)?);
        }

        if let Some(cache) = &self.cache {
            cache.put(&key, &combined);
        }

        Ok(combined)
    }

    fn with_retry<T, F>(&self, url: &str, mut op: F) -> Result<T>
    where
        F: FnMut() -> Result<T, reqwest::Error>,
    {
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            match op() {
                Ok(value) => return Ok(value),
                Err(error) if should_retry_error(&error) && attempt < self.max_retries => {
                    last_error = Some(error);
                    std::thread::sleep(backoff_duration(attempt));
                }
                Err(error) => return Err(error.into()),
            }
        }

        Err(anyhow::anyhow!(
            "request retries exhausted for {url}: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown error".to_string())
        ))
    }
}

fn should_retry_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

fn should_retry_error(error: &reqwest::Error) -> bool {
    error
        .status()
        .map(should_retry_status)
        .unwrap_or_else(|| error.is_timeout() || error.is_connect() || error.is_request())
}

fn backoff_duration(attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_millis(400),
        1 => Duration::from_millis(900),
        _ => Duration::from_millis(1_800),
    }
}
