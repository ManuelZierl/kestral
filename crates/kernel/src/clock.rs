//! Clock seam.
//!
//! Kernel services never read wall-clock time directly; they receive a
//! `Clock`. This keeps grant expiry, lease expiry, and ledger timestamps
//! deterministic under test and makes time an explicit dependency rather
//! than a hidden one.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Test clock that only moves when told to. Share it via `Arc` so the test
/// keeps a handle to the same instant the kernel reads.
pub struct FixedClock {
    now: Mutex<DateTime<Utc>>,
}

impl FixedClock {
    pub fn new(start: DateTime<Utc>) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(start),
        })
    }

    pub fn advance_to(&self, moment: DateTime<Utc>) {
        let mut now = self.now.lock().expect("clock lock poisoned");
        assert!(moment >= *now, "FixedClock cannot move backwards");
        *now = moment;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("clock lock poisoned")
    }
}

impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> DateTime<Utc> {
        (**self).now()
    }
}
