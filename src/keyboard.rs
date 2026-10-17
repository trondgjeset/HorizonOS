use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

const BUFFER_CAPACITY: usize = 128;
const BUFFER_MASK: usize = BUFFER_CAPACITY - 1;

static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);
static DROPPED_CHARACTERS: AtomicUsize = AtomicUsize::new(0);
static BUFFER: [AtomicU8; BUFFER_CAPACITY] = [const { AtomicU8::new(0) }; BUFFER_CAPACITY];

/// Enqueues one translated ASCII character from the keyboard interrupt path.
///
/// Returns `false` and increments the dropped-character counter when the
/// bounded queue is full.
pub fn push_character(character: u8) -> bool {
    let head = HEAD.load(Ordering::Relaxed);
    let next = head.wrapping_add(1);
    if next.wrapping_sub(TAIL.load(Ordering::Acquire)) > BUFFER_CAPACITY {
        DROPPED_CHARACTERS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    BUFFER[head & BUFFER_MASK].store(character, Ordering::Relaxed);
    HEAD.store(next, Ordering::Release);
    true
}

/// Removes the oldest queued character, or returns `None` when the queue is empty.
pub fn pop_character() -> Option<u8> {
    let tail = TAIL.load(Ordering::Relaxed);
    if tail == HEAD.load(Ordering::Acquire) {
        return None;
    }
    let character = BUFFER[tail & BUFFER_MASK].load(Ordering::Relaxed);
    TAIL.store(tail.wrapping_add(1), Ordering::Release);
    Some(character)
}

/// Reports whether at least one keyboard character is waiting to be consumed.
pub fn has_pending_input() -> bool {
    TAIL.load(Ordering::Acquire) != HEAD.load(Ordering::Acquire)
}

/// Returns and clears the number of characters dropped since the last call.
pub fn take_dropped_count() -> usize {
    DROPPED_CHARACTERS.swap(0, Ordering::AcqRel)
}

#[cfg(test)]
mod tests {
    use super::{
        has_pending_input, pop_character, push_character, take_dropped_count, BUFFER_CAPACITY,
    };

    #[test]
    fn queue_preserves_order_and_rejects_overflow() {
        assert!(!has_pending_input());
        assert!(push_character(b'a'));
        assert!(push_character(b'b'));
        assert!(has_pending_input());
        assert_eq!(pop_character(), Some(b'a'));
        assert_eq!(pop_character(), Some(b'b'));
        assert_eq!(pop_character(), None);

        for value in 0..BUFFER_CAPACITY {
            assert!(push_character(value as u8));
        }
        assert!(!push_character(b'x'));
        assert_eq!(take_dropped_count(), 1);
        assert_eq!(pop_character(), Some(0));
        assert!(push_character(b'x'));
        assert_eq!(take_dropped_count(), 0);
    }
}
