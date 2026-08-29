//! A fixed-capacity ring buffer that drops the *oldest* entry on overflow
//! and counts what it dropped, for order-sensitive streams where gaps must
//! be visible rather than silently absorbed (event log, alerts — landing in
//! later phases; exercised here so the primitive is tested before anything
//! depends on it).

use std::collections::VecDeque;

pub struct BoundedRing<T> {
    capacity: usize,
    items: VecDeque<T>,
    dropped: u64,
}

impl<T> BoundedRing<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "BoundedRing capacity must be > 0");
        Self {
            capacity,
            items: VecDeque::with_capacity(capacity),
            dropped: 0,
        }
    }

    /// Pushes a new item, dropping the oldest one first if already at
    /// capacity. Returns whether an item had to be dropped.
    pub fn push(&mut self, item: T) -> bool {
        let mut did_drop = false;
        if self.items.len() == self.capacity {
            self.items.pop_front();
            self.dropped += 1;
            did_drop = true;
        }
        self.items.push_back(item);
        did_drop
    }

    /// Items oldest-first, i.e. in the order they'll be dropped/consumed.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_exceeds_capacity() {
        let mut ring = BoundedRing::new(3);
        for i in 0..10 {
            ring.push(i);
        }
        assert_eq!(ring.len(), 3);
    }

    #[test]
    fn overflow_drops_oldest_and_counts_it() {
        let mut ring = BoundedRing::new(3);
        for i in 0..5 {
            ring.push(i);
        }
        // 0 and 1 were dropped; 2,3,4 survive, oldest-first.
        assert_eq!(ring.dropped(), 2);
        let survivors: Vec<i32> = ring.iter().copied().collect();
        assert_eq!(survivors, vec![2, 3, 4]);
    }

    #[test]
    fn survivor_order_is_preserved_across_repeated_overflow() {
        let mut ring: BoundedRing<u32> = BoundedRing::new(2);
        let drops: Vec<bool> = (0..6).map(|i| ring.push(i)).collect();
        assert_eq!(drops, vec![false, false, true, true, true, true]);
        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), vec![4, 5]);
        assert_eq!(ring.dropped(), 4);
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn zero_capacity_panics_at_construction() {
        let _: BoundedRing<u32> = BoundedRing::new(0);
    }
}
