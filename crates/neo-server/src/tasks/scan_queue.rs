//! The scan lane. Scans run one at a time like a one-permit semaphore, but
//! the waiting ones form a queue an admin can reorder, which a semaphore's
//! fixed first-come order does not allow.
//!
//! A scan keeps its place until its row has left `queued`: a cancelled one
//! until it is marked cancelled, and the one that takes the slot until it is
//! marked running. A reader that sees a queued scan row therefore always
//! finds its position (book/src/contract.md, `TaskSnapshot`).

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct ScanQueue {
    state: Mutex<State>,
    /// Woken whenever the slot frees up or the front of the queue changes.
    /// The queue holds one scan per library, so waking every waiter is cheap.
    changed: Notify,
}

#[derive(Default)]
struct State {
    /// A scan, or work that must not interleave with one, holds the slot.
    held: bool,
    /// The scan that took the slot but is not marked running yet. It stays
    /// first in line until then.
    starting: Option<i64>,
    /// Tasks waiting for the slot, the next one to run first.
    waiting: VecDeque<i64>,
}

impl ScanQueue {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("scan queue mutex poisoned")
    }

    /// Puts a task at the back of the queue. It keeps its place until the
    /// ticket takes the slot or is dropped.
    pub(super) fn join(self: &Arc<Self>, id: i64) -> Ticket {
        self.lock().waiting.push_back(id);
        Ticket {
            queue: Arc::clone(self),
            id,
            waiting: true,
        }
    }

    /// The slot, when it is free and no scan waits for it.
    pub(super) fn try_acquire(self: &Arc<Self>) -> Option<ScanPermit> {
        let mut state = self.lock();
        if state.held || !state.waiting.is_empty() {
            return None;
        }
        state.held = true;
        Some(ScanPermit {
            queue: Arc::clone(self),
            id: None,
        })
    }

    /// Moves a waiting task to the front. False when it is not waiting,
    /// because it already holds the slot or never joined.
    pub(super) fn move_to_front(&self, id: i64) -> bool {
        let mut state = self.lock();
        let Some(index) = state.waiting.iter().position(|&w| w == id) else {
            return false;
        };
        if index > 0 {
            state.waiting.remove(index);
            state.waiting.push_front(id);
            drop(state);
            self.changed.notify_waiters();
        }
        true
    }

    /// Where a task stands, 1 being the next to run. A scan that is
    /// starting is still 1, and the waiting ones come after it.
    pub(super) fn position(&self, id: i64) -> Option<usize> {
        let state = self.lock();
        if state.starting == Some(id) {
            return Some(1);
        }
        let ahead = usize::from(state.starting.is_some());
        state
            .waiting
            .iter()
            .position(|&w| w == id)
            .map(|index| index + 1 + ahead)
    }

    fn leave(&self, id: i64) {
        let mut state = self.lock();
        if let Some(index) = state.waiting.iter().position(|&w| w == id) {
            state.waiting.remove(index);
        }
        drop(state);
        // The task behind it may now be at the front of a free slot.
        self.changed.notify_waiters();
    }

    fn started(&self, id: i64) {
        let mut state = self.lock();
        if state.starting == Some(id) {
            state.starting = None;
        }
    }

    fn release(&self, id: Option<i64>) {
        let mut state = self.lock();
        state.held = false;
        if id.is_some() && state.starting == id {
            state.starting = None;
        }
        drop(state);
        self.changed.notify_waiters();
    }
}

/// A task's place in the queue.
pub(super) struct Ticket {
    queue: Arc<ScanQueue>,
    id: i64,
    waiting: bool,
}

impl Ticket {
    /// Waits until the slot is free and this task is at the front, then
    /// takes the slot. The task counts as first in line until
    /// [`ScanPermit::started`]. Dropping the ticket gives up the place in the queue.
    /// Dropping only the future keeps it, so a cancelled task can end its
    /// row before it leaves and is never seen queued without a place.
    pub(super) async fn acquire(&mut self) -> ScanPermit {
        loop {
            // Registered before the check, so a change between the check
            // and the wait still wakes this task.
            let changed = self.queue.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.queue.lock();
                if !state.held && state.waiting.front() == Some(&self.id) {
                    state.waiting.pop_front();
                    state.held = true;
                    state.starting = Some(self.id);
                    self.waiting = false;
                    return ScanPermit {
                        queue: Arc::clone(&self.queue),
                        id: Some(self.id),
                    };
                }
            }
            changed.await;
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if self.waiting {
            self.queue.leave(self.id);
        }
    }
}

/// The scan slot. Dropping it lets the task at the front of the queue run.
pub struct ScanPermit {
    queue: Arc<ScanQueue>,
    /// The scan holding the slot. `None` for work such as a cleanup.
    id: Option<i64>,
}

impl ScanPermit {
    /// Takes the scan out of first place once its row says running.
    pub(super) fn started(&self) {
        if let Some(id) = self.id {
            self.queue.started(id);
        }
    }
}

impl Drop for ScanPermit {
    fn drop(&mut self) {
        self.queue.release(self.id);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Owns the ticket in the spawned task, so aborting the task gives up
    /// the place as a dropped task would.
    async fn take(mut ticket: Ticket) -> ScanPermit {
        ticket.acquire().await
    }

    /// Takes the slot and marks the scan running, as `run` in tasks.rs does.
    async fn run(queue: &Arc<ScanQueue>, id: i64) -> ScanPermit {
        let permit = queue.join(id).acquire().await;
        permit.started();
        permit
    }

    async fn started(handle: &tokio::task::JoinHandle<ScanPermit>) -> bool {
        tokio::time::sleep(Duration::from_millis(20)).await;
        handle.is_finished()
    }

    #[tokio::test]
    async fn waiting_scans_run_in_queue_order_after_a_move() {
        let queue = Arc::new(ScanQueue::default());
        let first = run(&queue, 1).await;
        let second = tokio::spawn(take(queue.join(2)));
        let third = tokio::spawn(take(queue.join(3)));
        assert_eq!((queue.position(2), queue.position(3)), (Some(1), Some(2)));

        assert!(queue.move_to_front(3));
        assert_eq!((queue.position(3), queue.position(2)), (Some(1), Some(2)));
        assert!(!started(&third).await, "the running scan keeps the slot");

        drop(first);
        let third = third.await.unwrap();
        third.started();
        assert!(!started(&second).await);
        assert_eq!(queue.position(2), Some(1));
        drop(third);
        drop(second.await.unwrap());
        assert!(queue.try_acquire().is_some());
    }

    #[tokio::test]
    async fn a_starting_scan_stays_first_until_it_is_marked_running() {
        let queue = Arc::new(ScanQueue::default());
        let first = run(&queue, 1).await;
        let next = tokio::spawn(take(queue.join(2)));
        let _last = queue.join(3);
        drop(first);
        let next = next.await.unwrap();
        assert_eq!((queue.position(2), queue.position(3)), (Some(1), Some(2)));
        assert!(!queue.move_to_front(2), "it holds the slot");

        next.started();
        assert_eq!((queue.position(2), queue.position(3)), (None, Some(1)));
    }

    #[tokio::test]
    async fn moving_a_task_that_is_not_waiting_changes_nothing() {
        let queue = Arc::new(ScanQueue::default());
        let running = run(&queue, 1).await;
        let _waiting = queue.join(2);
        assert!(!queue.move_to_front(1));
        assert!(!queue.move_to_front(9));
        assert!(queue.move_to_front(2), "already first, still waiting");
        assert_eq!(queue.position(2), Some(1));
        drop(running);
    }

    #[tokio::test]
    async fn a_dropped_waiter_leaves_and_lets_the_next_one_run() {
        let queue = Arc::new(ScanQueue::default());
        let running = run(&queue, 1).await;
        let cancelled = tokio::spawn(take(queue.join(2)));
        let next = tokio::spawn(take(queue.join(3)));
        assert!(!started(&next).await);

        drop(running);
        // Task 2 may have taken the slot by now. Aborting it either drops its
        // place in the queue or drops the permit it got, and both free task 3.
        cancelled.abort();
        let _ = cancelled.await;
        drop(next.await.unwrap());
        assert_eq!(queue.position(2), None);
    }

    #[tokio::test]
    async fn the_free_slot_goes_to_a_waiter_before_try_acquire() {
        let queue = Arc::new(ScanQueue::default());
        let held = queue.try_acquire().expect("free at first");
        assert!(queue.try_acquire().is_none());
        let waiter = tokio::spawn(take(queue.join(1)));
        assert!(!started(&waiter).await);
        drop(held);
        // The waiter wins the slot even if try_acquire runs before it wakes.
        assert!(queue.try_acquire().is_none());
        drop(waiter.await.unwrap());
        assert!(queue.try_acquire().is_some());
    }
}
