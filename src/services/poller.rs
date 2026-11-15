//! A background refresher shared by the feed widgets (Sports, YouTube, Reddit, Netflix, Stocks).
//!
//! Each runs on its own thread and only touches the network while its widget is on screen: an
//! off-screen widget costs nothing but a parked thread. Showing a widget again refreshes it at once
//! if its data has gone stale, and changing its configuration always refreshes it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};

/// Wake at least this often while active, to notice the PC waking from sleep.
const CHECK_EVERY: Duration = Duration::from_secs(60);

/// The longest a failing service is left alone between attempts.
pub const MAX_RETRY: Duration = Duration::from_secs(30 * 60);

/// How long to wait after `failures` failures in a row (1 = the first): `base`, doubling with each
/// further failure, up to [`MAX_RETRY`]. A service that is down or rate-limiting us (HTTP 429)
/// gets steadily more room instead of a request every few minutes for as long as it lasts; the
/// first success resets the count.
pub fn backoff(base: Duration, failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(8);
    base.saturating_mul(1 << doublings).min(MAX_RETRY.max(base))
}

/// When the next refresh is due, by both clocks. The monotonic clock is immune to clock changes
/// but may not advance while the PC sleeps; the wall clock catches up after a night's sleep, so a
/// laptop opened in the morning doesn't show last night's scores until a long timer runs out.
#[derive(Clone, Copy)]
struct Due {
    mono: Instant,
    wall: SystemTime,
}

impl Due {
    fn now() -> Self {
        Self { mono: Instant::now(), wall: SystemTime::now() }
    }

    fn after(wait: Duration) -> Self {
        Self { mono: Instant::now() + wait, wall: SystemTime::now() + wait }
    }

    fn reached(&self) -> bool {
        Instant::now() >= self.mono || SystemTime::now() >= self.wall
    }

    fn remaining(&self) -> Duration {
        self.mono.saturating_duration_since(Instant::now())
    }
}

enum Msg<C> {
    Configure(C),
    Active(bool),
}

pub struct Poller<C> {
    tx: Sender<Msg<C>>,
}

impl<C> Poller<C> {
    pub fn configure(&self, config: C) {
        let _ = self.tx.send(Msg::Configure(config));
    }

    /// On screen or not. Only active pollers fetch.
    pub fn set_active(&self, active: bool) {
        let _ = self.tx.send(Msg::Active(active));
    }
}

/// Starts a poller. `work` runs one refresh with the current configuration and returns how long to
/// wait before the next (shorter while something is live, longer after a failure, etc).
pub fn spawn<C: Send + 'static>(
    name: &str,
    config: C,
    mut work: impl FnMut(&C) -> Duration + Send + 'static,
) -> Poller<C> {
    let (tx, rx) = mpsc::channel::<Msg<C>>();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            // Image decoding goes through WinRT.
            crate::platform::init_worker_thread();
            let mut config = config;
            let mut active = false;
            let mut due = Due::now();
            loop {
                let timeout = if active { due.remaining().min(CHECK_EVERY) } else { Duration::from_secs(3600) };
                let mut handle = |msg: Msg<C>, due: &mut Due, active: &mut bool| match msg {
                    Msg::Configure(c) => {
                        config = c;
                        *due = Due::now();
                    }
                    Msg::Active(a) => *active = a,
                };
                match rx.recv_timeout(timeout) {
                    Ok(msg) => handle(msg, &mut due, &mut active),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                // Coalesce bursts (e.g. several settings edits) into one refresh.
                while let Ok(msg) = rx.try_recv() {
                    handle(msg, &mut due, &mut active);
                }
                if active && due.reached() {
                    let wait = work(&config);
                    due = Due::after(wait);
                }
            }
        })
        .expect("spawn poller thread");
    Poller { tx }
}

/// Runs `f` over `items` on at most `width` threads at once and returns the results in order. A
/// refresh's requests are independent, so their latency overlaps instead of adding up: 13 leagues
/// or 100 logos take about as long as the slowest few, not the sum of all of them.
pub fn fan_out<T: Sync, R: Send>(items: &[T], width: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    if items.len() <= 1 || width <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..width.min(items.len()) {
            s.spawn(|| {
                crate::platform::init_worker_thread(); // image decoding is WinRT
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                }
            });
        }
    });
    // Every index below `items.len()` was claimed by exactly one worker, and a worker panic would
    // already have propagated out of the scope.
    slots.into_iter().map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()).expect("filled")).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    #[test]
    fn fan_out_keeps_order_and_overlaps_work() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let running = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let items: Vec<u64> = (0..24).collect();
        let started = std::time::Instant::now();
        let out = super::fan_out(&items, 6, |&n| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            running.fetch_sub(1, Ordering::SeqCst);
            n * 2
        });
        assert_eq!(out, items.iter().map(|n| n * 2).collect::<Vec<_>>(), "in order");
        assert!(peak.load(Ordering::SeqCst) <= 6, "bounded");
        assert!(started.elapsed() < Duration::from_millis(24 * 20 / 2), "overlapped, not serial");
        assert_eq!(super::fan_out(&[] as &[u8], 4, |_| 0), Vec::<i32>::new());
        assert_eq!(super::fan_out(&[7], 4, |&n| n), [7]);
    }
    use std::time::Duration;

    #[test]
    fn failures_back_off_up_to_a_cap() {
        let three = Duration::from_secs(180);
        let waits: Vec<u64> = (1..=6).map(|n| super::backoff(three, n).as_secs()).collect();
        assert_eq!(waits, [180, 360, 720, 1440, 1800, 1800], "doubles, then holds at 30 min");
        assert_eq!(super::backoff(three, 0), three, "no failures yet: the base");
        assert_eq!(super::backoff(three, u32::MAX), super::MAX_RETRY, "never overflows");
        let long = Duration::from_secs(3600);
        assert_eq!(super::backoff(long, 3), long, "a base above the cap is kept, not shortened");
    }

    #[test]
    fn wall_clock_catches_up_after_sleep() {
        use std::time::{Instant, SystemTime};
        // As after a night's sleep: the monotonic deadline is still ahead, the wall clock's passed.
        let slept = super::Due {
            mono: Instant::now() + Duration::from_secs(600),
            wall: SystemTime::now() - Duration::from_secs(1),
        };
        assert!(slept.reached());
        assert!(!super::Due::after(Duration::from_secs(600)).reached());
        assert!(super::Due::now().reached());
    }

    #[test]
    fn only_works_while_active_and_refreshes_on_configure() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let p = super::spawn("test-poller", 1, move |c: &i32| {
            s.lock().unwrap().push(*c);
            Duration::from_secs(600)
        });
        std::thread::sleep(Duration::from_millis(60));
        assert!(seen.lock().unwrap().is_empty(), "inactive pollers stay quiet");
        p.set_active(true);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(*seen.lock().unwrap(), [1]);
        // Hiding and showing again doesn't refetch fresh data...
        p.set_active(false);
        p.set_active(true);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(*seen.lock().unwrap(), [1]);
        // ...but a new configuration does.
        p.configure(2);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(*seen.lock().unwrap(), [1, 2]);
    }
}
