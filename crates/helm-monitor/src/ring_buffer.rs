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
}
