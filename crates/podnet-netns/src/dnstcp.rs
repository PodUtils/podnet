//! DNS over TCP from the fake DNS of the pod (T-383).
//!
//! Where a seccomp filter of the host refuses each UDP socket of the
//! program (the lab `udp-denied` of pod-lab), the resolver of the program
//! cannot send a datagram. glibc with `RES_OPTIONS=use-vc` sends each query
//! over TCP to port 53 of its nameserver. The pod answers that connection
//! itself, with the framing of RFC 7766: a length of 2 bytes (big endian)
//! before each message.
//!
//! The backend owns the fake DNS. A task serves each connection, and gives
//! each query to the backend on a channel ([`Queries`]); the backend
//! answers it from the fake DNS ([`Query::answer`]).

#[allow(unused_imports)]
use crate::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

/// The size of the buffer of each direction of a connection.
const BUFFER: usize = 64 * 1024;

/// One query of a connection, and the channel of its answer.
pub(crate) struct Query {
    message: Vec<u8>,
    reply: oneshot::Sender<Vec<u8>>,
}

impl Query {
    /// Answer the query from `dns`. A message that is not a query gets no
    /// answer: the connection closes.
    pub(crate) fn answer(self, dns: &mut FakeDns) {
        let reply = match crate::dns::ParsedQuery::parse(&self.message) {
            Ok(query) => dns.answer(&query),
            Err(_) => Vec::new(),
        };
        let _ = self.reply.send(reply);
    }
}

/// The queries of the DNS connections of the program, for the backend.
pub(crate) struct Queries {
    tx: mpsc::UnboundedSender<Query>,
    rx: mpsc::UnboundedReceiver<Query>,
}

impl Queries {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Queries { tx, rx }
    }

    /// The next query. It waits.
    pub(crate) async fn next(&mut self) -> Option<Query> {
        self.rx.recv().await
    }

    /// The next query that waits now, else `None`.
    pub(crate) fn try_next(&mut self) -> Option<Query> {
        self.rx.try_recv().ok()
    }

    /// The asker of the connections: give it to the dials.
    pub(crate) fn asker(&self) -> Asker {
        Asker {
            tx: self.tx.clone(),
        }
    }
}

/// Makes the connections of the program to the fake DNS.
#[derive(Clone)]
pub(crate) struct Asker {
    tx: mpsc::UnboundedSender<Query>,
}

impl Asker {
    /// A new connection to the fake DNS: the end of the program. A task
    /// serves the other end. Call it inside a runtime of tokio.
    pub(crate) fn connect(&self) -> crate::upstream::Upstream {
        let (program, pod) = tokio::io::duplex(BUFFER);
        tokio::spawn(serve(pod, self.tx.clone()));
        Box::new(program)
    }

    /// Ask one query (a datagram of the program) and wait for its answer.
    /// `None`: no answer (not a query, or the backend ended).
    pub(crate) async fn ask(&self, message: Vec<u8>) -> Option<Vec<u8>> {
        let (reply, answer) = oneshot::channel();
        self.tx.send(Query { message, reply }).ok()?;
        answer.await.ok().filter(|a| !a.is_empty())
    }
}

/// Serve one connection: read each message, ask the backend, and write the
/// answer. Stop at the end of the stream, at an error, or at a message
/// that gets no answer.
async fn serve<S>(mut stream: S, tx: mpsc::UnboundedSender<Query>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let mut length = [0u8; 2];
        if stream.read_exact(&mut length).await.is_err() {
            break;
        }
        let mut message = vec![0u8; usize::from(u16::from_be_bytes(length))];
        if stream.read_exact(&mut message).await.is_err() {
            break;
        }
        let (reply, answer) = oneshot::channel();
        if tx.send(Query { message, reply }).is_err() {
            break;
        }
        let Ok(answer) = answer.await else { break };
        let Ok(length) = u16::try_from(answer.len()) else {
            break;
        };
        if answer.is_empty() {
            break;
        }
        let mut framed = Vec::with_capacity(2 + answer.len());
        framed.extend_from_slice(&length.to_be_bytes());
        framed.extend_from_slice(&answer);
        if stream.write_all(&framed).await.is_err() {
            break;
        }
    }
    let _ = stream.shutdown().await;
}

/// Whether this process cannot make an `AF_INET` datagram socket: the
/// socket is refused with `EPERM`, `EACCES` or `ENOSYS`. The program
/// gets the same filter as this process, so its resolver cannot send a
/// datagram either. Gives the reason.
pub(crate) fn udp_socket_refused() -> Option<String> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd >= 0 {
        unsafe { libc::close(fd) };
        return None;
    }
    let e = std::io::Error::last_os_error();
    matches!(
        e.raw_os_error(),
        Some(libc::EPERM | libc::EACCES | libc::ENOSYS)
    )
    .then(|| format!("a UDP socket is refused: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-383: two framed queries on one connection get two framed answers
    /// from the fake DNS, and a message that is not a query closes it.
    #[test]
    fn framed_queries_get_framed_answers() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut queries = Queries::new();
            let mut stream = queries.asker().connect();
            let backend = tokio::spawn(async move {
                let mut dns = FakeDns::new();
                while let Some(query) = queries.next().await {
                    query.answer(&mut dns);
                }
            });
            for (id, name) in [(7u16, "one.example"), (8, "two.example")] {
                let query = crate::dns::build_query(id, name, crate::dns::TYPE_A).unwrap();
                let mut framed = (query.len() as u16).to_be_bytes().to_vec();
                framed.extend_from_slice(&query);
                stream.write_all(&framed).await.unwrap();
                let mut length = [0u8; 2];
                stream.read_exact(&mut length).await.unwrap();
                let mut answer = vec![0u8; usize::from(u16::from_be_bytes(length))];
                stream.read_exact(&mut answer).await.unwrap();
                assert_eq!(u16::from_be_bytes([answer[0], answer[1]]), id);
                let ips = crate::dns::parse_answers(&answer).unwrap();
                assert_eq!(ips.len(), 1, "{name}");
                assert!(matches!(ips[0], IpAddr::V4(v4) if v4.octets()[..2] == [198, 18]));
            }
            stream.write_all(&[0, 3, 1, 2, 3]).await.unwrap();
            let mut rest = Vec::new();
            stream.read_to_end(&mut rest).await.unwrap();
            assert!(rest.is_empty(), "{rest:?}");
            drop(stream);
            backend.abort();
        });
    }
}
