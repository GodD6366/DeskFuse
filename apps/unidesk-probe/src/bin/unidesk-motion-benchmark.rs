//! Synthetic Quinn A/B probe. Not the production input protocol or native input injection.
//! Only aggregate metadata leaves this process; coordinates and credentials are never logged.
use std::{collections::VecDeque, error::Error, net::SocketAddr, sync::Arc, time::Duration};

use serde::Serialize;
use tokio::{
    net::UdpSocket,
    sync::{mpsc, watch},
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const FRAME_BYTES: usize = 33;
const HELP: &str = "unidesk-motion-benchmark [--loss-percent 0|1|3] [--duration-ms 5000] [--rate-hz 125] [--delay-ms 5] [--seed 42] [--burst-every-ms 0] [--burst-ms 0]\nRuns reliable then hybrid over real loopback QUIC, dropping actual UDP packets in BOTH directions after handshake. JSON aggregates only. Fixed synthetic trajectory, same-host monotonic clock, no native input, no production protocol/permission/pairing acceptance. Burst loss starts after each full interval. Independent runs have different QUIC packetization; seed reproduces policy, not exact packet trace.";

#[derive(Clone, Debug, Serialize)]
struct Args {
    loss_percent: f64,
    duration_ms: u64,
    rate_hz: u64,
    delay_ms: u64,
    seed: u64,
    burst_every_ms: u64,
    burst_ms: u64,
}
impl Default for Args {
    fn default() -> Self {
        Self {
            loss_percent: 0.,
            duration_ms: 5000,
            rate_hz: 125,
            delay_ms: 5,
            seed: 42,
            burst_every_ms: 0,
            burst_ms: 0,
        }
    }
}
impl Args {
    fn parse(raw: &[String]) -> std::result::Result<Self, String> {
        let mut out = Self::default();
        let mut args = raw.iter();
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--loss-percent" => out.loss_percent = value.parse().map_err(|_| "invalid loss")?,
                "--duration-ms" => {
                    out.duration_ms = value.parse().map_err(|_| "invalid duration")?
                }
                "--rate-hz" => out.rate_hz = value.parse().map_err(|_| "invalid rate")?,
                "--delay-ms" => out.delay_ms = value.parse().map_err(|_| "invalid delay")?,
                "--seed" => out.seed = value.parse().map_err(|_| "invalid seed")?,
                "--burst-every-ms" => {
                    out.burst_every_ms = value.parse().map_err(|_| "invalid burst interval")?
                }
                "--burst-ms" => {
                    out.burst_ms = value.parse().map_err(|_| "invalid burst duration")?
                }
                _ => return Err(format!("unknown argument {flag}")),
            }
        }
        if !out.loss_percent.is_finite()
            || !(0.0..=30.0).contains(&out.loss_percent)
            || !(100..=120_000).contains(&out.duration_ms)
            || !(1..=1000).contains(&out.rate_hz)
            || out.delay_ms > 250
            || out.burst_every_ms > 120_000
            || out.burst_ms > 2000
            || (out.burst_every_ms == 0) != (out.burst_ms == 0)
            || (out.burst_ms != 0 && out.burst_ms >= out.burst_every_ms)
        {
            return Err("bounds: loss 0..30%, duration 100..120000ms, rate 1..1000Hz, delay 0..250ms; burst needs both values, 0 < burst-ms <= 2000 and burst-ms < every-ms <= 120000".into());
        }
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Reliable,
    Hybrid,
}

// Minimal fixed-size benchmark envelope, deliberately NOT a production input wire message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sample {
    snapshot: bool,
    seq: u64,
    sent_ns: u64,
    x: u64,
    y: u64,
}
impl Sample {
    fn new(seq: u64, origin: Instant, snapshot: bool) -> Self {
        Self {
            snapshot,
            seq,
            sent_ns: origin.elapsed().as_nanos() as u64,
            x: seq % 1920,
            y: (seq * 7) % 1080,
        }
    }
    fn encode(self) -> [u8; FRAME_BYTES] {
        let mut data = [0; FRAME_BYTES];
        data[0] = u8::from(self.snapshot);
        for (chunk, value) in data[1..].as_chunks_mut::<8>().0.iter_mut().zip([
            self.seq,
            self.sent_ns,
            self.x,
            self.y,
        ]) {
            chunk.copy_from_slice(&value.to_be_bytes());
        }
        data
    }
    fn decode(data: &[u8]) -> Option<Self> {
        if data.len() != FRAME_BYTES || data[0] > 1 {
            return None;
        }
        let mut values = data[1..]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| u64::from_be_bytes(*c));
        Some(Self {
            snapshot: data[0] == 1,
            seq: values.next()?,
            sent_ns: values.next()?,
            x: values.next()?,
            y: values.next()?,
        })
    }
}

#[derive(Default, Serialize)]
struct ProxyStats {
    packets_seen: u64,
    random_drops: u64,
    burst_drops: u64,
    queue_overflow_drops: u64,
}
struct Policy {
    state: u64,
}
impl Policy {
    fn drop_random(&mut self, percent: f64) -> bool {
        // SplitMix64 also accepts seed zero, unlike unguarded xorshift.
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((z ^ (z >> 31)) >> 11) as f64 / ((1u64 << 53) as f64) * 100. < percent
    }
}
struct Packet {
    due: Instant,
    target: SocketAddr,
    data: Vec<u8>,
}
fn in_burst(args: &Args, elapsed_ms: u64) -> bool {
    args.burst_every_ms != 0
        && elapsed_ms >= args.burst_every_ms
        && elapsed_ms % args.burst_every_ms < args.burst_ms
}
async fn proxy(
    socket: UdpSocket,
    server: SocketAddr,
    args: Args,
    mut active: watch::Receiver<Option<Instant>>,
    mut stop: watch::Receiver<bool>,
) -> Result<ProxyStats> {
    let mut stats = ProxyStats::default();
    let mut policy = Policy { state: args.seed };
    let mut client = None;
    let mut pending: VecDeque<Packet> = VecDeque::new();
    let mut data = vec![0; 65536];
    loop {
        let next = pending
            .front()
            .map_or_else(|| Instant::now() + Duration::from_secs(3600), |p| p.due);
        tokio::select! {
            _ = stop.changed() => break,
            _ = active.changed() => {},
            received = socket.recv_from(&mut data) => {
                let (size, source) = received?;
                let target = if source == server {
                    let Some(client) = client else { continue; }; client
                } else {
                    if client.is_some_and(|known| known != source) { continue; }
                    client = Some(source); server
                };
                let origin = *active.borrow();
                if let Some(origin) = origin {
                    stats.packets_seen += 1;
                    if in_burst(&args, origin.elapsed().as_millis() as u64) { stats.burst_drops += 1; continue; }
                    if policy.drop_random(args.loss_percent) { stats.random_drops += 1; continue; }
                }
                if pending.len() >= 8192 { stats.queue_overflow_drops += 1; continue; }
                // Fixed delay in both directions. Handshake bypasses impairment, and activation
                // can only increase due times, so the FIFO remains deadline ordered.
                pending.push_back(Packet { due: Instant::now() + Duration::from_millis(if origin.is_some() { args.delay_ms } else { 0 }), target, data: data[..size].to_vec() });
            }
            _ = tokio::time::sleep_until(next), if !pending.is_empty() => {
                if let Some(packet) = pending.pop_front() { socket.send_to(&packet.data, packet.target).await?; }
            }
        }
    }
    Ok(stats)
}

#[derive(Serialize)]
struct Distribution {
    count: usize,
    p50_ms: Option<f64>,
    p95_ms: Option<f64>,
    p99_ms: Option<f64>,
    max_ms: Option<f64>,
}
fn distribution(mut values: Vec<f64>) -> Distribution {
    values.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        if values.is_empty() {
            None
        } else {
            Some(values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)])
        }
    };
    Distribution {
        count: values.len(),
        p50_ms: percentile(0.50),
        p95_ms: percentile(0.95),
        p99_ms: percentile(0.99),
        max_ms: values.last().copied(),
    }
}
#[derive(Default)]
struct Measurements {
    accepted: u64,
    stale: u64,
    invalid: u64,
    latest: Option<Sample>,
    previous_update_ns: Option<u64>,
    age_ms: Vec<f64>,
    gaps_ms: Vec<f64>,
    snapshot_received: bool,
    snapshot_age_ms: Option<f64>,
}
impl Measurements {
    fn receive(&mut self, sample: Sample, at_ns: u64) {
        if sample.x != sample.seq % 1920
            || sample.y != sample.seq * 7 % 1080
            || sample.sent_ns > at_ns
        {
            self.invalid += 1;
            return;
        }
        if sample.snapshot {
            self.snapshot_received = true;
            self.snapshot_age_ms = Some((at_ns - sample.sent_ns) as f64 / 1e6);
        }
        if self.latest.is_some_and(|last| sample.seq <= last.seq) {
            if !sample.snapshot {
                self.stale += 1;
            }
            return;
        }
        self.latest = Some(sample);
        if !sample.snapshot {
            self.accepted += 1;
            self.age_ms.push((at_ns - sample.sent_ns) as f64 / 1e6);
            if let Some(previous) = self.previous_update_ns {
                self.gaps_ms.push((at_ns - previous) as f64 / 1e6);
            }
            self.previous_update_ns = Some(at_ns);
        }
    }
}
#[derive(Serialize)]
struct Run {
    mode: Mode,
    generated_samples: u64,
    accepted_motion_samples: u64,
    stale_motion_samples: u64,
    invalid_samples: u64,
    sample_age: Distribution,
    motion_update_gap: Distribution,
    first_update_ms: Option<f64>,
    final_motion_to_snapshot_ms: Option<f64>,
    final_snapshot_received: bool,
    final_snapshot_age_ms: Option<f64>,
    final_position_converged: bool,
    elapsed_ms: f64,
    proxy: ProxyStats,
}

async fn run(args: &Args, mode: Mode) -> Result<Run> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    let server_config = quinn::ServerConfig::with_single_cert(
        vec![cert.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
    )?;
    let server = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse()?)?;
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.cert.der().clone())?;
    let mut client = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    client.set_default_client_config(quinn::ClientConfig::with_root_certificates(Arc::new(
        roots,
    ))?);
    let relay = UdpSocket::bind("127.0.0.1:0").await?;
    let relay_addr = relay.local_addr()?;
    let (active_tx, active_rx) = watch::channel(None);
    let (stop_tx, stop_rx) = watch::channel(false);
    let proxy_task = tokio::spawn(proxy(
        relay,
        server.local_addr()?,
        args.clone(),
        active_rx,
        stop_rx,
    ));
    let pair = async {
        let connecting = client.connect(relay_addr, "localhost")?;
        let accepting = async {
            server
                .accept()
                .await
                .ok_or("missing incoming")?
                .await
                .map_err(Into::into)
        };
        let (outgoing, incoming): (quinn::Connection, quinn::Connection) = tokio::try_join!(
            async {
                connecting
                    .await
                    .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })
            },
            accepting
        )?;
        Ok::<_, Box<dyn Error + Send + Sync>>((outgoing, incoming))
    };
    let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(10), pair).await??;
    if matches!(mode, Mode::Hybrid)
        && outgoing
            .max_datagram_size()
            .is_none_or(|size| size < FRAME_BYTES)
    {
        return Err("datagram negotiation unavailable".into());
    }
    // Let handshake ACKs settle before timed impairment; each A/B gets a fresh connection.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let origin = Instant::now();
    active_tx.send(Some(origin))?;
    let (tx, mut rx) = mpsc::channel(4096);
    let stream_connection = incoming.clone();
    let stream_tx = tx.clone();
    let reader = tokio::spawn(async move {
        let mut stream = stream_connection.accept_uni().await?;
        loop {
            let mut frame = [0; FRAME_BYTES];
            stream.read_exact(&mut frame).await?;
            if let Some(sample) = Sample::decode(&frame) {
                let done = sample.snapshot;
                if stream_tx
                    .send((sample, origin.elapsed().as_nanos() as u64))
                    .await
                    .is_err()
                    || done
                {
                    break;
                }
            }
        }
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });
    let datagram_connection = incoming.clone();
    let datagram_reader = tokio::spawn(async move {
        while let Ok(bytes) = datagram_connection.read_datagram().await {
            if let Some(sample) = Sample::decode(&bytes)
                && tx
                    .send((sample, origin.elapsed().as_nanos() as u64))
                    .await
                    .is_err()
            {
                break;
            }
        }
    });
    let generated = (args.duration_ms * args.rate_hz / 1000).max(1);
    let send_connection = outgoing.clone();
    let rate = args.rate_hz;
    let sender = tokio::spawn(async move {
        let mut stream = send_connection.open_uni().await?;
        for seq in 1..=generated {
            tokio::time::sleep_until(
                origin + Duration::from_nanos((seq - 1) * 1_000_000_000 / rate),
            )
            .await;
            let sample = Sample::new(seq, origin, false).encode();
            match mode {
                Mode::Reliable => stream.write_all(&sample).await?,
                Mode::Hybrid => send_connection.send_datagram(sample.to_vec().into())?,
            }
        }
        tokio::time::sleep(Duration::from_millis(32)).await;
        stream
            .write_all(&Sample::new(generated, origin, true).encode())
            .await?;
        stream.finish()?;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });
    let mut measurements = Measurements::default();
    let mut first_update_ms = None;
    let deadline = origin + Duration::from_millis(args.duration_ms + 15_000);
    while !measurements.snapshot_received {
        let received = tokio::time::timeout_at(deadline, rx.recv()).await;
        let Ok(Some((sample, at_ns))) = received else {
            break;
        };
        let before = measurements.accepted;
        measurements.receive(sample, at_ns);
        if before == 0 && measurements.accepted > 0 {
            first_update_ms = Some(at_ns as f64 / 1e6);
        }
    }
    let elapsed_ns = origin.elapsed().as_nanos() as u64;
    sender.abort();
    // A finished task's error remains observable; an unfinished sender is a timed-out run.
    match sender.await {
        Ok(result) => result?,
        Err(error) if error.is_cancelled() => {}
        Err(error) => return Err(error.into()),
    }
    reader.abort();
    datagram_reader.abort();
    outgoing.close(0u32.into(), b"benchmark complete");
    incoming.close(0u32.into(), b"benchmark complete");
    let _ = stop_tx.send(true);
    let stats = proxy_task.await??;
    let converged = measurements.latest.is_some_and(|s| {
        s.seq == generated && s.x == generated % 1920 && s.y == generated * 7 % 1080
    });
    Ok(Run {
        mode,
        generated_samples: generated,
        accepted_motion_samples: measurements.accepted,
        stale_motion_samples: measurements.stale,
        invalid_samples: measurements.invalid,
        sample_age: distribution(measurements.age_ms),
        motion_update_gap: distribution(measurements.gaps_ms),
        first_update_ms,
        final_motion_to_snapshot_ms: measurements
            .previous_update_ns
            .map(|last| (elapsed_ns - last) as f64 / 1e6),
        final_snapshot_received: measurements.snapshot_received,
        final_snapshot_age_ms: measurements.snapshot_age_ms,
        final_position_converged: converged && measurements.snapshot_received,
        elapsed_ms: elapsed_ns as f64 / 1e6,
        proxy: stats,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    let args = Args::parse(&raw)?;
    let reliable = run(&args, Mode::Reliable).await?;
    let hybrid = run(&args, Mode::Hybrid).await?;
    let success = reliable.final_position_converged
        && hybrid.final_position_converged
        && reliable.invalid_samples == 0
        && hybrid.invalid_samples == 0
        && reliable.proxy.queue_overflow_drops == 0
        && hybrid.proxy.queue_overflow_drops == 0;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "unidesk-motion-benchmark-v1", "configuration": args,
            "layer": "synthetic_quinn_loopback_udp_packet_impairment",
            "clock": "same_process_monotonic", "wire_format": "synthetic_fixed_33_bytes_not_production_input_protocol",
            "native_input_tested": false, "physical_two_machine_tested": false,
            "impairment": "both_directions_after_handshake_fixed_delay_seeded_packet_loss_optional_periodic_burst",
            "seed_limit": "repeatable_policy_not_identical_quic_packetization_or_scheduling",
            "update_gap_definition": "between_accepted_motion_arrivals_excludes_snapshot; boundary_delays_reported_separately",
            "runs": [reliable, hybrid], "passed_correctness": success
        }))?
    );
    if !success {
        return Err("benchmark convergence/integrity failed; inspect JSON aggregates".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_round_trip_and_reject_truncated() {
        let sample = Sample::new(100, Instant::now(), false);
        assert_eq!(Sample::decode(&sample.encode()), Some(sample));
        assert_eq!(Sample::decode(&[0; 32]), None);
        let mut bytes = sample.encode();
        bytes[0] = 2;
        assert_eq!(Sample::decode(&bytes), None);
    }
    #[test]
    fn old_motion_and_snapshot_do_not_rewind() {
        let origin = Instant::now();
        let mut m = Measurements::default();
        m.receive(Sample::new(4, origin, false), 1_000_000);
        m.receive(Sample::new(2, origin, false), 2_000_000);
        m.receive(Sample::new(3, origin, true), 3_000_000);
        assert_eq!(m.latest.unwrap().seq, 4);
        assert_eq!(m.accepted, 1);
        assert_eq!(m.stale, 1);
        assert!(m.snapshot_received);
    }
    #[test]
    fn final_snapshot_recovers_missing_last_motion() {
        let origin = Instant::now();
        let mut m = Measurements::default();
        m.receive(Sample::new(1, origin, false), 1_000_000);
        m.receive(Sample::new(10, origin, true), 2_000_000);
        assert_eq!(m.latest.unwrap().seq, 10);
        assert_eq!(m.accepted, 1);
    }
    #[test]
    fn policy_is_seeded_and_burst_is_periodic() {
        let mut a = Policy { state: 42 };
        let mut b = Policy { state: 42 };
        let samples: Vec<bool> = (0..1000).map(|_| a.drop_random(3.)).collect();
        assert_eq!(
            samples,
            (0..1000).map(|_| b.drop_random(3.)).collect::<Vec<_>>()
        );
        assert!(samples.iter().any(|v| *v));
        let args = Args {
            burst_every_ms: 1000,
            burst_ms: 100,
            ..Args::default()
        };
        assert!(!in_burst(&args, 0));
        assert!(in_burst(&args, 1000));
        assert!(!in_burst(&args, 1100));
        assert!(in_burst(&args, 2050));
    }
    #[test]
    fn percentile_and_args_bounds() {
        let values = distribution(vec![4., 2., 1., 3.]);
        assert_eq!(values.p50_ms, Some(2.));
        assert_eq!(values.p99_ms, Some(4.));
        assert_eq!(distribution(vec![]).max_ms, None);
        for args in [
            ["--loss-percent", "NaN"],
            ["--rate-hz", "0"],
            ["--burst-ms", "1"],
        ] {
            assert!(Args::parse(&args.map(String::from)).is_err());
        }
    }
}
