//! Receives dummy WebRTC messages on a UDP socket.
//!
//! The first four bytes of the payload indicate a packet sequence number.
//! The sequence numbers start at 1.
//! Store the incoming packets in a buffer and play them as soon as the next
//! packet in the sequence is available. If it ever detects a loss i.e. a
//! packet is missing after 3 later packets have been received, send a NACK
//! back to the sender that contains the sequence number of the missing packet.
//!
//! On receiving a timeout packet (sequence number is the max u32 integer),
//! print packet statistics. Print the average, p95, and p99 latencies, where
//! the latencies are how long the packet stayed in the queue. Print histogram.
use std::io;
use std::net::SocketAddr;
use std::collections::VecDeque;

use clap::Parser;
use tokio;
use log::{trace, debug};
use tokio::net::UdpSocket;
use tokio::time::{Instant, Duration};

#[derive(Parser)]
struct Cli {
    /// Port to listen on.
    #[arg(long)]
    port: u16,
    /// Client address to send NACKs to.
    #[arg(long)]
    client_addr: SocketAddr,
    /// Number of bytes to expect in the payload.
    #[arg(long, short)]
    bytes: usize,
    /// End-to-end RTT in ms, which is also how often to resend NACKs.
    #[arg(long)]
    rtt: u64,
}

const TIMEOUT_SEQNO: u32 = u32::MAX;

struct Statistics {
    values: Vec<Duration>,
}

impl Statistics {
    /// Create a new histogram for adding duration values.
    fn new() -> Self {
        Self {
            values: Vec::new(),
        }
    }

    /// Add a new duration value.
    fn add_value(&mut self, value: Duration) {
        self.values.push(value);
    }

    /// Print average, p95, and p99 latency statistics.
    fn print_statistics(&mut self) {
        self.values.sort();
        let len = self.values.len();
        println!("Num Values: {}", len);
        println!("Average: {:?}", self.values[(len as f64 * 0.50) as usize]);
        println!("p95: {:?}", self.values[(len as f64 * 0.95) as usize]);
        println!("p99: {:?}", self.values[(len as f64 * 0.99) as usize]);
        println!("Percentiles = {:?}", (0..101).collect::<Vec<_>>());
        println!("Latencies (ns) = {:?}", (0..101)
            .map(|percent| (percent as f64) / 100.0)
            .map(|percent| ((len as f64) * percent) as usize)
            .map(|index| std::cmp::min(index, len - 1))
            .map(|index| self.values[index])
            .map(|duration| duration.as_secs() * 1000000000 + duration.subsec_nanos() as u64)
            .collect::<Vec<_>>());
    }

    /// Print a histogram of the latency statistics.
    fn print_histogram(&self) {
        println!("no histogram yet");
        // unimplemented!()
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Packet {
    seqno: u32,
    time_recv: Option<Instant>,
    time_nack: Option<Instant>,
}

impl Packet {
    fn new(seqno: u32) -> Self {
        Self { seqno, time_recv: None, time_nack: None }
    }
}

struct BufferedPackets {
    send_sock: UdpSocket,
    nack_addr: SocketAddr,
    nack_frequency: Duration,
    /// Next seqno to play, and the seqno of the first packet in the buffer
    /// if the buffer is non-empty.
    next_seqno: u32,
    buffer: VecDeque<Packet>,
}

impl BufferedPackets {
    async fn new(
        nack_addr: SocketAddr, nack_frequency: Duration,
    ) -> io::Result<Self> {
        Ok(Self {
            send_sock: UdpSocket::bind("0.0.0.0:0").await?,
            nack_addr,
            nack_frequency,
            next_seqno: 1,
            buffer: VecDeque::new(),
        })
    }

    /// Receive a packet with this sequence number.
    fn recv_seqno(&mut self, new_seqno: u32, now: Instant) {
        // Ignore the seqno if it has already been received.
        if new_seqno < self.next_seqno {
            return;
        }

        // Add packets to the buffer until the seqno is guaranteed to be there.
        if self.buffer.is_empty() {
            self.buffer.push_back(Packet::new(self.next_seqno));
        }
        let next_seqno_to_push = self.buffer.back().unwrap().seqno + 1;
        for seqno in next_seqno_to_push..(new_seqno + 1) {
            self.buffer.push_back(Packet::new(seqno));
        }

        // Go through the buffer and mark the new packet received.
        for packet in self.buffer.iter_mut() {
            if packet.seqno == new_seqno {
                if packet.time_recv.is_none() {
                    packet.time_recv = Some(now);
                    packet.time_nack = None;
                }
                return;
            }
        }

        // Packet should have been marked received.
        unreachable!()
    }

    /// Return the received time of the next packet to play if the next packet
    /// in the sequence is available. Removes that packet from the buffer.
    fn pop_seqno(&mut self) -> Option<Instant> {
        if !self.buffer.is_empty() && self.buffer.front().unwrap().time_recv.is_some() {
            self.next_seqno += 1;
            Some(self.buffer.pop_front().unwrap().time_recv.unwrap())
        } else {
            None
        }
    }

    /// Send NACKs to the given client address if any packets are missing i.e.,
    /// three later packets have been received. Also resend NACKs if it has
    /// been more than an RTT since the last NACK for that sequence number.
    /// It may be considerably more than an RTT for NACK retransmissions if
    /// this function is only called on receiving a packet.
    async fn send_nacks(
        &mut self, now: Instant,
    ) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        for packet in self.buffer.iter_mut() {
            if packet.time_recv.is_some() {
                continue;
            }
            if let Some(time_nack) = packet.time_nack.as_mut() {
                if now - *time_nack > self.nack_frequency {
                    let buf = packet.seqno.to_be_bytes();
                    debug!("nacking {} (again)", packet.seqno);
                    self.send_sock.send_to(&buf, &self.nack_addr).await?;
                    *time_nack = now;
                }
            } else {
                debug!("nacking {}", packet.seqno);
                let buf = packet.seqno.to_be_bytes();
                packet.time_nack = Some(now);
                self.send_sock.send_to(&buf, &self.nack_addr).await?;
                continue;
            }
        }
        Ok(())
    }
}


#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    env_logger::init();

    let args = Cli::parse();
    let mut stats = Statistics::new();

    // Listen for incoming packets.
    let nack_frequency = Duration::from_millis(args.rtt);
    let mut pkts = BufferedPackets::new(args.client_addr, nack_frequency).await?;
    let mut buf = vec![0; args.bytes];
    let sock = UdpSocket::bind(format!("0.0.0.0:{}", args.port)).await.unwrap();
    debug!("webrtc server is now listening");
    loop {
        let (len, _addr) = sock.recv_from(&mut buf).await?;
        assert_eq!(len, args.bytes);
        let seqno = u32::from_be_bytes([
            buf[0],
            buf[1],
            buf[2],
            buf[3],
        ]);
        trace!("received seqno {} ({} bytes)", seqno, len);
        if seqno == TIMEOUT_SEQNO {
            debug!("timeout message received");
            break;
        }
        let now = Instant::now();
        pkts.recv_seqno(seqno, now);
        while let Some(time_recv) = pkts.pop_seqno() {
            stats.add_value(now - time_recv);
        }
        pkts.send_nacks(now).await?;
    }

    // Print statistics before exiting.
    stats.print_statistics();
    stats.print_histogram();
    Ok(())
}
