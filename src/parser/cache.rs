//! Parser 热路径使用的小型有界缓存.

use std::cell::RefCell;
use std::collections::VecDeque;

pub(crate) const MAX_CACHE_INPUT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_CACHE_ENTRY_BYTES: usize = 64 * 1024;

pub(crate) struct FixedCache<K, V> {
    entries: VecDeque<(K, V)>,
    capacity: usize,
}

impl<K, V> FixedCache<K, V> {
    pub(crate) const fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
        }
    }
}

impl<K: PartialEq, V: Clone> FixedCache<K, V> {
    fn get(&self, key: &K) -> Option<V> {
        self.entries
            .iter()
            .find_map(|(cached_key, value)| (cached_key == key).then(|| value.clone()))
    }
}

impl<K: PartialEq, V> FixedCache<K, V> {
    fn insert(&mut self, key: K, value: V) {
        if let Some((_, cached_value)) = self
            .entries
            .iter_mut()
            .find(|(cached_key, _)| cached_key == &key)
        {
            *cached_value = value;
            return;
        }

        if self.entries.len() >= self.capacity && self.capacity > 0 {
            self.entries.pop_front();
        }

        if self.capacity > 0 {
            self.entries.push_back((key, value));
        }
    }
}

pub(crate) fn get_or_try_insert_with<K, V, E>(
    cache: &RefCell<FixedCache<K, V>>,
    key: K,
    input_bytes: usize,
    value_bytes: impl FnOnce(&V) -> usize,
    build: impl FnOnce(&K) -> Result<V, E>,
) -> Result<V, E>
where
    K: PartialEq,
    V: Clone,
{
    if input_bytes <= MAX_CACHE_INPUT_BYTES
        && let Some(value) = cache.borrow().get(&key)
    {
        return Ok(value);
    }

    let value = build(&key)?;
    if input_bytes <= MAX_CACHE_INPUT_BYTES
        && input_bytes.saturating_add(value_bytes(&value)) <= MAX_CACHE_ENTRY_BYTES
    {
        cache.borrow_mut().insert(key, value.clone());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_values_are_not_cached() {
        let cache = RefCell::new(FixedCache::new(1));
        let mut builds = 0;

        for _ in 0..2 {
            let value = get_or_try_insert_with(
                &cache,
                "key",
                3,
                |_| MAX_CACHE_ENTRY_BYTES,
                |_| {
                    builds += 1;
                    Ok::<_, ()>("value".to_owned())
                },
            )
            .unwrap();
            assert_eq!(value, "value");
        }

        assert_eq!(builds, 2);
    }

    #[test]
    fn oversized_inputs_are_not_cached() {
        let cache = RefCell::new(FixedCache::new(1));
        let mut builds = 0;

        for _ in 0..2 {
            get_or_try_insert_with(
                &cache,
                "key",
                MAX_CACHE_INPUT_BYTES + 1,
                |_| 1,
                |_| {
                    builds += 1;
                    Ok::<_, ()>(())
                },
            )
            .unwrap();
        }

        assert_eq!(builds, 2);
    }
}
