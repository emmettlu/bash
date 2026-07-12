//! 解析器和 pattern 热路径使用的小型有界缓存.

use std::cell::RefCell;
use std::collections::VecDeque;

pub(crate) const MAX_CACHE_KEY_BYTES: usize = 16 * 1024;
const MAX_TOTAL_CACHE_KEY_BYTES: usize = 256 * 1024;

struct CacheEntry<K, V> {
    key: K,
    value: V,
    key_bytes: usize,
}

pub(crate) struct FixedCache<K, V> {
    entries: VecDeque<CacheEntry<K, V>>,
    capacity: usize,
    total_key_bytes: usize,
    max_total_key_bytes: usize,
}

impl<K, V> FixedCache<K, V> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self::with_key_byte_budget(capacity, MAX_TOTAL_CACHE_KEY_BYTES)
    }

    fn with_key_byte_budget(capacity: usize, max_total_key_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
            total_key_bytes: 0,
            max_total_key_bytes,
        }
    }

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.total_key_bytes = 0;
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

impl<K: PartialEq, V: Clone> FixedCache<K, V> {
    fn get(&self, key: &K) -> Option<V> {
        self.entries
            .iter()
            .find_map(|entry| (entry.key == *key).then(|| entry.value.clone()))
    }
}

impl<K: PartialEq, V> FixedCache<K, V> {
    fn insert(&mut self, key: K, value: V, key_bytes: usize) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == key) {
            entry.value = value;
            return;
        }

        if self.capacity == 0 || key_bytes > self.max_total_key_bytes {
            return;
        }

        while self.entries.len() >= self.capacity
            || self.total_key_bytes.saturating_add(key_bytes) > self.max_total_key_bytes
        {
            let Some(removed) = self.entries.pop_front() else {
                break;
            };
            self.total_key_bytes = self.total_key_bytes.saturating_sub(removed.key_bytes);
        }

        self.total_key_bytes += key_bytes;
        self.entries.push_back(CacheEntry {
            key,
            value,
            key_bytes,
        });
    }
}

pub(crate) fn get_or_try_insert_with<K, V, E>(
    cache: &RefCell<FixedCache<K, V>>,
    key: K,
    key_bytes: usize,
    build: impl FnOnce(&K) -> Result<V, E>,
) -> Result<V, E>
where
    K: PartialEq,
    V: Clone,
{
    if key_bytes <= MAX_CACHE_KEY_BYTES
        && let Some(value) = cache.borrow().get(&key)
    {
        return Ok(value);
    }

    let value = build(&key)?;
    if key_bytes <= MAX_CACHE_KEY_BYTES {
        cache.borrow_mut().insert(key, value.clone(), key_bytes);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_keys_hit_the_cache() {
        let cache = RefCell::new(FixedCache::new(1));
        let mut builds = 0;

        for _ in 0..2 {
            let value = get_or_try_insert_with(&cache, "key", 3, |_| {
                builds += 1;
                Ok::<_, ()>("value".to_owned())
            })
            .unwrap();
            assert_eq!(value, "value");
        }

        assert_eq!(builds, 1);
    }

    #[test]
    fn oversized_keys_are_not_cached() {
        let cache = RefCell::new(FixedCache::new(1));
        let mut builds = 0;

        for _ in 0..2 {
            get_or_try_insert_with(&cache, "key", MAX_CACHE_KEY_BYTES + 1, |_| {
                builds += 1;
                Ok::<_, ()>(())
            })
            .unwrap();
        }

        assert_eq!(builds, 2);
        assert_eq!(cache.borrow().len(), 0);
    }

    #[test]
    fn total_key_budget_evicts_oldest_entries() {
        let cache = RefCell::new(FixedCache::with_key_byte_budget(3, 6));
        let mut builds = 0;

        for key in ["aaa", "bbb", "aaa", "ccc", "aaa"] {
            get_or_try_insert_with(&cache, key, key.len(), |_| {
                builds += 1;
                Ok::<_, ()>(())
            })
            .unwrap();
        }

        assert_eq!(builds, 4);
        assert_eq!(cache.borrow().len(), 2);
    }
}
