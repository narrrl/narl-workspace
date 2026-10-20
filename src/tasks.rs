//! Background loops that outlive a panic in one of their passes.
//!
//! A task spawned and forgotten dies silently when it panics, and the server
//! carries on without backups, or mail, with nothing in the log after the
//! panic itself. Every long-running loop goes through [`supervise`] instead,
//! which logs the panic and starts the loop again.

use std::{future::Future, time::Duration};

/// Long enough that a loop panicking at once does not spin.
const RESTART: Duration = Duration::from_secs(10);

/// Run `make()` for as long as the server lives, starting it again after a
/// panic. A loop that returns has finished on purpose and is not restarted.
pub fn supervise<F, Fut>(name: &'static str, make: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            match tokio::spawn(make()).await {
                Ok(()) => return,
                Err(e) if e.is_panic() => {
                    let payload = e.into_panic();
                    let message = payload
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "(no message)".into());
                    tracing::error!("{name} panicked: {message}; starting it again in {RESTART:?}");
                    tokio::time::sleep(RESTART).await;
                }
                // Cancelled: the runtime is shutting down.
                Err(_) => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test(start_paused = true)]
    async fn a_loop_that_panics_is_started_again() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        super::supervise("test", move || {
            let counter = Arc::clone(&counter);
            async move {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("first pass");
                }
            }
        });
        tokio::time::sleep(super::RESTART * 2).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }
}
