//! The `hf` carrier of PodUtils/podnet#499 against a modem simulator.
//!
//! The simulator answers the ASCII command set of `pod-offline`: `OK` to
//! `SPEED`, `BUSY` to the first `TX` and `OK` to the second. The test
//! counts the retransmissions at both layers: the modem writes the frame
//! twice, and the ARQ of the session writes it once (it defers). It also
//! holds a transmission while the modem receives, and checks that the
//! carrier says that the link is half duplex.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use pod_link::Carrier as _;
use podnet::offline::hf::{session, Activity, DeferArq, Hf, SPEED_MOST, TURNAROUND};

enum Phase {
    Line,
    Data,
}

/// A modem that answers the commands of the driver, and counts the frames
/// that came.
struct Sim {
    to_host: VecDeque<u8>,
    from_host: Vec<u8>,
    replies: VecDeque<&'static str>,
    phase: Phase,
    line: Vec<u8>,
    data: Vec<u8>,
    left: usize,
    tx: u32,
    frames: Vec<Vec<u8>>,
}

impl Sim {
    fn new(replies: &[&'static str]) -> Sim {
        Sim {
            to_host: VecDeque::new(),
            from_host: Vec::new(),
            replies: replies.iter().copied().collect(),
            phase: Phase::Line,
            line: Vec::new(),
            data: Vec::new(),
            left: 0,
            tx: 0,
            frames: Vec::new(),
        }
    }

    fn say(&mut self, text: &str) {
        self.to_host.extend(text.bytes());
    }

    fn command(&mut self, text: &str) {
        if let Some(rest) = text.strip_prefix("TX ") {
            let count: usize = rest.parse().expect("a count");
            self.tx += 1;
            self.phase = Phase::Data;
            self.left = count;
            self.data.clear();
        } else if text.starts_with("SPEED ") {
            let reply = self.replies.pop_front().unwrap_or("OK");
            self.say(&format!("{reply}\r"));
        }
    }

    fn data_end(&mut self) {
        self.frames.push(self.data.clone());
        let reply = self.replies.pop_front().unwrap_or("OK");
        self.say(&format!("{reply}\r"));
        self.phase = Phase::Line;
    }
}

impl Read for Sim {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.to_host.pop_front() {
            Some(byte) => {
                buf[0] = byte;
                Ok(1)
            }
            None => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the simulator has no answer",
            )),
        }
    }
}

impl Write for Sim {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for &byte in buf {
            self.from_host.push(byte);
            match self.phase {
                Phase::Line => {
                    if byte == b'\r' {
                        let text = String::from_utf8_lossy(&self.line).into_owned();
                        self.line.clear();
                        self.command(&text);
                    } else {
                        self.line.push(byte);
                    }
                }
                Phase::Data => {
                    self.data.push(byte);
                    self.left -= 1;
                    if self.left == 0 {
                        self.data_end();
                    }
                }
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_busy_modem_stops_the_session_and_only_the_modem_retries() {
    // SPEED, the first TX, the second TX.
    let mut session = session(Sim::new(&["OK", "BUSY", "OK"]), 1200).expect("a session");
    assert!(session.arq().deferring_arq());
    assert_eq!(session.arq().window(), 1);
    assert_eq!(session.policy(), DeferArq::DEGRADED);
    assert_eq!(session.policy().session_retransmits(), 0);

    let now = Instant::now();
    let err = session.send_at(now, b"hello").expect_err("a busy channel");
    assert_eq!(err.name(), "busy");
    assert_eq!(session.modem().writes(), 1, "the first attempt went out");

    // The modem receives, and no byte goes to the line.
    let before = session.modem().link().from_host.len();
    let err = session
        .send_at(now + Duration::from_secs(30), b"hello")
        .expect_err("a receiving modem");
    assert_eq!(err.name(), "receiving");
    assert_eq!(
        session.modem().link().from_host.len(),
        before,
        "a byte went to the line while the modem received"
    );
    assert_eq!(session.modem().link().tx, 1);

    // The channel comes back, and the same frame goes out once more. The
    // session ARQ sends nothing of its own.
    session.modem_mut().link_mut().say("IDLE\r");
    assert!(session
        .receive_at(now + TURNAROUND)
        .expect("the idle word")
        .is_none());
    assert_eq!(session.modem().activity(), Activity::Idle);
    let attempts = session
        .send_at(now + TURNAROUND, b"hello")
        .expect("the second attempt");
    assert_eq!(attempts, 2, "one retransmission of the frame");
    assert_eq!(session.modem().writes(), 2);
    assert_eq!(session.modem().retransmissions(), 1);
    assert_eq!(session.arq().retransmissions(), 0);
    assert_eq!(
        session.modem().retransmissions() + u32::from(session.policy().session_retransmits()),
        1,
        "one retransmission chain, not two"
    );
    assert!(
        session
            .arq_mut()
            .poll_at(now + Duration::from_secs(3600))
            .expect("a poll")
            .is_none(),
        "the ARQ of the session sent the frame again"
    );

    let frames = &session.modem().link().frames;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0], frames[1], "the two attempts differ");
}

#[test]
fn a_reception_holds_every_transmission() {
    let mut session = session(Sim::new(&["OK"]), 1200).expect("a session");
    let now = Instant::now();
    session.modem_mut().note_receive_at(now);
    assert_eq!(session.modem().activity(), Activity::Receiving);
    let before = session.modem().link().from_host.len();
    let err = session
        .send_at(now + Duration::from_millis(1), b"frame")
        .expect_err("a receiving modem");
    assert_eq!(err.name(), "receiving");
    assert_eq!(session.modem().link().from_host.len(), before);
    assert_eq!(session.modem().link().tx, 0, "a frame went out");
}

#[test]
fn the_speed_is_three_hundred_to_three_thousand_and_the_link_is_half_duplex() {
    assert_eq!(
        session(Sim::new(&[]), 299).err().expect("a speed").name(),
        "speed"
    );
    assert_eq!(
        session(Sim::new(&[]), 3_001).err().expect("a speed").name(),
        "speed"
    );
    let facts = Hf::with_spec(format!("hf:/dev/ttyUSB0,{SPEED_MOST}")).facts();
    assert!(!facts.duplex, "the carrier says that it is full duplex");
    assert_eq!(facts.rate.bytes_per_second, Some(u64::from(SPEED_MOST / 8)));
    let slow = Hf::with_spec("hf:/dev/ttyUSB0,1200".to_string()).facts();
    assert_eq!(slow.rate.bytes_per_second, Some(150));
}
