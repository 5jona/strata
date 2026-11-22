//! Zero-polling `NETLINK_ROUTE` watcher that debounces network transitions and re-syncs DNS.

use std::future::Future;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::time::Duration;

use strata_sys::net::{drain_netlink_events, open_route_netlink};
use tokio::io::unix::AsyncFd;
use tokio::sync::watch;

use crate::manager::ContainerManager;

/// Coalesces rapid link/address/route bursts (Wi-Fi/5G/VPN handoffs) before syncing DNS.
const DNS_SYNC_DEBOUNCE: Duration = Duration::from_millis(800);

/// Spawns the background `NETLINK_ROUTE` `AsyncFd` watcher task.
///
/// Exits gracefully with a diagnostic log if `AF_NETLINK` is unavailable in restricted
/// test/sandbox environments.
pub(crate) fn spawn_net_watcher(
    manager: Arc<ContainerManager>,
    shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_net_watcher(manager, shutdown))
}

async fn run_net_watcher(manager: Arc<ContainerManager>, shutdown: watch::Receiver<bool>) {
    let fd = match open_route_netlink() {
        Ok(fd) => fd,
        Err(err) => {
            eprintln!("stratad: netlink route watcher unavailable ({err}); auto DNS sync disabled");
            return;
        }
    };

    run_net_watcher_loop(fd, DNS_SYNC_DEBOUNCE, shutdown, move || {
        let mgr = Arc::clone(&manager);
        async move {
            mgr.sync_dns_all().await;
        }
    })
    .await;
}

async fn run_net_watcher_loop<F, Fut>(
    fd: OwnedFd,
    debounce_window: Duration,
    mut shutdown: watch::Receiver<bool>,
    mut on_change: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    // SAFETY: `fd` is an exclusively owned, non-blocking `OwnedFd` that remains valid
    // until `AsyncFd` is dropped.
    let async_fd = match unsafe { AsyncFd::register(fd) } {
        Ok(afd) => afd,
        Err(err) => {
            eprintln!(
                "stratad: failed to register netlink fd with epoll ({err}); auto DNS sync disabled"
            );
            return;
        }
    };

    let mut pending_sync = false;
    let debounce = tokio::time::sleep(debounce_window);
    tokio::pin!(debounce);

    loop {
        if *shutdown.borrow() {
            break;
        }

        tokio::select! {
            _ = wait_shutdown(&mut shutdown) => {
                break;
            }
            guard_res = async_fd.readable() => {
                let mut guard = match guard_res {
                    Ok(g) => g,
                    Err(err) => {
                        eprintln!("stratad: netlink route watcher read error ({err}); stopping");
                        break;
                    }
                };
                match drain_netlink_events(guard.get_inner().as_fd()) {
                    Ok(drained) => {
                        if drained.exhausted {
                            // `drain_netlink_events` consumed `EAGAIN`/`EWOULDBLOCK`; clear Tokio's
                            // edge-triggered readiness so `readable()` parks cleanly on epoll.
                            guard.clear_ready();
                        } else {
                            // Queue hit the per-pass cap; retain readiness and yield so other
                            // tasks and the epoll driver are never starved.
                            drop(guard);
                            tokio::task::yield_now().await;
                        }
                        if !drained.events.is_empty() {
                            pending_sync = true;
                            debounce
                                .as_mut()
                                .reset(tokio::time::Instant::now() + debounce_window);
                        }
                    }
                    Err(err) => {
                        eprintln!("stratad: netlink route recv failed ({err}); stopping");
                        break;
                    }
                }
            }
            () = &mut debounce, if pending_sync => {
                pending_sync = false;
                on_change().await;
            }
        }
    }
}

async fn wait_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixDatagram;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn nlmsg_frame(msg_type: u16) -> [u8; 16] {
        let mut frame = [0_u8; 16];
        frame[0..4].copy_from_slice(&16_u32.to_ne_bytes());
        frame[4..6].copy_from_slice(&msg_type.to_ne_bytes());
        frame
    }

    #[tokio::test(flavor = "current_thread")]
    async fn clears_readiness_when_idle_and_debounces_bursts() {
        let (rx, tx) = UnixDatagram::pair().expect("create unix datagram pair");
        rx.set_nonblocking(true).expect("set nonblocking");
        let fd: OwnedFd = rx.into();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let sync_count = Arc::new(AtomicUsize::new(0));
        let sync_counter = Arc::clone(&sync_count);

        let handle = tokio::spawn(run_net_watcher_loop(
            fd,
            Duration::from_millis(40),
            shutdown_rx,
            move || {
                let c = Arc::clone(&sync_counter);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            },
        ));

        // On a single-threaded runtime, a readiness leak would spin forever and prevent this
        // timer from firing or `shutdown_tx` from completing promptly.
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(sync_count.load(Ordering::SeqCst), 0);

        // Send two rapid netlink-formatted datagrams; debounce should coalesce them into 1 sync.
        let frame = nlmsg_frame(libc::RTM_NEWADDR);
        tx.send(&frame).expect("send first frame");
        tx.send(&frame).expect("send second frame");

        tokio::time::sleep(Duration::from_millis(90)).await;
        assert_eq!(sync_count.load(Ordering::SeqCst), 1);

        let _ = shutdown_tx.send(true);
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("watcher must stop promptly on shutdown")
            .expect("watcher task should not panic");
    }
}
