//! Fixed-capacity ring buffer for lightweight metric history.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RingBuffer<T> {
    capacity: usize,
    values: VecDeque<T>,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            values: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    pub fn push(&mut self, value: T) {
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.values.iter()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<T> Default for RingBuffer<T> {
    fn default() -> Self {
        Self::new(60)
    }
}

#[cfg(test)]
mod tests {
    use super::RingBuffer;

    #[test]
    fn keeps_last_n_values_in_order() {
        let mut buffer = RingBuffer::new(3);
        for value in 1..=5 {
            buffer.push(value);
        }

        let values = buffer.iter().copied().collect::<Vec<_>>();
        assert_eq!(values, vec![3, 4, 5]);
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn zero_capacity_is_clamped_to_one() {
        let mut buffer = RingBuffer::new(0);
        buffer.push("first");
        buffer.push("second");

        let values = buffer.iter().copied().collect::<Vec<_>>();
        assert_eq!(values, vec!["second"]);
        assert_eq!(buffer.capacity(), 1);
    }

    // ── Property tests ──────────────────────────────────────────────────────

    #[cfg(test)]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Property 1: Capacity invariant — len() never exceeds capacity after any
            /// number of pushes.
            #[test]
            fn property_len_never_exceeds_capacity(
                capacity in 1usize..256,
                pushes in proptest::collection::vec(0i32..1000, 0..512),
            ) {
                let mut buf = RingBuffer::new(capacity);
                for v in pushes {
                    buf.push(v);
                    prop_assert!(buf.len() <= buf.capacity());
                }
            }

            /// Property 2: After pushing exactly `capacity` items, len() == capacity.
            #[test]
            fn property_full_buffer_len_equals_capacity(capacity in 1usize..128) {
                let mut buf = RingBuffer::new(capacity);
                for i in 0..capacity {
                    buf.push(i as i32);
                }
                prop_assert_eq!(buf.len(), capacity);
            }

            /// Property 3: Circular overwrite — the last `capacity` pushed values are
            /// exactly the iterator output.
            #[test]
            fn property_last_n_values_preserved(
                capacity in 1usize..64,
                extra in 0usize..64,
            ) {
                let total = capacity + extra;
                let mut buf = RingBuffer::new(capacity);
                let items: Vec<i32> = (0..total as i32).collect();
                for &v in &items {
                    buf.push(v);
                }
                let expected: Vec<i32> = items[items.len() - capacity..].to_vec();
                let got: Vec<i32> = buf.iter().copied().collect();
                prop_assert_eq!(got, expected);
            }

            /// Property 4: is_empty() is true iff len() == 0.
            #[test]
            fn property_is_empty_consistent_with_len(
                capacity in 1usize..64,
                pushes in 0usize..128,
            ) {
                let mut buf = RingBuffer::new(capacity);
                for i in 0..pushes {
                    buf.push(i as i32);
                }
                prop_assert_eq!(buf.is_empty(), buf.len() == 0);
            }
        }
    }
}
