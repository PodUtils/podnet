//! The nfnetlink client of the netfilter plane (T-152, T-153): the
//! constants, the builder of a netlink message, the exchange with the
//! kernel and the reader of attributes. `nfprobe` (the measure) and
//! `nfrules` (the rules of the `netns-kernel` backend) use it.

pub(crate) const NETLINK_NETFILTER: i32 = 12;
pub(crate) const NFNL_SUBSYS_CTNETLINK: u16 = 1;
pub(crate) const NFNL_SUBSYS_NFTABLES: u16 = 10;
pub(crate) const NFNL_MSG_BATCH_BEGIN: u16 = 0x10;
pub(crate) const NFNL_MSG_BATCH_END: u16 = 0x11;
pub(crate) const NFT_MSG_NEWTABLE: u16 = 0;
pub(crate) const NFT_MSG_NEWCHAIN: u16 = 3;
pub(crate) const NFT_MSG_NEWRULE: u16 = 6;
pub(crate) const IPCTNL_MSG_CT_GET: u16 = 1;
pub(crate) const NLM_F_REQUEST: u16 = 0x1;
pub(crate) const NLM_F_ACK: u16 = 0x4;
pub(crate) const NLM_F_DUMP: u16 = 0x300;
pub(crate) const NLM_F_CREATE: u16 = 0x400;
pub(crate) const NLM_F_APPEND: u16 = 0x800;
pub(crate) const NLMSG_ERROR: u16 = 2;
pub(crate) const NLMSG_DONE: u16 = 3;
pub(crate) const NLA_F_NESTED: u16 = 0x8000;
pub(crate) const NFPROTO_IPV4: u8 = 2;
pub(crate) const NF_INET_LOCAL_OUT: u32 = 3;
pub(crate) const NFT_REG_1: u32 = 1;
pub(crate) const NFT_META_L4PROTO: u32 = 16;
pub(crate) const NFT_PAYLOAD_TRANSPORT_HEADER: u32 = 2;
pub(crate) const SO_ORIGINAL_DST: i32 = 80;

/// The netlink messages of a probe, built before the fork.
pub(crate) struct Builder {
    pub(crate) buf: Vec<u8>,
    start: usize,
    nests: Vec<usize>,
}

impl Builder {
    pub(crate) fn new() -> Self {
        Builder {
            buf: Vec::new(),
            start: 0,
            nests: Vec::new(),
        }
    }

    /// Start a message: the header and the `nfgenmsg`.
    pub(crate) fn msg(&mut self, ty: u16, flags: u16, seq: u32, family: u8, res_id: u16) {
        self.start = self.buf.len();
        self.buf.extend_from_slice(&0u32.to_ne_bytes());
        self.buf.extend_from_slice(&ty.to_ne_bytes());
        self.buf.extend_from_slice(&flags.to_ne_bytes());
        self.buf.extend_from_slice(&seq.to_ne_bytes());
        self.buf.extend_from_slice(&0u32.to_ne_bytes());
        self.buf.extend_from_slice(&[family, 0]);
        self.buf.extend_from_slice(&res_id.to_be_bytes());
    }

    pub(crate) fn end_msg(&mut self) {
        let len = (self.buf.len() - self.start) as u32;
        self.buf[self.start..self.start + 4].copy_from_slice(&len.to_ne_bytes());
    }

    pub(crate) fn attr(&mut self, ty: u16, data: &[u8]) {
        self.buf
            .extend_from_slice(&((4 + data.len()) as u16).to_ne_bytes());
        self.buf.extend_from_slice(&ty.to_ne_bytes());
        self.buf.extend_from_slice(data);
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
    }

    pub(crate) fn text(&mut self, ty: u16, text: &str) {
        let mut data = text.as_bytes().to_vec();
        data.push(0);
        self.attr(ty, &data);
    }

    pub(crate) fn be32(&mut self, ty: u16, value: u32) {
        self.attr(ty, &value.to_be_bytes());
    }

    pub(crate) fn nest(&mut self, ty: u16) {
        self.nests.push(self.buf.len());
        self.buf.extend_from_slice(&0u16.to_ne_bytes());
        self.buf
            .extend_from_slice(&(ty | NLA_F_NESTED).to_ne_bytes());
    }

    pub(crate) fn end_nest(&mut self) {
        if let Some(at) = self.nests.pop() {
            let len = (self.buf.len() - at) as u16;
            self.buf[at..at + 2].copy_from_slice(&len.to_ne_bytes());
        }
    }

    /// One expression of a rule: `NFTA_LIST_ELEM { NAME, DATA { body } }`.
    pub(crate) fn expr(&mut self, name: &str, body: impl FnOnce(&mut Self)) {
        self.nest(1);
        self.text(1, name);
        self.nest(2);
        body(self);
        self.end_nest();
        self.end_nest();
    }

    /// `NFTA_DATA_VALUE` in a nest of the type `ty`.
    pub(crate) fn value(&mut self, ty: u16, value: &[u8]) {
        self.nest(ty);
        self.attr(1, value);
        self.end_nest();
    }
}

pub(crate) fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Send `msg` on `fd`, then read the answers. With `acks`, wait for the
/// acknowledgement of each sequence number: return the sequence number and
/// the errno of the first error. Without `acks`, read a dump to its end.
/// `entry` gets the attributes of each other message (the body after the
/// `nfgenmsg`).
pub(crate) fn exchange(
    fd: i32,
    msg: &[u8],
    acks: &[u32],
    entry: &mut dyn FnMut(&[u8]),
) -> Result<(), (u32, i32)> {
    // SAFETY: send of a buffer that lives.
    if unsafe { libc::send(fd, msg.as_ptr().cast(), msg.len(), 0) } < 0 {
        return Err((0, errno()));
    }
    let mut acked = 0usize;
    let mut buf = [0u8; 16384];
    loop {
        // SAFETY: recv into a buffer that lives.
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n <= 0 {
            return Err((0, if n < 0 { errno() } else { libc::EPROTO }));
        }
        let mut at = 0usize;
        let n = n as usize;
        while at + 16 <= n {
            let len = u32::from_ne_bytes(buf[at..at + 4].try_into().unwrap_or([0; 4])) as usize;
            let ty = u16::from_ne_bytes([buf[at + 4], buf[at + 5]]);
            let seq = u32::from_ne_bytes(buf[at + 8..at + 12].try_into().unwrap_or([0; 4]));
            if len < 16 || at + len > n {
                break;
            }
            let body = &buf[at + 16..at + len];
            match ty {
                NLMSG_ERROR => {
                    let error = i32::from_ne_bytes(body[..4].try_into().unwrap_or([0; 4]));
                    if error != 0 {
                        return Err((seq, -error));
                    }
                    if acks.contains(&seq) {
                        acked += 1;
                        if acked == acks.len() {
                            return Ok(());
                        }
                    }
                }
                NLMSG_DONE => return Ok(()),
                _ if body.len() > 4 => entry(&body[4..]),
                _ => {}
            }
            at += (len + 3) & !3;
        }
    }
}

/// A netfilter netlink socket with a read time limit of `seconds`.
pub(crate) fn open_socket(seconds: i64) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    // SAFETY: socket gives a new descriptor that nothing else owns.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            NETLINK_NETFILTER,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is open and owned here.
    let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    // `time_t` is 32 bits on some targets (i686, ARMv7 and ARMv6 musl).
    #[allow(
        clippy::useless_conversion,
        reason = "the conversion is needed where time_t has 32 bits"
    )]
    let limit = libc::timeval {
        tv_sec: seconds
            .clamp(0, i64::from(i32::MAX))
            .try_into()
            .unwrap_or_default(),
        tv_usec: 0,
    };
    // SAFETY: setsockopt with a timeval that lives.
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&limit as *const libc::timeval).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }
    Ok(owned)
}

/// The attributes of `data`: (type without the flags, payload).
pub(crate) fn attrs(data: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        if at + 4 > data.len() {
            return None;
        }
        let len = u16::from_ne_bytes([data[at], data[at + 1]]) as usize;
        let ty = u16::from_ne_bytes([data[at + 2], data[at + 3]]) & 0x3fff;
        if len < 4 || at + len > data.len() {
            return None;
        }
        let payload = &data[at + 4..at + len];
        at += (len + 3) & !3;
        Some((ty, payload))
    })
}
