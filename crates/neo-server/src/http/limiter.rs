//! In-memory limits on failed password checks, as described in "密码错误的
//! 次数限制" in book/src/auth.md.
//!
//! Counts live in fixed-size count-min sketches instead of per-key tables:
//! memory never grows, nothing is evicted, so a flood of fresh keys can
//! neither reset an attacker's count nor fill a table. Hash collisions can
//! only over-count, which under a large flood may slow down or refuse an
//! innocent source. That is the accepted trade-off.

use std::{
    hash::{BuildHasher, Hash, RandomState},
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

/// A source with this many failures in the current and previous window is
/// refused with 429.
pub const SOURCE_LIMIT: u32 = 10;
const SOURCE_WINDOW: Duration = Duration::from_secs(60);
/// A user name past this many failures is slowed down, never refused, so
/// nobody can lock the owner of a one-user server out by guessing at the
/// admin's name. Two five-minute windows approximate ten minutes.
pub const USER_LIMIT: u32 = 20;
const USER_WINDOW: Duration = Duration::from_secs(5 * 60);
/// Past this many failures per minute across all sources and names, every
/// attempt is slowed down.
pub const GLOBAL_LIMIT: u32 = 200;
const GLOBAL_WINDOW: Duration = Duration::from_secs(60);
pub const DELAY: Duration = Duration::from_secs(1);

const ROWS: usize = 4;
const WIDTH: usize = 4096;
/// User names in the database are at most 64 characters, so a longer
/// requested name cannot match a user and is cut down before hashing.
const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Check the password, but only after `DELAY`.
    Delay,
    /// Refuse with 429 without checking the password.
    Block,
}

/// The address only, never the port. IPv6 clients usually control a whole
/// /64, so they are counted per /64.
fn source_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6((u128::from(v6) & !((1u128 << 64) - 1)).into()),
        },
    }
}

fn user_key(name: &str) -> String {
    name.chars().take(MAX_NAME_CHARS).collect()
}

fn epoch_of(start: Instant, window: Duration, now: Instant) -> u64 {
    (now.saturating_duration_since(start).as_nanos() / window.as_nanos()) as u64
}

/// Two rotating windows. Rotation moves current to previous and clears
/// only the new current. A `now` older than the current window, as when a
/// request read the clock before waiting for the lock, counts in the
/// current window and never clears anything.
struct Windows<T> {
    window: Duration,
    start: Instant,
    epoch: u64,
    current: T,
    previous: T,
    empty: fn() -> T,
}

impl<T> Windows<T> {
    fn new(window: Duration, start: Instant, empty: fn() -> T) -> Self {
        Self {
            window,
            start,
            epoch: 0,
            current: empty(),
            previous: empty(),
            empty,
        }
    }

    fn rotate(&mut self, now: Instant) -> u64 {
        let epoch = epoch_of(self.start, self.window, now);
        if epoch > self.epoch {
            if epoch == self.epoch + 1 {
                std::mem::swap(&mut self.current, &mut self.previous);
            } else {
                // Nothing counted before the previous window still matters.
                self.previous = (self.empty)();
            }
            self.current = (self.empty)();
            self.epoch = epoch;
        }
        self.epoch
    }

    /// The window that holds counts made in `epoch`, if it is still kept.
    fn holding(&mut self, epoch: u64, now: Instant) -> Option<&mut T> {
        let current = self.rotate(now);
        if epoch == current {
            Some(&mut self.current)
        } else if epoch + 1 == current {
            Some(&mut self.previous)
        } else {
            None
        }
    }
}

type Cells = Box<[[u16; WIDTH]; ROWS]>;

fn empty_cells() -> Cells {
    Box::new([[0; WIDTH]; ROWS])
}

/// A count-min sketch over two rotating windows. The count for a key is the
/// smallest over all rows of current plus previous window.
struct Sketch {
    /// SipHash keyed randomly per process and per row, so an attacker can
    /// neither precompute keys that collide with a victim's nor steer clear
    /// of their own cells.
    hashers: [RandomState; ROWS],
    windows: Windows<Cells>,
}

impl Sketch {
    fn new(window: Duration, start: Instant) -> Self {
        Self {
            hashers: std::array::from_fn(|_| RandomState::new()),
            windows: Windows::new(window, start, empty_cells),
        }
    }

    fn slots<K: Hash>(&self, key: &K) -> [usize; ROWS] {
        std::array::from_fn(|row| (self.hashers[row].hash_one(key) as usize) % WIDTH)
    }

    fn count<K: Hash>(&mut self, key: &K, now: Instant) -> u32 {
        self.windows.rotate(now);
        let w = &self.windows;
        self.slots(key)
            .iter()
            .enumerate()
            .map(|(row, &i)| u32::from(w.current[row][i]) + u32::from(w.previous[row][i]))
            .min()
            .unwrap_or(0)
    }

    /// Counters saturate instead of wrapping, so they never fall back to 0.
    fn add<K: Hash>(&mut self, key: &K, now: Instant) -> u64 {
        let epoch = self.windows.rotate(now);
        for (row, i) in self.slots(key).into_iter().enumerate() {
            let cell = &mut self.windows.current[row][i];
            *cell = cell.saturating_add(1);
        }
        epoch
    }

    /// Takes back one count added in window `epoch`. A count in a window
    /// that has since been dropped is gone anyway.
    fn remove<K: Hash>(&mut self, key: &K, epoch: u64, now: Instant) {
        let slots = self.slots(key);
        if let Some(cells) = self.windows.holding(epoch, now) {
            for (row, i) in slots.into_iter().enumerate() {
                cells[row][i] = cells[row][i].saturating_sub(1);
            }
        }
    }
}

struct State {
    sources: Sketch,
    users: Sketch,
    total: Windows<u32>,
}

impl State {
    fn total(&mut self, now: Instant) -> u32 {
        self.total.rotate(now);
        self.total.current.saturating_add(self.total.previous)
    }

    fn verdict(&mut self, source: &IpAddr, user: &String, now: Instant) -> Verdict {
        if self.sources.count(source, now) >= SOURCE_LIMIT {
            Verdict::Block
        } else if self.users.count(user, now) >= USER_LIMIT || self.total(now) >= GLOBAL_LIMIT {
            Verdict::Delay
        } else {
            Verdict::Allow
        }
    }
}

/// One counted attempt, kept so that a correct password can be refunded.
#[derive(Debug)]
pub struct Reservation {
    source: IpAddr,
    user: String,
    source_epoch: u64,
    user_epoch: u64,
    total_epoch: u64,
}

pub struct LoginLimiter {
    state: Mutex<State>,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl LoginLimiter {
    fn new(start: Instant) -> Self {
        Self {
            state: Mutex::new(State {
                sources: Sketch::new(SOURCE_WINDOW, start),
                users: Sketch::new(USER_WINDOW, start),
                total: Windows::new(GLOBAL_WINDOW, start, || 0),
            }),
        }
    }

    /// Decides on an attempt and, unless it is refused, counts it as a
    /// failure right away under the same lock. Concurrent attempts cannot
    /// all pass the check before any of them is counted, and an attempt
    /// that never finishes (an error, a panic in argon2, a client that
    /// disconnects) stays counted. Only `refund` after a correct password
    /// takes the count back. Every failure counts the same, whether the
    /// password was wrong, the name unknown or the account has no password.
    ///
    /// Only the source count can refuse. The name and global counts only
    /// add a delay, so a well-behaved source can always log in.
    pub fn reserve(&self, ip: IpAddr, user: &str, now: Instant) -> (Verdict, Option<Reservation>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let source = source_key(ip);
        let user = user_key(user);
        let verdict = s.verdict(&source, &user, now);
        if verdict == Verdict::Block {
            return (verdict, None);
        }
        let source_epoch = s.sources.add(&source, now);
        let user_epoch = s.users.add(&user, now);
        let total_epoch = s.total.rotate(now);
        s.total.current = s.total.current.saturating_add(1);
        let reservation = Reservation {
            source,
            user,
            source_epoch,
            user_epoch,
            total_epoch,
        };
        (verdict, Some(reservation))
    }

    /// Takes back the count of an attempt whose password was correct. It
    /// only touches that attempt's own cells, so other keys keep theirs.
    pub fn refund(&self, r: Reservation, now: Instant) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.sources.remove(&r.source, r.source_epoch, now);
        s.users.remove(&r.user, r.user_epoch, now);
        if let Some(total) = s.total.holding(r.total_epoch, now) {
            *total = total.saturating_sub(1);
        }
    }

    #[cfg(test)]
    fn check(&self, ip: IpAddr, user: &str, now: Instant) -> Verdict {
        let mut s = self.state.lock().unwrap();
        s.verdict(&source_key(ip), &user_key(user), now)
    }

    #[cfg(test)]
    fn record_failure(&self, ip: IpAddr, user: &str, now: Instant) {
        let _ = self.reserve(ip, user, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn source_is_blocked_after_ten_failures_and_recovers() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        for i in 0..SOURCE_LIMIT {
            assert_eq!(
                limiter.check(ip("192.0.2.1"), "a", start),
                Verdict::Allow,
                "{i}"
            );
            limiter.record_failure(ip("192.0.2.1"), &format!("u{i}"), start);
        }
        assert_eq!(
            limiter.check(ip("192.0.2.1"), "a", start + secs(10)),
            Verdict::Block
        );
        assert_eq!(
            limiter.check(ip("192.0.2.2"), "a", start + secs(10)),
            Verdict::Allow
        );
        // The failures are in the previous window for the next minute.
        assert_eq!(
            limiter.check(ip("192.0.2.1"), "a", start + secs(90)),
            Verdict::Block
        );
        assert_eq!(
            limiter.check(ip("192.0.2.1"), "a", start + secs(120)),
            Verdict::Allow
        );
    }

    #[test]
    fn name_failures_only_delay() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        for i in 0..USER_LIMIT {
            limiter.record_failure(ip(&format!("192.0.2.{}", i + 1)), "alice", start);
        }
        let fresh = ip("198.51.100.1");
        assert_eq!(limiter.check(fresh, "alice", start), Verdict::Delay);
        assert_eq!(limiter.check(fresh, "bob", start), Verdict::Allow);
        assert_eq!(
            limiter.check(fresh, "alice", start + USER_WINDOW * 2),
            Verdict::Allow
        );
    }

    #[test]
    fn global_failures_delay_everyone() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        for i in 0..GLOBAL_LIMIT {
            limiter.record_failure(
                IpAddr::V4((0x0a00_0000 + i).into()),
                &format!("n{i}"),
                start,
            );
        }
        assert_eq!(
            limiter.check(ip("198.51.100.1"), "x", start),
            Verdict::Delay
        );
        assert_eq!(
            limiter.check(ip("198.51.100.1"), "x", start + GLOBAL_WINDOW * 2),
            Verdict::Allow
        );
    }

    #[test]
    fn rotation_keeps_the_previous_window() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        let attacker = ip("192.0.2.1");
        // Five failures late in one window and five early in the next
        // still add up to the limit.
        for _ in 0..5 {
            limiter.record_failure(attacker, "a", start + secs(59));
        }
        for _ in 0..5 {
            limiter.record_failure(attacker, "a", start + secs(61));
        }
        assert_eq!(
            limiter.check(attacker, "a", start + secs(62)),
            Verdict::Block
        );
        // A clock reading older than the current window neither clears nor
        // rotates anything.
        assert_eq!(
            limiter.check(attacker, "a", start + secs(1)),
            Verdict::Block
        );
        assert_eq!(
            limiter.check(attacker, "a", start + secs(62)),
            Verdict::Block
        );
        // In the window after next, the first five have expired and the
        // second five are still counted.
        assert_eq!(
            limiter.check(attacker, "a", start + secs(125)),
            Verdict::Allow
        );
        for _ in 0..5 {
            limiter.record_failure(attacker, "a", start + secs(125));
        }
        assert_eq!(
            limiter.check(attacker, "a", start + secs(126)),
            Verdict::Block
        );
        // After a long idle period the counts start from zero again.
        assert_eq!(
            limiter.check(attacker, "a", start + secs(100_000)),
            Verdict::Allow
        );
    }

    #[test]
    fn counters_saturate() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        let mut s = limiter.state.lock().unwrap();
        for _ in 0..(u32::from(u16::MAX) + 10) {
            s.sources.add(&ip("192.0.2.1"), start);
        }
        assert_eq!(
            s.sources.count(&ip("192.0.2.1"), start),
            u32::from(u16::MAX)
        );
    }

    #[test]
    fn reservations_count_until_refunded() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        let me = ip("192.0.2.1");
        for _ in 0..SOURCE_LIMIT * 3 {
            let (verdict, reservation) = limiter.reserve(me, "alice", start);
            assert_eq!(verdict, Verdict::Allow);
            limiter.refund(reservation.unwrap(), start);
        }
        assert_eq!(limiter.check(me, "alice", start), Verdict::Allow);
        for _ in 0..SOURCE_LIMIT {
            let _ = limiter.reserve(me, "alice", start);
        }
        let (verdict, reservation) = limiter.reserve(me, "alice", start);
        assert_eq!(verdict, Verdict::Block);
        assert!(reservation.is_none());
    }

    #[test]
    fn refund_lands_in_the_window_it_was_reserved_in() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        let me = ip("192.0.2.1");
        let count = |at: u64| {
            limiter
                .state
                .lock()
                .unwrap()
                .sources
                .count(&source_key(me), start + secs(at))
        };
        let (_, early) = limiter.reserve(me, "a", start + secs(59));
        let (_, dropped) = limiter.reserve(me, "a", start + secs(59));
        for _ in 0..2 {
            limiter.record_failure(me, "a", start + secs(59));
        }
        for _ in 0..5 {
            limiter.record_failure(me, "a", start + secs(61));
        }
        // Window 0 holds 4, window 1 holds 5. The refund after rotation
        // must come out of window 0, not the current window.
        assert_eq!(count(61), 9);
        limiter.refund(early.unwrap(), start + secs(61));
        assert_eq!(count(61), 8);
        // In window 2 only window 1 is left, untouched by the refund.
        assert_eq!(count(125), 5);
        // Window 0 is gone now, so refunding into it changes nothing.
        limiter.refund(dropped.unwrap(), start + secs(125));
        assert_eq!(count(125), 5);
    }

    #[test]
    fn long_names_share_a_key() {
        let long = "x".repeat(MAX_NAME_CHARS);
        assert_eq!(user_key(&format!("{long}a")), user_key(&format!("{long}b")));
    }

    #[test]
    fn ipv6_counts_per_prefix() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        for i in 0..SOURCE_LIMIT {
            limiter.record_failure(ip(&format!("2001:db8::{:x}", i + 1)), "a", start);
        }
        assert_eq!(
            limiter.check(ip("2001:db8::ffff"), "b", start),
            Verdict::Block
        );
        assert_eq!(
            limiter.check(ip("2001:db8:0:1::1"), "b", start),
            Verdict::Allow
        );
        assert_eq!(source_key(ip("::ffff:192.0.2.1")), ip("192.0.2.1"));
    }

    #[test]
    fn flooding_neither_resets_nor_blocks_broadly() {
        let start = Instant::now();
        let limiter = LoginLimiter::new(start);
        for _ in 0..SOURCE_LIMIT {
            limiter.record_failure(ip("192.0.2.1"), "alice", start);
        }
        // 10 000 distinct sources and names, one failure each.
        for i in 0..10_000u32 {
            limiter.record_failure(
                IpAddr::V4((0x0a00_0000 + i).into()),
                &format!("n{i}"),
                start,
            );
        }
        assert_eq!(limiter.check(ip("192.0.2.1"), "x", start), Verdict::Block);
        // An unrelated source is only refused if all four of its counters
        // collided with about ten flood keys each, so almost none are.
        let blocked = (0..1000u32)
            .filter(|i| {
                let fresh = IpAddr::V4((0xc633_6400 + i).into());
                limiter.check(fresh, "x", start) == Verdict::Block
            })
            .count();
        assert!(blocked <= 10, "{blocked} of 1000 fresh sources blocked");
    }
}
