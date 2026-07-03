//! Small fixed-size caches used by parser and pattern hot paths.

use std::cell::RefCell;
use std::collections::VecDeque;

pub(crate) struct FixedCache<K, V> {
    entries: VecDeque<(K, V)>,
    capacity: usize,
}

impl<K, V> FixedCache<K, V> {
    pub(crate) fn new(capacity: usize) -> Self {
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
    build: impl FnOnce(&K) -> Result<V, E>,
) -> Result<V, E>
where
    K: Clone + PartialEq,
    V: Clone,
{
    if let Some(value) = cache.borrow().get(&key) {
        return Ok(value);
    }

    let value = build(&key)?;
    cache.borrow_mut().insert(key, value.clone());
    Ok(value)
}
