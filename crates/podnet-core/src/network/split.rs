//! The split of the directions of a flow, an option of a group (T-179). It
//! is off by default: one flow uses one member (T-096).
//!
//! What podnet can split:
//!
//! * UDP. The datagrams of a socket leave by the member `up` and are
//!   received on the member `down` ([`SplitDatagrams`]). It works when
//!   both members end at one far host, for example two tunnels to one
//!   endpoint, and the far host answers on the path of `down`. Datagrams
//!   that arrive on `up` are delivered too, so that no answer is lost.
//!
//! What podnet cannot split:
//!
//! * TCP. One byte stream has both directions. A split needs two
//!   connections that a far end joins, and podnet has no such far end. The
//!   option refuses it ([`TCP_REFUSAL`]).

use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};

use super::{Datagrams, RecvMeta, Transmit};

/// The text of the refusal of a TCP split.
pub const TCP_REFUSAL: &str =
    "a split of TCP needs a far end that joins two connections, and podnet has none; \
     the flows of TCP keep one member (T-179)";

/// The members of a split, as indexes of the group: `up` sends, `down`
/// receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub up: usize,
    pub down: usize,
}

/// The datagrams of a split socket: a send goes to `up`, a receive reads
/// `down` first and `up` second.
#[derive(Debug)]
pub struct SplitDatagrams {
    up: Arc<dyn Datagrams>,
    down: Arc<dyn Datagrams>,
}

impl SplitDatagrams {
    pub fn new(up: Arc<dyn Datagrams>, down: Arc<dyn Datagrams>) -> SplitDatagrams {
        SplitDatagrams { up, down }
    }
}

impl Datagrams for SplitDatagrams {
    fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.up.poll_writable(cx)
    }

    fn try_send(&self, transmit: &Transmit<'_>) -> io::Result<()> {
        self.up.try_send(transmit)
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        if let Poll::Ready(r) = self.down.poll_recv(cx, bufs, meta) {
            return Poll::Ready(r);
        }
        self.up.poll_recv(cx, bufs, meta)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.up.local_addr()
    }

    fn max_transmit_segments(&self) -> usize {
        self.up.max_transmit_segments()
    }

    fn max_receive_segments(&self) -> usize {
        self.up
            .max_receive_segments()
            .min(self.down.max_receive_segments())
    }

    fn may_fragment(&self) -> bool {
        self.up.may_fragment()
    }
}

/// Read the value of a `splits` entry: `off`, or `up=MEMBER down=MEMBER`
/// (words split by a space or a comma). `members` are the names of the
/// group. It returns the names of `up` and `down`, or `None` for `off`.
pub fn parse(value: &str, members: &[String]) -> Result<Option<(String, String)>, String> {
    let words: Vec<&str> = value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    if words == ["off"] {
        return Ok(None);
    }
    let (mut up, mut down) = (None, None);
    for word in &words {
        match word.split_once('=') {
            Some(("up", m)) if up.is_none() => up = Some(m),
            Some(("down", m)) if down.is_none() => down = Some(m),
            _ if word.starts_with("tcp") => return Err(TCP_REFUSAL.to_string()),
            _ => return Err(format!("{word:?}: give off, or up=MEMBER down=MEMBER")),
        }
    }
    let (Some(up), Some(down)) = (up, down) else {
        return Err("give off, or up=MEMBER down=MEMBER".to_string());
    };
    for m in [up, down] {
        if !members.iter().any(|n| n == m) {
            return Err(format!("{m:?} is not a member of the group"));
        }
    }
    if up == down {
        return Err("up and down name one member; a split needs two".to_string());
    }
    Ok(Some((up.to_string(), down.to_string())))
}

#[cfg(test)]
mod tests;
