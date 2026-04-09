use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DiskCache {
    dir: PathBuf,
}

impl Default for DiskCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskCache {
    pub fn new() -> Self {
        let dir = default_cache_dir();
        std::fs::create_dir_all(&dir).ok();
        Self { dir }
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).ok();
        Self { dir }
    }

    pub fn cache_key(url: &str, range: Option<(u64, u64)>) -> String {
        match range {
            Some((start, end_exclusive)) => format!("{url}|{start}-{end_exclusive}"),
            None => url.to_string(),
        }
    }

    pub fn cache_key_ranges(url: &str, ranges: &[(u64, u64)]) -> String {
        let mut key = format!("{url}|ranges:");
        for (index, (start, end_exclusive)) in ranges.iter().enumerate() {
            if index > 0 {
                key.push(',');
            }
            key.push_str(&format!("{start}-{end_exclusive}"));
        }
        key
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.cache_path(key)).ok()
    }

    pub fn put(&self, key: &str, data: &[u8]) {
        let path = self.cache_path(key);
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }
        let _ = std::fs::write(path, data);
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    fn cache_path(&self, key: &str) -> PathBuf {
        let hash = hash_key(key);
        self.dir.join(&hash[..2]).join(format!("{hash}.bin"))
    }
}

fn default_cache_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("rustbox").join("cache");
    }
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("rustbox");
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home).join(".cache").join("rustbox");
    }

    PathBuf::from(".rustbox").join("cache")
}

fn hash_key(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_deterministic() {
        assert_eq!(
            DiskCache::cache_key("https://example.com/a", Some((1, 5))),
            DiskCache::cache_key("https://example.com/a", Some((1, 5)))
        );
    }

    #[test]
    fn cache_round_trip_works() {
        let root = std::env::temp_dir().join(format!("rustbox-cache-test-{}", std::process::id()));
        let cache = DiskCache::with_dir(root.clone());
        let key = DiskCache::cache_key("https://example.com/a", Some((10, 20)));
        cache.put(&key, b"hello");
        assert_eq!(cache.get(&key), Some(b"hello".to_vec()));
        let _ = std::fs::remove_dir_all(root);
    }
}
