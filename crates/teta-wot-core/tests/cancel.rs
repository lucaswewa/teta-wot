//! Integration tests for resettable cancellation and cancellable waits.

use std::future::{Future, poll_fn};
use std::sync::{Arc, Barrier, mpsc};
use std::task::Poll;
use std::time::{Duration, Instant};

use teta_wot_core::{CancelToken, Cancelled};

async fn assert_pending<F: Future>(mut future: std::pin::Pin<&mut F>) {
    assert!(
        poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx)))
            .await
            .is_pending()
    );
}

#[test]
fn new_and_default_tokens_are_independent_and_uncancelled() {
    let first = CancelToken::new();
    let second = CancelToken::default();
    assert!(!first.is_cancelled());
    assert!(!second.is_cancelled());
    assert_eq!(first.check(), Ok(()));
    first.cancel();
    assert!(first.is_cancelled());
    assert!(!second.is_cancelled());
    assert_eq!(second.check(), Ok(()));
}

#[test]
fn clones_share_cancellation_and_checks_consume_it_once() {
    let token = CancelToken::new();
    let clone = token.clone();
    token.cancel();
    assert!(clone.is_cancelled());
    assert!(clone.is_cancelled()); // Inspecting the flag does not consume it.
    assert_eq!(clone.check(), Err(Cancelled));
    assert!(!token.is_cancelled());
    assert_eq!(token.check(), Ok(()));
    clone.cancel();
    assert_eq!(token.check(), Err(Cancelled));
    assert_eq!(clone.check(), Ok(()));
}

#[test]
fn repeated_cancellation_requests_coalesce_until_consumed() {
    let token = CancelToken::new();
    token.cancel();
    token.cancel();
    assert_eq!(token.check(), Err(Cancelled));
    assert_eq!(token.check(), Ok(()));
    token.cancel();
    assert_eq!(token.check(), Err(Cancelled));
}

#[test]
fn concurrent_checks_have_exactly_one_consumer() {
    let token = CancelToken::new();
    token.cancel();
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let token = token.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                token.check().is_err()
            })
        })
        .collect();
    let consumers = threads
        .into_iter()
        .filter_map(|thread| thread.join().unwrap().then_some(()))
        .count();
    assert_eq!(consumers, 1);
    assert!(!token.is_cancelled());
}

#[tokio::test]
async fn cancelled_consumes_pending_requests_and_can_wait_again() {
    let token = CancelToken::new();
    token.cancel();
    assert_eq!(token.cancelled().await, Cancelled);
    assert!(!token.is_cancelled());
    let waiting = token.cancelled();
    tokio::pin!(waiting);
    assert_pending(waiting.as_mut()).await;
    token.cancel();
    assert_eq!(waiting.await, Cancelled);
    assert_eq!(token.check(), Ok(()));
}

#[tokio::test]
async fn competing_async_waiters_consume_separate_requests() {
    let token = CancelToken::new();
    let first = token.cancelled();
    let second = token.cancelled();
    tokio::pin!(first, second);
    assert_pending(first.as_mut()).await;
    assert_pending(second.as_mut()).await;
    token.cancel();
    assert_eq!(first.await, Cancelled);
    // Both waiters were notified, but only the first consumed the flag.
    assert_pending(second.as_mut()).await;
    token.cancel();
    assert_eq!(second.await, Cancelled);
    assert!(!token.is_cancelled());
}

#[tokio::test]
async fn dropping_a_notified_waiter_does_not_consume_cancellation() {
    let token = CancelToken::new();
    let mut waiting = Box::pin(token.cancelled());
    assert_pending(waiting.as_mut()).await;
    token.cancel();
    drop(waiting);
    assert!(token.is_cancelled());
    assert_eq!(token.cancelled().await, Cancelled);
    assert!(!token.is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn async_sleep_completes_after_the_duration_without_cancellation() {
    let token = CancelToken::new();
    let sleep = token.sleep(Duration::from_secs(10));
    tokio::pin!(sleep);
    assert_pending(sleep.as_mut()).await;
    tokio::time::advance(Duration::from_secs(9)).await;
    assert_pending(sleep.as_mut()).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(sleep.await, Ok(()));
    assert!(!token.is_cancelled());
    token.cancel();
    assert_eq!(token.check(), Err(Cancelled));
}

#[tokio::test(start_paused = true)]
async fn async_sleep_is_interrupted_and_token_can_be_reused() {
    let token = CancelToken::new();
    let sleep = token.sleep(Duration::from_secs(3600));
    tokio::pin!(sleep);
    assert_pending(sleep.as_mut()).await;
    token.clone().cancel();
    assert_eq!(sleep.await, Err(Cancelled));
    assert!(!token.is_cancelled());
    assert_eq!(token.sleep(Duration::ZERO).await, Ok(()));
}

#[tokio::test(start_paused = true)]
async fn pending_cancellation_wins_over_zero_duration_async_sleep() {
    let token = CancelToken::new();
    token.cancel();
    assert_eq!(token.sleep(Duration::ZERO).await, Err(Cancelled));
    assert_eq!(token.sleep(Duration::ZERO).await, Ok(()));
}

#[tokio::test]
async fn cancellation_from_another_thread_wakes_an_async_waiter() {
    let token = CancelToken::new();
    let waiting = token.cancelled();
    tokio::pin!(waiting);
    assert_pending(waiting.as_mut()).await;
    let other = token.clone();
    let thread = std::thread::spawn(move || other.cancel());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap(),
        Cancelled
    );
    thread.join().unwrap();
    assert!(!token.is_cancelled());
}

#[test]
fn blocking_sleep_times_out_normally_and_handles_pending_cancellation() {
    let token = CancelToken::new();
    let duration = Duration::from_millis(5);
    let started = Instant::now();
    assert_eq!(token.sleep_blocking(duration), Ok(()));
    assert!(started.elapsed() >= duration);
    assert!(!token.is_cancelled());
    token.cancel();
    assert_eq!(token.sleep_blocking(Duration::ZERO), Err(Cancelled));
    assert_eq!(token.sleep_blocking(Duration::ZERO), Ok(()));
}

#[test]
fn cancellation_from_another_thread_interrupts_blocking_sleep() {
    let token = CancelToken::new();
    let other = token.clone();
    let (started, ready) = mpsc::channel();
    let (finished, result) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        started.send(()).unwrap();
        let outcome = other.sleep_blocking(Duration::from_secs(30));
        finished.send(outcome).unwrap();
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
    // Cancellation works both before entering the wait and while waiting.
    token.cancel();
    assert_eq!(
        result.recv_timeout(Duration::from_secs(2)).unwrap(),
        Err(Cancelled)
    );
    thread.join().unwrap();
    assert!(!token.is_cancelled());
    assert_eq!(token.check(), Ok(()));
}

#[test]
fn cancelled_error_has_the_public_message() {
    assert_eq!(Cancelled.to_string(), "The action was cancelled.");
}
