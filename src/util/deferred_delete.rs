//! Deferred deletion queue

#[derive(Debug)]
struct Item<T> {
    _value: T,
    // Time to live
    ttl: u32,
}

/// Deletion queue that stores resources until they are ready to be deleted.
#[derive(Debug)]
pub struct DeletionQueue<T> {
    max_ttl: u32,
    items: Vec<Item<T>>,
}

impl<T> DeletionQueue<T> {
    /// Create a new deletion queue. Items that are pushed onto this queue will be kept alive
    /// for `max_ttl` calls to [`DeletionQueue::next_frame`]
    pub fn new(max_ttl: u32) -> DeletionQueue<T> {
        DeletionQueue {
            max_ttl,
            items: vec![],
        }
    }

    /// Pushes a value onto the deletion queue.
    /// Note that this moves out of the parameter so that you can't access an object after
    /// it is pushed.
    pub fn push(&mut self, value: T) {
        self.items.push(Item {
            _value: value,
            ttl: self.max_ttl,
        });
    }

    /// Advance the frame counter by one, decreasing time to live by one on each element.
    /// If time to live of an element reaches zero, it is deleted.
    pub fn next_frame(&mut self) {
        self.items.iter_mut().for_each(|mut item| item.ttl -= 1);
        self.items.retain(|item| item.ttl != 0);
    }
}
