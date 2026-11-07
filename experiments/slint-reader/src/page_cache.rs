//! At most two adjacent native rasters. Values never leave their COM thread.
use std::collections::VecDeque;

pub const MAX_BYTES: usize = 24 * 1024 * 1024;
pub struct Cache<K, V> {
    entries: VecDeque<(K, V, usize)>,
    bytes: usize,
}
impl<K: PartialEq, V> Default for Cache<K, V> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
        }
    }
}
impl<K: PartialEq, V> Cache<K, V> {
    pub fn take(&mut self, key: &K) -> Option<V> {
        let at = self.entries.iter().position(|(k, _, _)| k == key)?;
        let (_, value, bytes) = self.entries.remove(at)?;
        self.bytes -= bytes;
        Some(value)
    }
    pub fn contains(&self, key: &K) -> bool {
        self.entries.iter().any(|(k, _, _)| k == key)
    }
    pub fn insert(&mut self, key: K, value: V, bytes: usize) {
        if bytes == 0 || bytes > MAX_BYTES {
            return;
        }
        self.take(&key);
        while self.entries.len() >= 2 || self.bytes + bytes > MAX_BYTES {
            if let Some((_, _, old)) = self.entries.pop_front() {
                self.bytes -= old;
            }
        }
        self.entries.push_back((key, value, bytes));
        self.bytes += bytes;
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.iter().map(|(key, _, _)| key)
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memory_and_entry_limits_evict_without_growing() {
        let mut c = Cache::default();
        c.insert(1, "one", MAX_BYTES / 2);
        c.insert(2, "two", MAX_BYTES / 2);
        c.insert(3, "three", MAX_BYTES / 2);
        assert_eq!(c.take(&1), None);
        assert_eq!(c.bytes(), MAX_BYTES);
        assert_eq!(c.take(&2), Some("two"));
        c.insert(4, "oversized", MAX_BYTES + 1);
        assert_eq!(c.take(&4), None);
        c.insert(3, "replacement", 1);
        assert_eq!(c.bytes(), 1);
        assert_eq!(c.take(&3), Some("replacement"));
        assert_eq!(c.bytes(), 0);
    }
    #[test]
    fn presentation_keys_cannot_reuse_another_scale_or_theme() {
        let mut c = Cache::default();
        c.insert((319, 125, 0), "dark at 125%", 40);
        assert!(!c.contains(&(319, 100, 0)));
        assert!(!c.contains(&(319, 125, 1)));
        assert_eq!(c.take(&(319, 125, 0)), Some("dark at 125%"));
    }
}
