//! The send calls on a UDP socket of the program under the seccomp
//! backend: `sendto`, `sendmsg` and `sendmmsg`.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) async fn handle_sendto(
    lfd: impl Answer,
    id: u64,
    a: [u64; 6],
    mem: RawFd,
    (map, fd): (&mut HashMap<Key, Sock>, Key),
    dns: &mut FakeDns,
    egress: &crate::udp::Egress<'_>,
) -> Result<()> {
    let buf = a[1];
    let len = a[2] as usize;
    let name = a[4];
    let namelen = a[5] as usize;
    let recorded = match map.get(&fd) {
        Some(Sock::Datagram(udp)) => udp.inbox.dest(),
        _ => return notif_continue(lfd, id),
    };
    let target = if name != 0 && namelen >= 4 {
        parse_sockaddr(&read_mem(mem, name, namelen.min(128))?)
            .map(|(_, ip, port)| (ip, port))
            .or(recorded)
    } else {
        recorded
    };
    let Some((ip, port)) = target else {
        return notif_respond(lfd, id, -1, libc::EDESTADDRREQ);
    };
    // The length comes from the program: check it before the read.
    if len > MAX_DATAGRAM {
        return notif_respond(lfd, id, -1, libc::EMSGSIZE);
    }
    let Ok(payload) = read_mem(mem, buf, len) else {
        return notif_respond(lfd, id, -1, libc::EFAULT);
    };
    match crate::udp::send_payload(map, fd, dns, egress, &payload, (ip, port)).await {
        Ok(sent) => notif_respond(lfd, id, sent as i64, 0),
        Err(errno) => notif_respond(lfd, id, -1, errno),
    }
}

/// `sendmmsg` is what the glibc resolver uses: a vector of `mmsghdr`, for
/// example the A and the AAAA query in one call.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_sendmmsg(
    lfd: impl Answer,
    id: u64,
    a: [u64; 6],
    (mem, mem_writable): (RawFd, bool),
    (map, fd): (&mut HashMap<Key, Sock>, Key),
    dns: &mut FakeDns,
    egress: &crate::udp::Egress<'_>,
) -> Result<()> {
    // struct mmsghdr is a struct msghdr (56 bytes), a u32 msg_len and padding.
    // The layouts of the child are those of this target: same ABI.
    const MMSGHDR_SIZE: u64 = std::mem::size_of::<libc::mmsghdr>() as u64;
    const MSG_LEN_OFFSET: u64 = std::mem::offset_of!(libc::mmsghdr, msg_len) as u64;
    let msgvec = a[1];
    let vlen = (a[2] as usize).min(64);
    if !map.contains_key(&fd) {
        return notif_continue(lfd, id);
    }
    if vlen == 0 {
        return notif_respond(lfd, id, 0, 0);
    }
    // Without write access to the child, msg_len cannot be set. Send one
    // message and report 1. glibc's resolver does not send the others: it
    // reads the msg_len of 0 as a failure. The pod therefore sets
    // RES_OPTIONS=single-request (issue 113), and glibc sends one query in
    // each call.
    let count = if mem_writable { vlen } else { 1 };
    let mut sent = 0usize;
    for index in 0..count {
        let entry = msgvec + index as u64 * MMSGHDR_SIZE;
        match send_msghdr(fd, entry, mem, map, dns, egress).await? {
            None => return notif_continue(lfd, id),
            Some(Ok(bytes)) => {
                if mem_writable {
                    write_mem(mem, entry + MSG_LEN_OFFSET, &(bytes as u32).to_ne_bytes())?;
                }
                sent += 1;
            }
            Some(Err(errno)) => {
                if sent == 0 {
                    return notif_respond(lfd, id, -1, errno);
                }
                break;
            }
        }
    }
    notif_respond(lfd, id, sent as i64, 0)
}

pub(crate) async fn handle_sendmsg(
    lfd: impl Answer,
    id: u64,
    a: [u64; 6],
    mem: RawFd,
    (map, fd): (&mut HashMap<Key, Sock>, Key),
    dns: &mut FakeDns,
    egress: &crate::udp::Egress<'_>,
) -> Result<()> {
    match send_msghdr(fd, a[1], mem, map, dns, egress).await? {
        None => notif_continue(lfd, id),
        Some(Ok(bytes)) => notif_respond(lfd, id, bytes as i64, 0),
        Some(Err(errno)) => notif_respond(lfd, id, -1, errno),
    }
}

/// Send one `struct msghdr` for the child. Return the byte count, or the
/// errno to give the child. `Ok(None)` means: not a socket of ours.
pub(crate) async fn send_msghdr(
    fd: Key,
    msg_ptr: u64,
    mem: RawFd,
    map: &mut HashMap<Key, Sock>,
    dns: &mut FakeDns,
    egress: &crate::udp::Egress<'_>,
) -> Result<Option<std::result::Result<usize, i32>>> {
    let recorded = match map.get(&fd) {
        Some(Sock::Datagram(udp)) => udp.inbox.dest(),
        _ => return Ok(None),
    };
    // struct msghdr on x86_64: name, namelen, iov, iovlen, control, controllen, flags.
    // struct msghdr of this target (the child has the same ABI).
    let hdr = read_mem(mem, msg_ptr, std::mem::size_of::<libc::msghdr>())?;
    let name_ptr = read_word(&hdr, std::mem::offset_of!(libc::msghdr, msg_name));
    let name_len = u32::from_ne_bytes(
        hdr[std::mem::offset_of!(libc::msghdr, msg_namelen)..][..4]
            .try_into()
            .unwrap(),
    ) as usize;
    let iov_ptr = read_word(&hdr, std::mem::offset_of!(libc::msghdr, msg_iov));
    let iov_len = read_word(&hdr, std::mem::offset_of!(libc::msghdr, msg_iovlen)) as usize;
    let target = if name_ptr != 0 && name_len >= 4 {
        parse_sockaddr(&read_mem(mem, name_ptr, name_len.min(128))?)
            .map(|(_, ip, port)| (ip, port))
            .or(recorded)
    } else {
        recorded
    };
    let Some((ip, port)) = target else {
        return Ok(Some(Err(libc::EDESTADDRREQ)));
    };
    // The lengths come from the program: check the count of the iovecs and
    // the sum of their lengths before a read (issue 303). Before, only the
    // first 8 iovecs were read, each up to 64 KiB.
    if iov_len > crate::inbox::UIO_MAXIOV {
        return Ok(Some(Err(libc::EMSGSIZE)));
    }
    let size = std::mem::size_of::<libc::iovec>();
    let Ok(iovs) = read_mem(mem, iov_ptr, iov_len * size) else {
        return Ok(Some(Err(libc::EFAULT)));
    };
    let parts: Vec<(u64, usize)> = iovs
        .chunks_exact(size)
        .map(|iov| {
            (
                read_word(iov, std::mem::offset_of!(libc::iovec, iov_base)),
                read_word(iov, std::mem::offset_of!(libc::iovec, iov_len)) as usize,
            )
        })
        .collect();
    let total = parts
        .iter()
        .fold(0usize, |sum, (_, len)| sum.saturating_add(*len));
    if total > MAX_DATAGRAM {
        return Ok(Some(Err(libc::EMSGSIZE)));
    }
    let mut payload = Vec::with_capacity(total);
    for (base, len) in parts {
        if len == 0 {
            continue;
        }
        let Ok(part) = read_mem(mem, base, len) else {
            return Ok(Some(Err(libc::EFAULT)));
        };
        payload.extend_from_slice(&part);
    }
    let sent = crate::udp::send_payload(map, fd, dns, egress, &payload, (ip, port)).await;
    Ok(Some(sent))
}
