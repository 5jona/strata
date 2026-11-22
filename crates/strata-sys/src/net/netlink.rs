//! Pure-Rust zero-polling `NETLINK_ROUTE` socket wrapper and `nlmsghdr` event parser.

use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use crate::error::{SysError, SysResult};

/// Size of the standard Linux `nlmsghdr` header in bytes.
const NLMSG_HDRLEN: usize = 16;

// Linux UAPI `<linux/rtnetlink.h>` multicast group masks (not exported by `libc` on Android).
const RTMGRP_LINK: u32 = 0x1;
const RTMGRP_IPV4_IFADDR: u32 = 0x10;
const RTMGRP_IPV4_ROUTE: u32 = 0x40;
const RTMGRP_IPV6_IFADDR: u32 = 0x100;
const RTMGRP_IPV6_ROUTE: u32 = 0x400;

/// Netlink multicast groups watched for link, IPv4/IPv6 address, and route transitions.
const ROUTE_NOTIFY_GROUPS: u32 =
    RTMGRP_LINK | RTMGRP_IPV4_IFADDR | RTMGRP_IPV6_IFADDR | RTMGRP_IPV4_ROUTE | RTMGRP_IPV6_ROUTE;

/// High-level network topology change decoded from kernel `NETLINK_ROUTE` notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetChangeEvent {
    /// Network interface link state changed (`RTM_NEWLINK` / `RTM_DELLINK`).
    LinkChanged,
    /// IPv4 or IPv6 interface address changed (`RTM_NEWADDR` / `RTM_DELADDR`).
    AddressChanged,
    /// IPv4 or IPv6 routing table changed (`RTM_NEWROUTE` / `RTM_DELROUTE`).
    RouteChanged,
}

/// Result of draining a non-blocking `NETLINK_ROUTE` socket.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct NetlinkDrain {
    /// Decoded link, address, and route events received during this drain pass.
    pub events: Vec<NetChangeEvent>,
    /// `true` once `recv` observed `EAGAIN`/`EWOULDBLOCK` (caller MUST clear `AsyncFd` readiness).
    /// `false` if the per-pass batch cap was hit before the queue emptied (caller should yield and
    /// resume draining without clearing readiness).
    pub exhausted: bool,
}

/// Maximum `recv` calls per drain pass to prevent a packet burst from starving the async runtime.
const MAX_DRAIN_CALLS: usize = 512;

/// Opens a non-blocking, close-on-exec `AF_NETLINK` (`NETLINK_ROUTE`) socket subscribed to
/// link, address, and route multicast notifications.
///
/// The returned `OwnedFd` can be wrapped directly in Tokio's `AsyncFd` for zero-polling wakeups.
pub fn open_route_netlink() -> SysResult<OwnedFd> {
    // SAFETY: `socket` with `AF_NETLINK` / `NETLINK_ROUTE` takes only integer constants
    // and returns a fresh file descriptor or -1 on error.
    let raw = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    };
    if raw < 0 {
        return Err(SysError::last_os("socket(AF_NETLINK, NETLINK_ROUTE)"));
    }
    // SAFETY: `raw` is a newly created valid descriptor owned exclusively by `fd`.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };

    // SAFETY: `sockaddr_nl` is a C POD struct whose zero representation is valid.
    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    addr.nl_pid = 0;
    addr.nl_groups = ROUTE_NOTIFY_GROUPS;

    // SAFETY: `fd` is an open netlink socket and `addr` is a properly initialized `sockaddr_nl`
    // whose exact byte length is passed to `bind`.
    let rc = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&raw const addr).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        return Err(SysError::last_os("bind(AF_NETLINK, NETLINK_ROUTE)"));
    }

    Ok(fd)
}

/// Drains currently queued `NETLINK_ROUTE` datagrams from a non-blocking netlink socket.
///
/// Sets `exhausted = true` once the kernel queue yields `EAGAIN`/`EWOULDBLOCK` so `AsyncFd`
/// callers know to call `clear_ready()`. If the kernel reports `ENOBUFS` due to a burst overflow,
/// records `NetChangeEvent::RouteChanged` and continues draining remaining queued messages.
pub fn drain_netlink_events(fd: BorrowedFd<'_>) -> io::Result<NetlinkDrain> {
    let mut buf = [0_u8; 8192];
    let mut events = Vec::new();
    for _ in 0..MAX_DRAIN_CALLS {
        // SAFETY: `fd` is a valid non-blocking netlink socket and `buf` is writable stack storage.
        let n = unsafe {
            libc::recv(
                fd.as_raw_fd(),
                buf.as_mut_ptr().cast::<libc::c_void>(),
                buf.len(),
                0,
            )
        };
        if n > 0 {
            let received = n as usize;
            events.extend(parse_netlink_events(&buf[..received]));
            continue;
        }
        if n == 0 {
            return Ok(NetlinkDrain {
                events,
                exhausted: true,
            });
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code == libc::EINTR => continue,
            Some(code) if code == libc::EAGAIN || code == libc::EWOULDBLOCK => {
                return Ok(NetlinkDrain {
                    events,
                    exhausted: true,
                });
            }
            Some(code) if code == libc::ENOBUFS => {
                events.push(NetChangeEvent::RouteChanged);
                continue;
            }
            _ => return Err(err),
        }
    }
    Ok(NetlinkDrain {
        events,
        exhausted: false,
    })
}

/// Parses one or more 4-byte aligned `nlmsghdr` frames from a raw `NETLINK_ROUTE` buffer.
///
/// Truncated, undersized, or unknown frames are safely skipped/terminated without panicking.
#[must_use]
pub fn parse_netlink_events(buf: &[u8]) -> Vec<NetChangeEvent> {
    let mut events = Vec::new();
    let mut offset = 0usize;

    while buf.len().saturating_sub(offset) >= NLMSG_HDRLEN {
        let hdr = &buf[offset..offset + NLMSG_HDRLEN];
        let nlmsg_len = u32::from_ne_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let nlmsg_type = u16::from_ne_bytes([hdr[4], hdr[5]]);

        if nlmsg_len < NLMSG_HDRLEN || nlmsg_len > buf.len().saturating_sub(offset) {
            break;
        }

        if let Some(evt) = classify_nlmsg_type(nlmsg_type) {
            events.push(evt);
        }

        let Some(aligned) = align4(nlmsg_len) else {
            break;
        };
        let Some(next_offset) = offset.checked_add(aligned) else {
            break;
        };
        offset = next_offset;
    }

    events
}

fn classify_nlmsg_type(nlmsg_type: u16) -> Option<NetChangeEvent> {
    match nlmsg_type {
        libc::RTM_NEWLINK | libc::RTM_DELLINK => Some(NetChangeEvent::LinkChanged),
        libc::RTM_NEWADDR | libc::RTM_DELADDR => Some(NetChangeEvent::AddressChanged),
        libc::RTM_NEWROUTE | libc::RTM_DELROUTE => Some(NetChangeEvent::RouteChanged),
        _ => None,
    }
}

fn align4(len: usize) -> Option<usize> {
    len.checked_add(3).map(|n| n & !3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_nlmsg(out: &mut Vec<u8>, msg_type: u16, payload_len: usize) {
        let total_len = (NLMSG_HDRLEN + payload_len) as u32;
        out.extend_from_slice(&total_len.to_ne_bytes());
        out.extend_from_slice(&msg_type.to_ne_bytes());
        out.extend_from_slice(&0_u16.to_ne_bytes()); // flags
        out.extend_from_slice(&1_u32.to_ne_bytes()); // seq
        out.extend_from_slice(&0_u32.to_ne_bytes()); // pid
        out.resize(out.len() + payload_len, 0xAA);
        let pad = (4 - (out.len() % 4)) % 4;
        out.resize(out.len() + pad, 0);
    }

    #[test]
    fn parses_multi_message_buffer_with_alignment_padding() {
        let mut buf = Vec::new();
        // Odd payload lengths (1, 3, 5) exercise 4-byte NLMSG_ALIGN padding between frames.
        push_nlmsg(&mut buf, libc::RTM_NEWLINK, 1);
        push_nlmsg(&mut buf, libc::RTM_DELADDR, 3);
        push_nlmsg(&mut buf, libc::RTM_NEWROUTE, 5);
        push_nlmsg(&mut buf, libc::RTM_DELLINK, 0);
        push_nlmsg(&mut buf, libc::RTM_NEWADDR, 2);
        push_nlmsg(&mut buf, libc::RTM_DELROUTE, 4);

        assert_eq!(
            parse_netlink_events(&buf),
            vec![
                NetChangeEvent::LinkChanged,
                NetChangeEvent::AddressChanged,
                NetChangeEvent::RouteChanged,
                NetChangeEvent::LinkChanged,
                NetChangeEvent::AddressChanged,
                NetChangeEvent::RouteChanged,
            ]
        );
    }

    #[test]
    fn ignores_unknown_message_types_without_stopping_parse() {
        let mut buf = Vec::new();
        push_nlmsg(&mut buf, libc::NLMSG_NOOP as u16, 4);
        push_nlmsg(&mut buf, libc::RTM_NEWADDR, 2);
        push_nlmsg(&mut buf, 9999, 7);
        push_nlmsg(&mut buf, libc::RTM_NEWROUTE, 0);

        assert_eq!(
            parse_netlink_events(&buf),
            vec![NetChangeEvent::AddressChanged, NetChangeEvent::RouteChanged]
        );
    }

    #[test]
    fn handles_truncated_and_corrupt_frames_safely() {
        // Empty and sub-header buffers.
        assert!(parse_netlink_events(&[]).is_empty());
        assert!(parse_netlink_events(&[0_u8; 15]).is_empty());

        // Valid frame followed by truncated second frame (claims 32 bytes, only 20 remain).
        let mut buf = Vec::new();
        push_nlmsg(&mut buf, libc::RTM_NEWLINK, 4);
        buf.extend_from_slice(&32_u32.to_ne_bytes());
        buf.extend_from_slice(&libc::RTM_NEWROUTE.to_ne_bytes());
        buf.resize(buf.len() + 14, 0);
        assert_eq!(
            parse_netlink_events(&buf),
            vec![NetChangeEvent::LinkChanged]
        );

        // Zero-length header must not infinite-loop.
        let zero_len = [0_u8; 32];
        assert!(parse_netlink_events(&zero_len).is_empty());

        // Undersized header (< 16 bytes) must terminate cleanly.
        let mut undersized = [0_u8; 20];
        undersized[0..4].copy_from_slice(&8_u32.to_ne_bytes());
        assert!(parse_netlink_events(&undersized).is_empty());
    }
}
