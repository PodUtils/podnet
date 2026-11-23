//! The `hf` carrier: an ARDOP or VARA modem on a serial line.
//!
//! The address is `hf:PATH[,BPS]`, and `--carrier hf:PATH[,BPS]` is the
//! same address in the carrier flag. The modem is a serial device with an
//! ASCII command set, as `pod-offline` drives it; the device is opened at
//! 115200 baud (the line to the modem, not the radio) and the radio speed
//! is `BPS`, 300 to 3000 bit/s, 1200 by default.
//!
//! The rules of this carrier are the rules of the driver in
//! [`pod_offline::hf`]:
//!
//! * The modem has its own ARQ. The session ARQ defers to it: a frame goes
//!   once, and the modem sends it again. Two retransmission chains do not
//!   stack.
//! * The link is half duplex. A `BUSY` word or a reception stops a
//!   transmission, and a reception is followed by a turnaround
//!   ([`TURNAROUND`]) before the next one.
//! * The degraded-link profile is mandatory: [`DeferArq::DEGRADED`], one
//!   frame in the window and every retransmission left to the modem.
//!
//! The driver blocks on the line while it waits for the answer of the
//! modem. The bridge keeps that block out of the async runtime.
//!
//! It cannot do: more than 3000 bit/s, a full-duplex session, a modem that
//! gives no `OK` after `TX`, or a transmission during a reception.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use pod_link::{
    BoxIo, Carrier, CarrierError, CarrierErrorKind, Cost, Facts, Listener, Pending, Rate,
};
use pod_offline::serial::arq::{self, Received, Receiver, MAX_PAYLOAD};
use pod_offline::serial::SerialSpec;

use super::bridge::{self, Last};

/// The name of the carrier.
pub const NAME: &str = "hf";

/// The speed of the line to the modem, not the speed of the radio.
pub const DEVICE_BAUD: u32 = 115_200;

pub use pod_offline::hf::{
    Activity, DeferArq, Modem, Session, DEFAULT_SPEED, SPEED_LEAST, SPEED_MOST, TURNAROUND,
};

/// The device and the radio speed of `hf:PATH[,BPS]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfSpec {
    path: String,
    bps: u32,
}

impl HfSpec {
    /// Read the address. A path is never empty and holds no comma and no
    /// white space; the speed is 300 to 3000 bit/s.
    pub fn parse(address: &str) -> Result<HfSpec, pod_offline::Error> {
        let rest = address
            .strip_prefix("hf:")
            .ok_or_else(|| pod_offline::Error::Address {
                why: format!("{address}: an address starts with hf:"),
            })?;
        let (path, bps) = match rest.rsplit_once(',') {
            Some((path, text)) => {
                if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(pod_offline::Error::Address {
                        why: format!("{address}: the speed is not a number"),
                    });
                }
                let bps: u32 = text.parse().map_err(|_| pod_offline::Error::Address {
                    why: format!("{address}: the speed is not a number"),
                })?;
                (path, bps)
            }
            None => (rest, DEFAULT_SPEED),
        };
        if path.is_empty() {
            return Err(pod_offline::Error::Address {
                why: format!("{address}: no path"),
            });
        }
        if path.contains(',') || path.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(pod_offline::Error::Address {
                why: format!("{address}: the path holds a comma, a space or a control byte"),
            });
        }
        if !(SPEED_LEAST..=SPEED_MOST).contains(&bps) {
            return Err(pod_offline::Error::Speed { bps });
        }
        Ok(HfSpec {
            path: path.to_string(),
            bps,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn bps(&self) -> u32 {
        self.bps
    }

    /// The address, as a person writes it.
    pub fn address(&self) -> String {
        format!("{NAME}:{},{}", self.path, self.bps)
    }

    /// The serial address of the line to the modem.
    fn line(&self) -> String {
        format!(
            "{}:{},{}",
            pod_offline::serial::PREFIX,
            self.path,
            DEVICE_BAUD
        )
    }
}

/// A session that defers each retransmission to the modem, with the
/// degraded profile. The policy is not a parameter: it is the rule of this
/// carrier.
pub fn session<L: Read + Write>(link: L, bps: u32) -> Result<Session<L>, pod_offline::Error> {
    let mut session = Session::open(link, bps, DeferArq::DEGRADED)?;
    session.modem_mut().set_speed(bps)?;
    Ok(session)
}

/// The carrier `hf`.
pub struct Hf {
    spec: Option<String>,
    last: Last,
}

impl Default for Hf {
    fn default() -> Hf {
        Hf {
            spec: None,
            last: bridge::last(),
        }
    }
}

impl Hf {
    /// A carrier that takes its address from the flag `--carrier hf:PATH[,BPS]`.
    pub fn with_spec(spec: String) -> Hf {
        Hf {
            spec: Some(spec),
            ..Hf::default()
        }
    }

    pub fn failure(&self) -> Option<String> {
        bridge::failure(&self.last)
    }
}

fn spec_of<'a>(carrier: &'a Hf, address: &'a str) -> Result<HfSpec, CarrierError> {
    let text = carrier.spec.as_deref().unwrap_or(address);
    HfSpec::parse(text).map_err(|e| {
        CarrierError::new(
            NAME,
            CarrierErrorKind::Usage,
            format!("{text}: {} ({})", e, e.name()),
        )
    })
}

fn refusal(e: pod_offline::Error, spec: &HfSpec) -> CarrierError {
    CarrierError::new(
        NAME,
        CarrierErrorKind::Unavailable,
        format!("{}: {} ({})", spec.path(), e, e.name()),
    )
}

fn start(spec: &HfSpec, last: Last) -> Result<tokio::net::UnixStream, CarrierError> {
    let line_spec = SerialSpec::parse(&spec.line()).map_err(|e| refusal(e, spec))?;
    let line = pod_offline::serial::open(&line_spec).map_err(|e| refusal(e, spec))?;
    let fd = bridge::device_fd(spec.path())
        .map_err(|e| CarrierError::new(NAME, CarrierErrorKind::Unavailable, e.to_string()))?;
    let bridge::Pair { app, device } =
        bridge::pair().map_err(|e| CarrierError::io(NAME, "the bridge of the line", e))?;
    let bps = spec.bps();
    std::thread::Builder::new()
        .name("podnet-hf".to_string())
        .spawn(move || run(line, fd, device, bps, last))
        .map_err(|e| CarrierError::io(NAME, "the thread of the line", e))?;
    Ok(app)
}

/// The loop of one modem: the device and the link, one thread.
fn run(
    line: pod_offline::serial::SerialLine,
    fd: std::os::fd::RawFd,
    mut socket: std::os::unix::net::UnixStream,
    bps: u32,
    last: Last,
) {
    let mut session = match session(line, bps) {
        Ok(session) => session,
        Err(e) => {
            bridge::note(&last, format!("{e} ({})", e.name()));
            return;
        }
    };
    let mut outgoing: VecDeque<Vec<u8>> = VecDeque::new();
    let mut incoming: Vec<u8> = Vec::new();
    let mut receiver = Receiver::new();
    let socket_fd = bridge::socket_fd(&socket);
    loop {
        let (device, link) = match bridge::wait_readable(fd, socket_fd, Duration::from_millis(50)) {
            Ok(ready) => ready,
            Err(e) => {
                bridge::note(&last, e.to_string());
                return;
            }
        };
        if link {
            match bridge::read_line(&mut socket) {
                Ok(bytes) if bytes.is_empty() => break,
                Ok(bytes) => incoming.extend_from_slice(&bytes),
                Err(e) => {
                    bridge::note(&last, e.to_string());
                    break;
                }
            }
        }
        while !incoming.is_empty() {
            let take = incoming.len().min(MAX_PAYLOAD);
            let chunk: Vec<u8> = incoming.drain(..take).collect();
            outgoing.push_back(chunk);
        }
        if device {
            match session.receive_at(Instant::now()) {
                Ok(Some(payload)) => match arq::parse(&payload) {
                    // The modem owns the retransmission and the
                    // acknowledgement of this direction, so no
                    // acknowledgement goes back: an `OK` after `TX` takes
                    // the frame out of the ARQ of the session.
                    Ok(Received::Data { seq, payload }) => {
                        if receiver.accept(seq) {
                            if let Err(e) = bridge::write_socket(&mut socket, &payload) {
                                bridge::note(&last, e.to_string());
                                return;
                            }
                        }
                    }
                    Ok(Received::Ack { seq }) => {
                        let _ = session.arq_mut().ack(seq);
                    }
                    Err(e) => bridge::note(&last, format!("{e} ({})", e.name())),
                },
                Ok(None) => {}
                Err(e) => {
                    bridge::note(&last, format!("{e} ({})", e.name()));
                    break;
                }
            }
        }
        // One frame at a time, and only when the channel is free for this
        // end and the turnaround after a reception has ended.
        loop {
            if !outgoing.is_empty()
                && session.modem().activity() == Activity::Idle
                && session
                    .modem()
                    .ready_at()
                    .is_none_or(|at| Instant::now() >= at)
            {
                let payload = outgoing[0].clone();
                match session.send_at(Instant::now(), &payload) {
                    Ok(_) => {
                        outgoing.pop_front();
                    }
                    Err(pod_offline::Error::Busy { .. }) | Err(pod_offline::Error::Receiving) => {
                        break
                    }
                    Err(e) => {
                        bridge::note(&last, format!("{e} ({})", e.name()));
                        return;
                    }
                }
            } else {
                break;
            }
        }
        // A frame above the payload maximum was queued by the split above,
        // so the session never sees one: the split is of the stream, not of
        // a caller frame, and it is the only place where this carrier cuts
        // bytes. The check below keeps the promise of a named error for a
        // frame that a caller of the module builds.
        if outgoing.iter().any(|payload| payload.len() > MAX_PAYLOAD) {
            bridge::note(&last, "the frame is above the maximum of the modem");
            return;
        }
    }
}

impl Carrier for Hf {
    fn name(&self) -> &'static str {
        NAME
    }

    fn facts(&self) -> Facts {
        let bps = self
            .spec
            .as_deref()
            .and_then(|text| HfSpec::parse(text).ok())
            .map_or(DEFAULT_SPEED, |spec| spec.bps());
        Facts {
            ordered: true,
            reliable: true,
            duplex: false,
            mtu: None,
            cost: Cost::High,
            rate: Rate {
                // Eight bits on the air for each byte, at most.
                bytes_per_second: Some(u64::from(bps / 8)),
                poll_interval: None,
                cost: Cost::High,
            },
        }
    }

    fn available(&self) -> Result<(), CarrierError> {
        let Some(text) = self.spec.as_deref() else {
            return Ok(());
        };
        let spec = HfSpec::parse(text)
            .map_err(|e| CarrierError::new(NAME, CarrierErrorKind::Usage, e.to_string()))?;
        let meta = std::fs::metadata(spec.path()).map_err(|e| {
            CarrierError::new(
                NAME,
                CarrierErrorKind::NotFound,
                format!("{}: {e}", spec.path()),
            )
        })?;
        use std::os::unix::fs::FileTypeExt;
        if !meta.file_type().is_char_device() {
            return Err(CarrierError::new(
                NAME,
                CarrierErrorKind::Unavailable,
                format!(
                    "{}: not a character device; the modem is on a serial line",
                    spec.path()
                ),
            ));
        }
        Ok(())
    }

    fn dial<'a>(&'a self, address: &'a str) -> Pending<'a, BoxIo> {
        Box::pin(async move {
            let spec = spec_of(self, address)?;
            let pair = start(&spec, self.last.clone())?;
            Ok(Box::new(pair) as BoxIo)
        })
    }

    fn listen<'a>(&'a self, address: &'a str) -> Pending<'a, Box<dyn Listener>> {
        Box::pin(async move {
            let spec = spec_of(self, address)?;
            let pair = start(&spec, self.last.clone())?;
            Ok(Box::new(HfListener {
                address: spec.address(),
                stream: Mutex::new(Some(pair)),
            }) as Box<dyn Listener>)
        })
    }
}

/// The one end of one modem.
struct HfListener {
    address: String,
    stream: Mutex<Option<tokio::net::UnixStream>>,
}

impl Listener for HfListener {
    fn address(&self) -> String {
        self.address.clone()
    }

    fn accept(&mut self) -> Pending<'_, BoxIo> {
        Box::pin(async move {
            let stream = self.stream.lock().unwrap_or_else(|e| e.into_inner()).take();
            match stream {
                Some(stream) => Ok(Box::new(stream) as BoxIo),
                None => Err(CarrierError::new(
                    NAME,
                    CarrierErrorKind::NotFound,
                    "the modem has one peer, and it is open",
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speed_is_three_hundred_to_three_thousand() {
        assert_eq!(HfSpec::parse("hf:/dev/ttyUSB0").unwrap().bps(), 1200);
        assert_eq!(HfSpec::parse("hf:/dev/ttyUSB0,300").unwrap().bps(), 300);
        assert_eq!(HfSpec::parse("hf:/dev/ttyUSB0,3000").unwrap().bps(), 3000);
        assert_eq!(
            HfSpec::parse("hf:/dev/ttyUSB0,299").unwrap_err().name(),
            "speed"
        );
        assert_eq!(
            HfSpec::parse("hf:/dev/ttyUSB0,3001").unwrap_err().name(),
            "speed"
        );
        assert_eq!(HfSpec::parse("/dev/ttyUSB0").unwrap_err().name(), "address");
    }

    #[test]
    fn the_profile_defers_every_retransmission() {
        assert_eq!(DeferArq::DEGRADED.session_retransmits(), 0);
        let arq = DeferArq::DEGRADED.arq(1200).unwrap();
        assert!(arq.deferring_arq());
        assert_eq!(arq.window(), 1);
    }

    #[test]
    fn the_facts_of_the_carrier_are_half_duplex() {
        let facts = Hf::with_spec("hf:/dev/ttyUSB0,1200".to_string()).facts();
        assert!(!facts.duplex);
        assert_eq!(facts.rate.bytes_per_second, Some(150));
    }

    #[test]
    fn the_timeout_of_the_session_is_not_the_two_hundred_milliseconds() {
        assert!(arq::retransmit_after(1200) > Duration::from_secs(4));
    }
}
