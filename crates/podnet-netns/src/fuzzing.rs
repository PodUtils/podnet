//! Entry points for the fuzz targets of the seccomp handlers (`fuzz/` of
//! the workspace calls them through `podnet::fuzzing`). Each function takes
//! untrusted bytes: a call of the program and the memory of the program.
//! It makes a fresh state for each input, runs one handler, and checks the
//! oracle: the handler does not panic, ends in [`LIMIT`], and gives one
//! answer to the notification. A handler that gives no answer must return
//! an error: the supervisor then answers.
//!
//! The input has three parts:
//!
//! - byte 0: the flags. Bits 0 and 1 select the socket of the call (none,
//!   UDP, TCP with no connection, connected TCP). Bit 2 makes the memory
//!   writable. Bit 3 connects the UDP socket to `10.66.255.254:53`. Bit 4
//!   makes the socket `AF_INET6`. Bits 5 to 7 select the call.
//! - bytes 1 to 24: the six arguments of the call, each a `u32` (little
//!   endian). A value with bit 31 set is `u64::MAX` minus its low 31 bits.
//! - the rest: the memory of the program, at address 0.

use crate::fdtable::Key;
use crate::prelude::*;

/// The longest time of one handler.
pub const LIMIT: Duration = Duration::from_secs(5);

/// The key of the socket of the call.
const KEY: Key = 7;

/// The id of the notification.
const ID: u64 = 0x1d;

/// One answer: the id, and the return value and the errno (`None`: the
/// kernel runs the call).
type Reply = (u64, Option<(i64, i32)>);

/// The answers of a handler, in memory.
#[derive(Default)]
struct Answers(std::cell::RefCell<Vec<Reply>>);

impl Answer for &Answers {
    fn respond(self, id: u64, val: i64, error: i32) -> Result<()> {
        self.0.borrow_mut().push((id, Some((val, error))));
        Ok(())
    }

    fn proceed(self, id: u64) -> Result<()> {
        self.0.borrow_mut().push((id, None));
        Ok(())
    }
}

/// One input, decoded.
struct Input {
    flags: u8,
    args: [u64; 6],
    memory: Vec<u8>,
}

impl Input {
    fn parse(data: &[u8]) -> Self {
        let mut head = [0u8; 25];
        let n = data.len().min(head.len());
        head[..n].copy_from_slice(&data[..n]);
        let mut args = [0u64; 6];
        for (i, arg) in args.iter_mut().enumerate() {
            let at = 1 + 4 * i;
            let v = u32::from_le_bytes(head[at..at + 4].try_into().expect("4 bytes"));
            *arg = match v & 0x8000_0000 {
                0 => u64::from(v),
                _ => u64::MAX - u64::from(v & 0x7fff_ffff),
            };
        }
        Input {
            flags: head[0],
            args,
            memory: data[n..].to_vec(),
        }
    }

    fn writable(&self) -> bool {
        self.flags & 0x04 != 0
    }

    fn v6(&self) -> bool {
        self.flags & 0x10 != 0
    }

    /// The call variant (bits 5 to 7).
    fn call(&self) -> u8 {
        self.flags >> 5
    }
}

/// The memory of the program: a sealed memfd (it cannot grow) with the
/// bytes at offset 0. A read or a write out of it fails, as at an address
/// that is not mapped.
fn memory_of(bytes: &[u8]) -> OwnedFd {
    let name = c"podnet-fuzz-mem";
    // SAFETY: a valid name; the flags are constants.
    let fd =
        unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    assert!(fd >= 0, "memfd_create: {}", std::io::Error::last_os_error());
    // SAFETY: a new descriptor that nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes).expect("write the memory image");
    let fd = OwnedFd::from(file);
    // SAFETY: fcntl on an open descriptor.
    unsafe {
        libc::fcntl(
            fd.as_raw_fd(),
            libc::F_ADD_SEALS,
            libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
        )
    };
    fd
}

/// The state of one input: the socket table, the fake DNS and an upstream
/// that refuses each dial.
struct State {
    map: HashMap<Key, Sock>,
    dns: FakeDns,
    proxies: Vec<Proxy>,
    /// The end of the program of the socketpair; the replies wait in it.
    _program: Option<OwnedFd>,
    _writes: tokio::sync::mpsc::Receiver<crate::udp::Written>,
}

/// Make the state of `input`. Call it in the runtime: a UDP socket starts
/// a task.
fn state_of(input: &Input) -> State {
    let mut map = HashMap::new();
    let (writes_tx, writes_rx) = tokio::sync::mpsc::channel(crate::udp::WRITES);
    let v6 = input.v6();
    let mut program = None;
    match input.flags & 0x03 {
        1 => {
            let (ours, theirs) = std::os::unix::net::UnixDatagram::pair().expect("socketpair");
            ours.set_nonblocking(true).expect("nonblocking");
            let inbox = crate::inbox::Inbox::new(theirs.try_clone().expect("dup"), v6);
            if input.flags & 0x08 != 0 {
                inbox.set_dest(Some((IpAddr::V4(TUN_GW), DNS_PORT)));
            }
            let pair = tokio::net::UnixDatagram::from_std(ours).expect("register");
            let udp = crate::udp::Udp::new(Arc::new(pair), Arc::new(inbox), (KEY, &writes_tx));
            map.insert(KEY, Sock::Datagram(udp));
            program = Some(OwnedFd::from(theirs));
        }
        2 => {
            let (ours, theirs) = std::os::unix::net::UnixStream::pair().expect("socketpair");
            ours.set_nonblocking(true).expect("nonblocking");
            let pair = tokio::net::UnixStream::from_std(ours).expect("register");
            map.insert(
                KEY,
                Sock::Stream {
                    pair,
                    bound: None,
                    v6,
                    inbound: None,
                    error: libc::ECONNREFUSED,
                    options: crate::sockopt::Options::new(),
                },
            );
            program = Some(OwnedFd::from(theirs));
        }
        3 => {
            let ip = match v6 {
                true => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
                false => IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)),
            };
            let options = crate::sockopt::Options::new();
            let connected = crate::sockopt::Connected::new(5, (ip, 443), None, options);
            map.insert(KEY, Sock::Connected(connected));
        }
        _ => {}
    }
    let mut dns = FakeDns::new();
    dns.v6 = true;
    let proxy = Proxy::parse("socks5://unix:/nonexistent/podnet-fuzz.sock").expect("proxy");
    State {
        map,
        dns,
        proxies: vec![proxy],
        _program: program,
        _writes: writes_rx,
    }
}

/// Run `handler` on a fresh state of `data`, and check the oracle. Return
/// the answers.
fn run<F>(data: &[u8], handler: F) -> Vec<Reply>
where
    F: for<'a> AsyncFnOnce(&'a Input, &'a Answers, RawFd, &'a mut State) -> Result<()>,
{
    let input = Input::parse(data);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let mem = memory_of(&input.memory);
    let answers = Answers::default();
    let result = runtime.block_on(async {
        let mut state = state_of(&input);
        let call = handler(&input, &answers, mem.as_raw_fd(), &mut state);
        tokio::time::timeout(LIMIT, call).await
    });
    let Ok(result) = result else {
        panic!("the handler gave no answer in {LIMIT:?}");
    };
    let answers = answers.0.into_inner();
    assert!(
        answers.iter().all(|(id, _)| *id == ID),
        "an answer to another id: {answers:?}"
    );
    match result {
        Ok(()) => assert_eq!(
            answers.len(),
            1,
            "a handler that returns Ok answers one time: {answers:?}"
        ),
        Err(_) => assert!(answers.len() <= 1, "more than one answer: {answers:?}"),
    }
    answers
}

/// `sendto` on the socket of the input.
fn sendto(data: &[u8]) -> Vec<Reply> {
    run(data, async |input, answers, mem, state| {
        let State {
            map, dns, proxies, ..
        } = state;
        let egress = crate::udp::Egress::of(proxies);
        crate::seccomp::handle_sendto(answers, ID, input.args, mem, (map, KEY), dns, &egress).await
    })
}

/// `sendmsg` (an even call variant) or `sendmmsg` (an odd call variant) on
/// the socket of the input.
fn sendmsg(data: &[u8]) -> Vec<Reply> {
    run(data, async |input, answers, mem, state| {
        let State {
            map, dns, proxies, ..
        } = state;
        let entry = (map, KEY);
        let egress = crate::udp::Egress::of(proxies);
        let egress = &egress;
        match input.call() % 2 {
            0 => {
                crate::seccomp::handle_sendmsg(answers, ID, input.args, mem, entry, dns, egress)
                    .await
            }
            _ => {
                let mem = (mem, input.writable());
                crate::seccomp::handle_sendmmsg(answers, ID, input.args, mem, entry, dns, egress)
                    .await
            }
        }
    })
}

/// `setsockopt`, `getsockopt`, `bind`, `getsockname` or `getpeername` (the
/// call variant) on the socket of the input.
fn sockopt(data: &[u8]) -> Vec<Reply> {
    run(data, async |input, answers, mem, state| {
        let (a, writable) = (input.args, input.writable());
        let entry = (&mut state.map, KEY);
        match input.call() % 5 {
            0 => crate::sockopt::handle_setsockopt(answers, ID, a, mem, entry, false),
            1 => crate::sockopt::handle_getsockopt(answers, ID, a, mem, writable, entry),
            2 => crate::sockopt::handle_bind(answers, ID, a, mem, entry),
            peer => {
                crate::sockopt::handle_sockname(answers, ID, a, mem, writable, entry, peer == 4)
            }
        }
    })
}

/// The target `seccomp_sendto`: `sendto` on the input.
pub fn seccomp_sendto(data: &[u8]) {
    sendto(data);
}

/// The target `seccomp_sendmsg`: `sendmsg` or `sendmmsg` on the input.
pub fn seccomp_sendmsg(data: &[u8]) {
    sendmsg(data);
}

/// The target `seccomp_sockopt`: a call of the socket options and the
/// addresses on the input.
pub fn seccomp_sockopt(data: &[u8]) {
    sockopt(data);
}

/// The arguments and the memory of one seed: the flags, six arguments, the
/// memory.
fn seed(flags: u8, args: [u32; 6], memory: &[u8]) -> Vec<u8> {
    let mut out = vec![flags];
    for a in args {
        out.extend_from_slice(&a.to_le_bytes());
    }
    out.extend_from_slice(memory);
    out
}

/// A DNS query for `example.com` (type A).
fn query() -> Vec<u8> {
    crate::dns::build_query(0x4242, "example.com", 1).expect("a valid name")
}

/// The memory of each seed: 64 zero bytes, then a `sockaddr_in` of
/// `10.66.255.254:53` (offset 64) and of `198.18.0.1:9` (offset 80).
fn base() -> Vec<u8> {
    let mut out = vec![0u8; 64];
    let to = |ip, port| crate::seccomp::sockaddr_in(libc::AF_INET as u16, IpAddr::V4(ip), port);
    out.extend(to(TUN_GW, 53));
    out.extend(to(Ipv4Addr::new(198, 18, 0, 1), 9));
    out
}

/// Write the word `value` (a pointer or a `size_t`) at `at`.
fn put_word(out: &mut [u8], at: usize, value: usize) {
    const WORD: usize = std::mem::size_of::<usize>();
    out[at..at + WORD].copy_from_slice(&value.to_ne_bytes());
}

/// A `struct mmsghdr` with the name at `name` (16 bytes) and one `iovec`
/// at `iov`.
fn mmsghdr(name: usize, iov: usize) -> Vec<u8> {
    let mut out = vec![0u8; std::mem::size_of::<libc::mmsghdr>()];
    put_word(&mut out, std::mem::offset_of!(libc::msghdr, msg_name), name);
    out[std::mem::offset_of!(libc::msghdr, msg_namelen)..][..4]
        .copy_from_slice(&16u32.to_ne_bytes());
    put_word(&mut out, std::mem::offset_of!(libc::msghdr, msg_iov), iov);
    put_word(&mut out, std::mem::offset_of!(libc::msghdr, msg_iovlen), 1);
    out
}

/// An `iovec` of `len` bytes at `base`.
fn iovec(base: usize, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; std::mem::size_of::<libc::iovec>()];
    put_word(&mut out, std::mem::offset_of!(libc::iovec, iov_base), base);
    put_word(&mut out, std::mem::offset_of!(libc::iovec, iov_len), len);
    out
}

/// The seed inputs of the target `name`.
pub fn seeds(name: &str) -> Vec<Vec<u8>> {
    let q = query();
    let len = q.len() as u32;
    match name {
        "seccomp_sendto" => {
            let mut mem = base();
            mem.extend_from_slice(&q);
            vec![
                // A query to the resolver, by address.
                seed(0x01, [3, 96, len, 0, 64, 16], &mem),
                // On a connected socket, with no address.
                seed(0x09, [3, 96, len, 0, 0, 0], &mem),
                // A datagram to another port: the upstream refuses.
                seed(0x01, [3, 96, 4, 0, 80, 16], &mem),
            ]
        }
        "seccomp_sendmsg" => {
            // Two iovecs at 96, two mmsghdr at 128, the query after them.
            let size = std::mem::size_of::<libc::mmsghdr>();
            let data = 128 + 2 * size;
            let mut mem = base();
            mem.extend(iovec(data, q.len()));
            mem.extend(iovec(data, 4));
            mem.resize(128, 0);
            mem.extend(mmsghdr(64, 96));
            mem.extend(mmsghdr(80, 96 + std::mem::size_of::<libc::iovec>()));
            mem.extend_from_slice(&q);
            vec![
                seed(0x01, [3, 128, 0, 0, 0, 0], &mem),
                seed(0x25, [3, 128, 2, 0, 0, 0], &mem),
            ]
        }
        "seccomp_sockopt" => {
            let mut mem = base();
            mem[16..20].copy_from_slice(&1i32.to_ne_bytes());
            mem[20..24].copy_from_slice(&4u32.to_ne_bytes());
            mem[24..28].copy_from_slice(&28u32.to_ne_bytes());
            vec![
                // setsockopt(SOL_SOCKET, SO_KEEPALIVE, 1).
                seed(0x02, [3, 1, 9, 16, 4, 0], &mem),
                // getsockopt(SOL_SOCKET, SO_ERROR).
                seed(0x26, [3, 1, 4, 32, 20, 0], &mem),
                // bind to 10.66.255.254:53.
                seed(0x41, [3, 64, 16, 0, 0, 0], &mem),
                // getsockname and getpeername.
                seed(0x67, [3, 32, 24, 0, 0, 0], &mem),
                seed(0x8d, [3, 32, 24, 0, 0, 0], &mem),
            ]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer of each seed: the seeds reach the paths of the handlers
    /// that send and answer, not only the refusals.
    #[test]
    fn each_seed_gets_its_answer() {
        let len = query().len() as i64;
        let sent = |n| vec![(ID, Some((n, 0)))];
        let failed = |errno| vec![(ID, Some((-1, errno)))];
        let s = seeds("seccomp_sendto");
        assert_eq!(sendto(&s[0]), sent(len));
        assert_eq!(sendto(&s[1]), sent(len));
        assert_eq!(sendto(&s[2]), failed(libc::ENETUNREACH));
        let s = seeds("seccomp_sendmsg");
        assert_eq!(sendmsg(&s[0]), sent(len));
        // The first message is sent, the second fails: the count is 1.
        assert_eq!(sendmsg(&s[1]), sent(1));
        let s = seeds("seccomp_sockopt");
        let answers: Vec<Vec<Reply>> = s.iter().map(|seed| sockopt(seed)).collect();
        assert_eq!(answers, vec![sent(0); 5]);
    }

    /// More queries in one `sendmmsg` than `net.unix.max_dgram_qlen` (10 by
    /// default): the limit does not apply to a socketpair, and each query
    /// gets its reply.
    #[test]
    fn a_burst_of_queries_in_one_sendmmsg_gets_an_answer() {
        const COUNT: usize = 32;
        let q = query();
        let size = std::mem::size_of::<libc::mmsghdr>();
        let data = 128 + COUNT * size;
        let mut mem = base();
        mem.extend(iovec(data, q.len()));
        mem.resize(128, 0);
        for _ in 0..COUNT {
            mem.extend(mmsghdr(64, 96));
        }
        mem.extend_from_slice(&q);
        let input = seed(0x25, [3, 128, COUNT as u32, 0, 0, 0], &mem);
        assert_eq!(sendmsg(&input), vec![(ID, Some((COUNT as i64, 0)))]);
    }

    #[test]
    fn an_empty_input_gets_an_answer() {
        assert_eq!(sendto(&[]), vec![(ID, None)]);
        assert_eq!(sendmsg(&[]), vec![(ID, None)]);
        assert_eq!(sockopt(&[]), vec![(ID, None)]);
    }
}
