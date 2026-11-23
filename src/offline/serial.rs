//! The `serial` carrier: AX.25 in KISS framing over a serial line.
//!
//! The address is `serial:PATH[,BAUD]`, and `--carrier serial:PATH[,BAUD]`
//! is the same address in the carrier flag. The line is opened by
//! `pod-offline` ([`pod_offline::serial::open`]): raw 8N1, `O_NOCTTY`,
//! `TIOCEXCL`, and the settings of the device are put back when the line
//! closes. There is no second serial implementation here.
//!
//! The frames are the AX.25 frames of [`pod_offline::serial::kiss`], with a
//! control byte, a sequence and a payload of at most 255 bytes inside the
//! 256-byte maximum. A frame above that maximum is the named error
//! `frame-too-large`; the carrier never splits it in silence. The window is
//! [`WINDOW`] frames (1 to 7), and the retransmission timeout is two frame
//! times at the baud rate ([`retransmit_after`]): about 4.3 s at 1200 baud,
//! with no fixed floor.
//!
//! The line is point-to-point. The dial and the listen of the carrier both
//! open the device: the two ends of one wire know each other, and the hello
//! of the link proves the keys above it. The line carries one peer for as
//! long as it is held.
//!
//! It cannot do: address a station by an AX.25 callsign (there is no
//! address field), share one line among three or more ends, carry a frame
//! above 256 bytes, or report a byte rate above the line speed.

use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use pod_link::{
    BoxIo, Carrier, CarrierError, CarrierErrorKind, Cost, Facts, Listener, Pending, Rate,
};
use pod_offline::serial::arq::{self, Arq, Received, Receiver, MAX_PAYLOAD as ARQ_MAX_PAYLOAD};
use pod_offline::serial::kiss::{self, Decoder, MAX_FRAME, PORT_AX25};
use pod_offline::serial::SerialSpec;

use super::bridge::{self, Last};

/// The name of the carrier.
pub const NAME: &str = "serial";

/// The window of the sender: four frames. AX.25 numbers a frame modulo 8,
/// so a window is 1 to 7.
pub const WINDOW: u8 = 4;

/// The shortest window.
pub const WINDOW_LEAST: u8 = arq::WINDOW_LEAST;
/// The longest window.
pub const WINDOW_MOST: u8 = arq::WINDOW_MOST;

/// The largest AX.25 frame, with its control byte.
pub const MAX: usize = MAX_FRAME;

/// The largest payload of one frame, with the control byte inside [`MAX`].
pub const MAX_PAYLOAD: usize = ARQ_MAX_PAYLOAD;

/// The time that one 256-byte frame spends on the line.
pub use pod_offline::serial::arq::frame_time;
/// The time after which a frame that is not acknowledged goes again.
pub use pod_offline::serial::arq::retransmit_after;

/// The KISS bytes of one AX.25 frame of `payload`. A payload above the
/// maximum gives `frame-too-large`.
pub fn frame(payload: &[u8]) -> Result<Vec<u8>, pod_offline::Error> {
    kiss::encode(PORT_AX25, payload)
}

/// An ARQ of `window` frames at `baud`.
pub fn arq(baud: u32, window: u8) -> Result<Arq, pod_offline::Error> {
    Arq::new(window, baud)
}

/// The carrier `serial`.
pub struct Serial {
    spec: Option<String>,
    window: u8,
    last: Last,
}

impl Default for Serial {
    fn default() -> Serial {
        Serial {
            spec: None,
            window: WINDOW,
            last: bridge::last(),
        }
    }
}

impl Serial {
    /// A carrier that takes its address from the flag
    /// `--carrier serial:PATH[,BAUD]`.
    pub fn with_spec(spec: String) -> Serial {
        Serial {
            spec: Some(spec),
            ..Serial::default()
        }
    }

    /// The window of the sender.
    pub fn window(&self) -> u8 {
        self.window
    }

    /// The last failure of a line of this carrier, when one ended.
    pub fn failure(&self) -> Option<String> {
        bridge::failure(&self.last)
    }
}

/// The spec of this carrier: the flag, else the address.
fn spec_of<'a>(carrier: &'a Serial, address: &'a str) -> Result<SerialSpec, CarrierError> {
    let text = carrier.spec.as_deref().unwrap_or(address);
    SerialSpec::parse(text).map_err(|e| {
        CarrierError::new(
            NAME,
            CarrierErrorKind::Usage,
            format!("{text}: {} ({})", e, e.name()),
        )
    })
}

fn refusal(e: pod_offline::Error, spec: &SerialSpec) -> CarrierError {
    let kind = match e {
        pod_offline::Error::Device { .. }
        | pod_offline::Error::Open { .. }
        | pod_offline::Error::Settings { .. } => CarrierErrorKind::Unavailable,
        pod_offline::Error::Baud { .. } => CarrierErrorKind::Usage,
        _ => CarrierErrorKind::Io(io::ErrorKind::Other),
    };
    CarrierError::new(NAME, kind, format!("{}: {} ({})", spec.path(), e, e.name()))
}

/// Open the line and start the thread of the bridge.
fn start(
    spec: &SerialSpec,
    window: u8,
    last: Last,
) -> Result<tokio::net::UnixStream, CarrierError> {
    let line = pod_offline::serial::open(spec).map_err(|e| refusal(e, spec))?;
    let fd = bridge::device_fd(spec.path())
        .map_err(|e| CarrierError::new(NAME, CarrierErrorKind::Unavailable, e.to_string()))?;
    let bridge::Pair { app, device } =
        bridge::pair().map_err(|e| CarrierError::io(NAME, "the bridge of the line", e))?;
    let baud = spec.baud();
    std::thread::Builder::new()
        .name("podnet-serial".to_string())
        .spawn(move || run(line, fd, device, baud, window, last))
        .map_err(|e| CarrierError::io(NAME, "the thread of the line", e))?;
    Ok(app)
}

/// The loop of one line: the device and the link, one thread.
fn run(
    mut line: pod_offline::serial::SerialLine,
    fd: std::os::fd::RawFd,
    mut socket: std::os::unix::net::UnixStream,
    baud: u32,
    window: u8,
    last: Last,
) {
    let mut arq = match Arq::new(window, baud) {
        Ok(arq) => arq,
        Err(e) => {
            bridge::note(&last, format!("{e} ({})", e.name()));
            return;
        }
    };
    let mut decoder = Decoder::new();
    let mut receiver = Receiver::new();
    let mut app: Vec<u8> = Vec::new();
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
                Ok(bytes) => app.extend_from_slice(&bytes),
                Err(e) => {
                    bridge::note(&last, e.to_string());
                    break;
                }
            }
        }
        // The bytes of the link go into frames of the maximum payload. The
        // ARQ gives back the frame of each place that is free, and it holds
        // the frames that do not fit its window.
        while !app.is_empty() {
            let take = app.len().min(MAX_PAYLOAD);
            let chunk: Vec<u8> = app.drain(..take).collect();
            match arq.push_at(Instant::now(), &chunk) {
                Ok(Some(frame)) => {
                    let wire = match kiss::encode(PORT_AX25, &frame) {
                        Ok(wire) => wire,
                        Err(e) => {
                            bridge::note(&last, format!("{e} ({})", e.name()));
                            return;
                        }
                    };
                    if let Err(e) = bridge::write_line(&mut line, &wire) {
                        bridge::note(&last, e.to_string());
                        return;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    bridge::note(&last, format!("{e} ({})", e.name()));
                    return;
                }
            }
        }
        if device {
            let bytes = match bridge::read_line(&mut line) {
                Ok(bytes) => bytes,
                Err(e) => {
                    bridge::note(&last, e.to_string());
                    break;
                }
            };
            if bytes.is_empty() {
                break;
            }
            for frame in decoder.push(&bytes) {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(e) => {
                        bridge::note(&last, format!("{e} ({})", e.name()));
                        continue;
                    }
                };
                if frame.port != PORT_AX25 {
                    continue;
                }
                match arq::parse(&frame.payload) {
                    Ok(Received::Data { seq, payload }) => {
                        if receiver.accept(seq) {
                            if let Err(e) = bridge::write_socket(&mut socket, &payload) {
                                bridge::note(&last, e.to_string());
                                return;
                            }
                        }
                        let ack = kiss::encode(PORT_AX25, &[receiver.ack()]);
                        if let Ok(ack) = ack {
                            if let Err(e) = bridge::write_line(&mut line, &ack) {
                                bridge::note(&last, e.to_string());
                                return;
                            }
                        }
                    }
                    Ok(Received::Ack { seq }) => {
                        let _ = arq.ack(seq);
                    }
                    Err(e) => {
                        bridge::note(&last, format!("{e} ({})", e.name()));
                    }
                }
            }
        }
        // A frame with a freed place, and a frame that waited for its
        // timeout, go on the line.
        loop {
            match arq.poll_at(Instant::now()) {
                Ok(Some(frame)) => {
                    let wire = match kiss::encode(PORT_AX25, &frame) {
                        Ok(wire) => wire,
                        Err(e) => {
                            bridge::note(&last, format!("{e} ({})", e.name()));
                            return;
                        }
                    };
                    if let Err(e) = bridge::write_line(&mut line, &wire) {
                        bridge::note(&last, e.to_string());
                        return;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    bridge::note(&last, format!("{e} ({})", e.name()));
                    return;
                }
            }
        }
    }
    let _ = line.restore();
}

impl Carrier for Serial {
    fn name(&self) -> &'static str {
        NAME
    }

    fn facts(&self) -> Facts {
        let baud = self
            .spec
            .as_deref()
            .and_then(|text| SerialSpec::parse(text).ok())
            .map_or(pod_offline::serial::DEFAULT_BAUD, |spec| spec.baud());
        Facts::with_rate(Rate {
            // 8N1: ten bits on the line for each byte.
            bytes_per_second: Some(u64::from(baud / arq::BITS_PER_BYTE)),
            poll_interval: None,
            cost: Cost::High,
        })
    }

    fn available(&self) -> Result<(), CarrierError> {
        let Some(text) = self.spec.as_deref() else {
            return Ok(());
        };
        let spec = SerialSpec::parse(text).map_err(|e| {
            CarrierError::new(
                NAME,
                CarrierErrorKind::Usage,
                format!("{text}: {} ({})", e, e.name()),
            )
        })?;
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
                    "{}: not a character device; a serial line is one, such as /dev/ttyUSB0",
                    spec.path()
                ),
            ));
        }
        Ok(())
    }

    fn dial<'a>(&'a self, address: &'a str) -> Pending<'a, BoxIo> {
        Box::pin(async move {
            let spec = spec_of(self, address)?;
            let pair = start(&spec, self.window, self.last.clone())?;
            Ok(Box::new(pair) as BoxIo)
        })
    }

    fn listen<'a>(&'a self, address: &'a str) -> Pending<'a, Box<dyn Listener>> {
        Box::pin(async move {
            let spec = spec_of(self, address)?;
            let pair = start(&spec, self.window, self.last.clone())?;
            Ok(Box::new(SerialListener {
                address: spec.address(),
                stream: Mutex::new(Some(pair)),
            }) as Box<dyn Listener>)
        })
    }
}

/// The one end of a line: the first accept gives it, and a second waits.
///
/// A wire has two ends, and the two ends are the two ends of this line. A
/// second peer cannot come while this line is held, so a second accept
/// waits instead of returning an error: an error would make the accept loop
/// of `link listen` ask again in a tight loop and log a warning each time.
struct SerialListener {
    address: String,
    stream: Mutex<Option<tokio::net::UnixStream>>,
}

impl Listener for SerialListener {
    fn address(&self) -> String {
        self.address.clone()
    }

    fn accept(&mut self) -> Pending<'_, BoxIo> {
        Box::pin(async move {
            let stream = self.stream.lock().unwrap_or_else(|e| e.into_inner()).take();
            match stream {
                Some(stream) => Ok(Box::new(stream) as BoxIo),
                None => std::future::pending::<Result<BoxIo, CarrierError>>().await,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_timeout_comes_from_the_baud_rate() {
        assert!(retransmit_after(1200) > Duration::from_secs(4));
        assert!(retransmit_after(1200) > Duration::from_millis(200));
        assert!(retransmit_after(9600) < retransmit_after(1200));
    }

    #[test]
    fn the_window_is_one_to_seven() {
        assert!(arq(1200, 0).is_err());
        assert!(arq(1200, 8).is_err());
        for window in WINDOW_LEAST..=WINDOW_MOST {
            assert!(arq(1200, window).is_ok());
        }
        assert_eq!(Serial::default().window(), WINDOW);
    }

    #[test]
    fn a_frame_above_the_maximum_is_refused_by_name() {
        let refused = frame(&vec![0u8; MAX + 1]).unwrap_err();
        assert_eq!(refused.name(), "frame-too-large");
        assert!(frame(&vec![0u8; MAX]).is_ok());
    }
}
