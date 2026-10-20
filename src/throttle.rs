use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Failed logins get progressively slower to answer.
///
/// The workspace has exactly one account, so this is one counter rather than a map
/// keyed by address: an attacker cannot step around it by changing source IP,
/// and there is no per-client table to grow. Someone hammering it slows down
/// every browser it covers, so the login keeps one for browsers it does not
/// know and one apiece for those it does; see [`LoginThrottles`].
#[derive(Debug, Default)]
pub struct LoginThrottle {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    failures: u32,
    next_allowed: i64,
    /// An attempt has been let through and not yet settled. Checking and
    /// recording are otherwise two steps, and a burst of guesses sent at once
    /// would all pass the check before the first failure was written down.
    in_flight: bool,
}

/// Wrong passwords happen; the first few are answered at full speed.
pub const FREE_ATTEMPTS: u32 = 3;
const MAX_DELAY: i64 = 30;

/// 1s, 2s, 4s, 8s… up to `MAX_DELAY`, starting once the free attempts are used.
fn delay_after(failures: u32) -> i64 {
    match failures.checked_sub(FREE_ATTEMPTS) {
        None | Some(0) => 0,
        Some(over) => 1i64
            .checked_shl(over - 1)
            .unwrap_or(MAX_DELAY)
            .min(MAX_DELAY),
    }
}

impl LoginThrottle {
    /// Let one attempt through, or `Err(seconds)` to wait first. Attempts go
    /// one at a time: a second one while the first is still being decided is
    /// told to come back in a second, so guesses cannot be made in parallel —
    /// and neither can the Argon2 runs behind a vault password, each of which
    /// takes 64 MiB.
    pub fn begin(&self) -> Result<Attempt<'_>, i64> {
        let mut state = self.lock();
        if state.in_flight {
            return Err(1);
        }
        match state.next_allowed - crate::now() {
            wait if wait > 0 => Err(wait),
            _ => {
                state.in_flight = true;
                Ok(Attempt { throttle: self })
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// How many browsers that have logged in before get a throttle of their own.
/// Past that, the one used longest ago is forgotten, and falls back to
/// sharing the anonymous one until it next logs in.
const KNOWN_DEVICES: usize = 64;

/// The login's throttles. A browser that has logged in before carries a
/// signed device cookie and is slowed only by its own mistakes; every other
/// request shares the anonymous throttle. A stranger guessing at the
/// password therefore slows strangers, never a browser the owner already
/// uses — a new one still waits in line with the guesses.
#[derive(Debug, Default)]
pub struct LoginThrottles {
    anonymous: Arc<LoginThrottle>,
    known: Mutex<HashMap<String, (Arc<LoginThrottle>, i64)>>,
}

impl LoginThrottles {
    /// The throttle for a login from `device`, a device id whose signature
    /// has already been checked, or from a browser without one.
    pub fn for_device(&self, device: Option<&str>) -> Arc<LoginThrottle> {
        let Some(device) = device else {
            return Arc::clone(&self.anonymous);
        };
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        let now = crate::now();
        if let Some((throttle, used)) = known.get_mut(device) {
            *used = now;
            return Arc::clone(throttle);
        }
        if known.len() >= KNOWN_DEVICES {
            let oldest = known
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                known.remove(&oldest);
            }
        }
        let throttle = Arc::<LoginThrottle>::default();
        known.insert(device.to_string(), (Arc::clone(&throttle), now));
        throttle
    }
}

/// One attempt that [`LoginThrottle::begin`] let through. Dropping it without
/// a verdict — a request that failed for some other reason — counts for
/// nothing either way.
pub struct Attempt<'a> {
    throttle: &'a LoginThrottle,
}

impl Attempt<'_> {
    /// Returns how many failures have piled up, so the caller can decide
    /// whether this one is worth telling anybody about.
    pub fn failed(self) -> u32 {
        let mut state = self.throttle.lock();
        state.failures = state.failures.saturating_add(1);
        state.next_allowed = crate::now() + delay_after(state.failures);
        state.failures
    }

    pub fn succeeded(self) {
        let mut state = self.throttle.lock();
        state.failures = 0;
        state.next_allowed = 0;
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        self.throttle.lock().in_flight = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{delay_after, Attempt, LoginThrottle, LoginThrottles, KNOWN_DEVICES, MAX_DELAY};

    #[test]
    fn the_first_few_mistakes_are_free() {
        assert_eq!(delay_after(0), 0);
        assert_eq!(delay_after(3), 0);
    }

    #[test]
    fn then_the_wait_doubles_up_to_the_cap() {
        assert_eq!(delay_after(4), 1);
        assert_eq!(delay_after(5), 2);
        assert_eq!(delay_after(6), 4);
        assert_eq!(delay_after(9), 30);
        // Never overflows, however long the attack runs.
        assert_eq!(delay_after(u32::MAX), MAX_DELAY);
    }

    #[test]
    fn a_correct_password_clears_the_debt() {
        let throttle = LoginThrottle::default();
        // Made directly: past the free attempts `begin` would make them wait.
        for _ in 0..10 {
            Attempt {
                throttle: &throttle,
            }
            .failed();
        }
        assert!(throttle.begin().is_err());
        Attempt {
            throttle: &throttle,
        }
        .succeeded();
        assert!(throttle.begin().is_ok());
    }

    #[test]
    fn attempts_go_one_at_a_time() {
        let throttle = LoginThrottle::default();
        let first = throttle.begin().unwrap();
        assert_eq!(throttle.begin().err(), Some(1));
        drop(first);
        assert!(throttle.begin().is_ok());
    }

    #[test]
    fn guesses_from_strangers_do_not_slow_a_known_browser() {
        let throttles = LoginThrottles::default();
        let anonymous = throttles.for_device(None);
        for _ in 0..10 {
            Attempt {
                throttle: &anonymous,
            }
            .failed();
        }
        assert!(throttles.for_device(None).begin().is_err());
        assert!(throttles.for_device(Some("laptop")).begin().is_ok());

        // And a known browser's own mistakes stay its own.
        let laptop = throttles.for_device(Some("laptop"));
        for _ in 0..10 {
            Attempt { throttle: &laptop }.failed();
        }
        assert!(throttles.for_device(Some("laptop")).begin().is_err());
        assert!(throttles.for_device(Some("phone")).begin().is_ok());
    }

    #[test]
    fn known_browsers_are_capped() {
        let throttles = LoginThrottles::default();
        for n in 0..KNOWN_DEVICES * 3 {
            throttles.for_device(Some(&format!("device-{n}")));
        }
        assert_eq!(throttles.known.lock().unwrap().len(), KNOWN_DEVICES);
    }
}
