//! Isolated, headless LAN input-protocol acceptance. No platform capture,
//! injection, system pasteboard, Keychain or persistent store is linked here.
use deskfuse_core::{
    Capabilities, ClipboardPayload, ConnectionContext, ConnectionState, DeviceId,
    DisplayDescriptor, DisplayId, DisplayTopology, Endpoint, InputPeerStatus, InputSessionId,
    InputShareRole, InputSharingSettings, InputSharingSnapshot, InterfaceKind, LinkScope,
    RouteCandidate, RouteHealth, RouteSource, SharedInputEvent, TrustState,
};
use deskfuse_identity::{MemoryIdentityProvider, MemoryTrustStore};
use deskfuse_runtime::{
    DiscoveryEvent, MemoryDiscovery, MemoryRuntimeStore, RuntimeBuilder, RuntimeCommand,
    RuntimeConfig, RuntimeEvent, RuntimeHandle,
};
use deskfuse_transport::{PeerHello, QuinnTransport, QuinnTransportConfig};
use futures::{StreamExt, stream::BoxStream};
use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

const ACK_MIME: &str = "application/x-deskfuse-input-probe-ack";
const CAPABILITIES: Capabilities = Capabilities::from_bits(
    Capabilities::NETWORK.bits()
        | Capabilities::CLIPBOARD.bits()
        | Capabilities::INPUT_SHARING.bits()
        | Capabilities::INPUT_ACTIVATION.bits()
        | Capabilities::DEVICE_NETWORK.bits(),
);
const HELP: &str = "deskfuse-input-probe --role receiver|controller --run-id TOKEN [--bind IP:PORT] [--peer DEVICE_ID@IP:PORT] [--peer-file PATH] [--rounds 3] [--frames 32] [--timeout 90] [--expect-rejected | --metadata-only] [--hybrid]\n\nReceiver prints DEVICE_ID; controller requires --peer. Only test peers with the same run ID and the opposite role are approved. All identities/trust/settings are in memory. With --hybrid on BOTH sides the negotiated mode must be hybrid; omitted keeps the old capability set. Synthetic display/event fixtures validate transport only; no native input or system clipboard access occurs. With --metadata-only on BOTH sides, sharing and native availability remain disabled; the selected controller and default-disabled receiver must exchange display topology with a two-way receipt fence and no input leases or events.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Controller,
    Receiver,
}
impl Role {
    fn label(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Receiver => "receiver",
        }
    }
    fn other(self) -> Self {
        match self {
            Self::Controller => Self::Receiver,
            Self::Receiver => Self::Controller,
        }
    }
}
#[derive(Debug)]
struct Args {
    role: Role,
    run_id: String,
    bind: SocketAddr,
    peer: Option<(DeviceId, Endpoint)>,
    peer_file: Option<std::path::PathBuf>,
    rounds: u32,
    frames: u32,
    timeout: Duration,
    expect_rejected: bool,
    metadata_only: bool,
    hybrid: bool,
}
impl Args {
    fn parse(raw: &[String]) -> Result<Self, String> {
        let mut role = None;
        let mut run_id = None;
        let mut bind = "0.0.0.0:43424".parse().unwrap();
        let mut peer = None;
        let mut peer_file = None;
        let mut rounds = 3;
        let mut frames = 32;
        let mut timeout = 90;
        let mut expect_rejected = false;
        let mut metadata_only = false;
        let mut hybrid = false;
        let mut args = raw.iter();
        while let Some(flag) = args.next() {
            if flag == "--hybrid" {
                hybrid = true;
                continue;
            }
            if flag == "--expect-rejected" {
                expect_rejected = true;
                continue;
            }
            if flag == "--metadata-only" {
                metadata_only = true;
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--role" => {
                    role = Some(match value.as_str() {
                        "controller" => Role::Controller,
                        "receiver" => Role::Receiver,
                        _ => return Err("role must be controller or receiver".into()),
                    })
                }
                "--run-id" => run_id = Some(value.clone()),
                "--bind" => bind = value.parse::<SocketAddr>().map_err(|e| e.to_string())?,
                "--peer" => {
                    let (id, endpoint) = value
                        .split_once('@')
                        .ok_or("peer must be DEVICE_ID@IP:PORT")?;
                    peer = Some((
                        DeviceId::from_hex(id).map_err(|e| e.to_string())?,
                        endpoint.parse::<Endpoint>().map_err(|e| e.to_string())?,
                    ));
                }
                "--peer-file" => peer_file = Some(std::path::PathBuf::from(value)),
                "--rounds" => rounds = value.parse::<u32>().map_err(|e| e.to_string())?,
                "--frames" => frames = value.parse::<u32>().map_err(|e| e.to_string())?,
                "--timeout" => timeout = value.parse::<u64>().map_err(|e| e.to_string())?,
                _ => return Err(format!("unknown argument {flag}")),
            }
        }
        let role = role.ok_or("--role is required")?;
        let run_id = run_id.ok_or("--run-id is required")?;
        if run_id.is_empty()
            || run_id.len() > 64
            || !run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(
                "run ID must be 1–64 ASCII letters, numbers, underscores or hyphens".into(),
            );
        }
        if expect_rejected && role != Role::Controller {
            return Err("--expect-rejected requires --role controller".into());
        }
        if expect_rejected && metadata_only {
            return Err("--metadata-only cannot be combined with --expect-rejected".into());
        }
        if role == Role::Controller && peer.is_none() {
            return Err("controller requires --peer DEVICE_ID@IP:PORT".into());
        }
        if !(1..=32).contains(&rounds)
            || !(16..=256).contains(&frames)
            || !(10..=600).contains(&timeout)
            || bind.port() == 0
        {
            return Err("rounds=1..32, frames=16..256, timeout=10..600 and a nonzero bind port are required".into());
        }
        Ok(Self {
            role,
            run_id,
            bind,
            peer,
            peer_file,
            rounds,
            frames,
            timeout: Duration::from_secs(timeout),
            expect_rejected,
            metadata_only,
            hybrid,
        })
    }
    fn capabilities(&self) -> Capabilities {
        if self.hybrid {
            CAPABILITIES | Capabilities::INPUT_MOTION_DATAGRAM_V1
        } else {
            CAPABILITIES
        }
    }
    fn name(&self, role: Role) -> String {
        format!("deskfuse-input-probe:{}:{}", self.run_id, role.label())
    }
    fn approves(&self, name: &str, id: DeviceId, selected: Option<DeviceId>) -> bool {
        name == self.name(self.role.other())
            && selected.is_none_or(|peer| peer == id)
            && self.peer.as_ref().is_none_or(|(peer, _)| *peer == id)
    }
}

fn topology(role: Role) -> DisplayTopology {
    DisplayTopology {
        generation: 1,
        displays: [("main", 0.0, true), ("secondary", 1920.0, false)]
            .into_iter()
            .map(|(suffix, x, primary)| DisplayDescriptor {
                id: format!("probe-{}-{suffix}", role.label()).into(),
                frame_x: x,
                frame_y: 0.0,
                width: 1920,
                height: 1080,
                scale_factor: 2.0,
                rotation_degrees: 0,
                is_primary: primary,
                physical_width_mm: None,
                physical_height_mm: None,
            })
            .collect(),
    }
}
fn target(round: u32) -> DisplayId {
    format!(
        "probe-receiver-{}",
        if round.is_multiple_of(2) {
            "secondary"
        } else {
            "main"
        }
    )
    .into()
}
fn entry(round: u32) -> SharedInputEvent {
    SharedInputEvent::Motion {
        x: 10.0 + f64::from(round),
        y: 20.0,
    }
}
fn frame(round: u32, index: u32) -> Vec<SharedInputEvent> {
    let modifier = 1 << 20;
    vec![
        SharedInputEvent::Modifiers {
            modifiers: modifier,
        },
        SharedInputEvent::Key {
            code: 0,
            pressed: true,
            modifiers: modifier,
            repeat: false,
        },
        SharedInputEvent::Key {
            code: 0,
            pressed: true,
            modifiers: modifier,
            repeat: true,
        },
        SharedInputEvent::Key {
            code: 0,
            pressed: false,
            modifiers: modifier,
            repeat: false,
        },
        SharedInputEvent::Modifiers { modifiers: 0 },
        SharedInputEvent::Scroll { dx: 1.0, dy: -2.0 },
        SharedInputEvent::Button {
            button: 0,
            pressed: true,
        },
        SharedInputEvent::Motion {
            x: 100.0 + f64::from(index),
            y: 200.0 + f64::from(round),
        },
        SharedInputEvent::Button {
            button: 0,
            pressed: false,
        },
    ]
}
fn verified_link(
    runtime: &RuntimeHandle,
    peer: DeviceId,
    session: Option<InputSessionId>,
) -> Result<InputPeerStatus, String> {
    let snapshot = runtime.input_snapshot();
    let link = snapshot
        .links
        .iter()
        .find(|l| l.peer_id == peer && session.is_none_or(|s| l.control_session == Some(s)))
        .ok_or("input peer/session missing")?;
    let context = link
        .context
        .as_ref()
        .ok_or("input connection context missing")?;
    if !link.ready
        || !context.input_eligible()
        || context.scope != LinkScope::Lan
        || !matches!(
            context.interface_kind,
            InterfaceKind::WiFi | InterfaceKind::Ethernet | InterfaceKind::Thunderbolt
        )
        || context.interface_name.as_ref().is_none_or(String::is_empty)
    {
        return Err(format!(
            "input is not a verified physical LAN link: ready={} reason={:?} context={context:?}",
            link.ready, link.reason
        ));
    }
    Ok(link.clone())
}
async fn command(runtime: &RuntimeHandle, command: RuntimeCommand) -> Result<(), String> {
    runtime.send(command).await.map_err(|e| e.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AckKind {
    Frames,
    Left,
    Released,
    Finish,
    Finished,
    MetadataReady,
    MetadataFinish,
    MetadataFinished,
}
impl AckKind {
    fn label(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::Left => "left",
            Self::Released => "released",
            Self::Finish => "finish",
            Self::Finished => "finished",
            Self::MetadataReady => "metadata-ready",
            Self::MetadataFinish => "metadata-finish",
            Self::MetadataFinished => "metadata-finished",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ack {
    run_id: String,
    kind: AckKind,
    round: u32,
    session: String,
    frames: u32,
    event_count: u64,
    last_sequence: u64,
}
impl Ack {
    fn encode(&self) -> Vec<u8> {
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.run_id,
            self.kind.label(),
            self.round,
            self.session,
            self.frames,
            self.event_count,
            self.last_sequence
        )
        .into_bytes()
    }
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let value = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let parts = value.split('|').collect::<Vec<_>>();
        if parts.len() != 7 {
            return Err("malformed acknowledgement".into());
        }
        let kind = match parts[1] {
            "frames" => AckKind::Frames,
            "left" => AckKind::Left,
            "released" => AckKind::Released,
            "finish" => AckKind::Finish,
            "finished" => AckKind::Finished,
            "metadata-ready" => AckKind::MetadataReady,
            "metadata-finish" => AckKind::MetadataFinish,
            "metadata-finished" => AckKind::MetadataFinished,
            _ => return Err("unknown acknowledgement stage".into()),
        };
        Ok(Self {
            run_id: parts[0].into(),
            kind,
            round: parts[2].parse::<u32>().map_err(|e| e.to_string())?,
            session: parts[3].into(),
            frames: parts[4].parse::<u32>().map_err(|e| e.to_string())?,
            event_count: parts[5].parse::<u64>().map_err(|e| e.to_string())?,
            last_sequence: parts[6].parse::<u64>().map_err(|e| e.to_string())?,
        })
    }
}
async fn send_ack(
    runtime: &RuntimeHandle,
    peer: DeviceId,
    ack: &Ack,
    sequence: &mut u64,
) -> Result<(), String> {
    *sequence += 1;
    command(
        runtime,
        RuntimeCommand::SendClipboard {
            peer_id: peer,
            payload: ClipboardPayload::new(ACK_MIME, ack.encode(), *sequence)
                .map_err(|e| e.to_string())?,
        },
    )
    .await
}
type EventStream = BoxStream<'static, RuntimeEvent>;

fn is_ordered_input_event(event: &RuntimeEvent) -> bool {
    matches!(
        event,
        RuntimeEvent::SharedInputReceived { .. }
            | RuntimeEvent::InputControlGranted { .. }
            | RuntimeEvent::InputControlReleased { .. }
            | RuntimeEvent::InputDeliveryInvalidated { .. }
    )
}

fn combined_events(runtime: &RuntimeHandle) -> EventStream {
    // Control/lifecycle edges are present in both streams. Keep them exclusively
    // with native input so they cannot overtake frames or be counted twice.
    let ordinary = runtime
        .event_stream()
        .filter(|event| futures::future::ready(!is_ordered_input_event(event)));
    let input = runtime
        .input_event_stream()
        .filter(|event| futures::future::ready(is_ordered_input_event(event)));
    Box::pin(futures::stream::select(ordinary, input))
}

async fn bootstrap(
    args: &Args,
    runtime: &RuntimeHandle,
    events: &mut EventStream,
    discovery: &MemoryDiscovery,
) -> Result<DeviceId, String> {
    let deadline = tokio::time::Instant::now() + args.timeout;
    let mut selected = args.peer.as_ref().map(|(id, _)| *id);
    let mut dialed = false;
    let mut decided = BTreeSet::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(deadline)=>return Err("timed out waiting for scoped test pairing".into()),
            _=ticker.tick()=>{
                if selected.is_none() && let Some(path)=&args.peer_file {
                    match std::fs::read_to_string(path) {
                        Ok(value)=>{
                            let(id,endpoint)=value.trim().split_once('@').ok_or("peer file must contain DEVICE_ID@IP:PORT")?;
                            let peer=DeviceId::from_hex(id).map_err(|e|e.to_string())?;
                            let endpoint=endpoint.parse::<Endpoint>().map_err(|e|e.to_string())?;
                            discovery.push(DiscoveryEvent::PeerHint {peer_id:peer,display_name:args.name(args.role.other()),capabilities:args.capabilities(),routes:vec![RouteCandidate::new(endpoint,RouteSource::Manual,RouteHealth::Unknown,args.capabilities())]});
                            selected=Some(peer);println!("PEER_HINT peer={peer} source=isolated-peer-file");
                        }
                        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
                        Err(error)=>return Err(format!("cannot read isolated peer file: {error}")),
                    }
                }
                let view=runtime.snapshot_view();
                for pending in &view.device_network.pending {
                    if !pending.incoming || !decided.insert(pending.request_id) {continue;}
                    let Some(request)=&pending.request else{return Err("incoming invitation omitted signed request".into());};
                    let accepted=args.role==Role::Receiver && pending.peer_id==request.device_id() && args.approves(request.display_name(),request.device_id(),selected);
                    println!("PAIRING_REQUEST peer={} name={} accepted={accepted} source=verified_incoming",request.device_id(),request.display_name());
                    if accepted {selected=Some(request.device_id());command(runtime,RuntimeCommand::approve_pairing(pending.request_id)).await?;}else{command(runtime,RuntimeCommand::reject_pairing(pending.request_id)).await?;}
                }
                if let Some(id)=selected {
                    if view.snapshot.peers.iter().any(|p|p.id==id&&p.trust==TrustState::Trusted&&p.connection==ConnectionState::Connected) {println!("PAIRED peer={id} role={}",args.role.label());return Ok(id);}
                    if !dialed && args.role==Role::Controller && (view.discovered_peers.iter().any(|p|p.peer_id==id)||view.snapshot.peers.iter().any(|p|p.id==id)) {
                        command(runtime,RuntimeCommand::RequestPairing{peer_id:id}).await?;dialed=true;println!("PAIRING_SENT peer={id} waiting_for_receiver=true");
                    }
                }
            },
            event=events.next()=>match event.ok_or("runtime event stream ended during pairing")? {
                RuntimeEvent::PairingFinished{outcome,..} if outcome!="accepted"=>return Err(format!("pairing ended: {outcome}")),
                RuntimeEvent::Error(error)=>eprintln!("PAIRING_DIAGNOSTIC {error}"),
                _=>{}
            }
        }
    }
}
async fn expect_rejection(
    args: &Args,
    runtime: &RuntimeHandle,
    events: &mut EventStream,
) -> Result<(), String> {
    let peer = args
        .peer
        .as_ref()
        .ok_or("expected rejection requires pinned peer")?
        .0;
    let deadline = tokio::time::Instant::now() + args.timeout;
    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    let mut sent = false;
    let mut request = None;
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(deadline)=>return Err("timed out waiting for recipient rejection".into()),
            _=ticker.tick()=>{
                let view=runtime.snapshot_view();
                if view.snapshot.peers.iter().any(|p|p.id==peer&&p.trust==TrustState::Trusted){return Err("recipient unexpectedly granted trust in rejection-only mode".into());}
                if !sent && (view.discovered_peers.iter().any(|p|p.peer_id==peer)||view.snapshot.peers.iter().any(|p|p.id==peer)) {command(runtime,RuntimeCommand::RequestPairing{peer_id:peer}).await?;sent=true;}
            },
            event=events.next()=>match event.ok_or("runtime event stream ended while awaiting rejection")? {
                RuntimeEvent::PairingRequestSent{peer_id,request_id} if peer_id==peer=>{request=Some(request_id);println!("PAIRING_SENT peer={peer} request={request_id} expected=rejected NATIVE_INPUT=false");},
                RuntimeEvent::PairingFinished{peer_id,request_id,outcome} if peer_id==peer && request==Some(request_id)=>{
                    if outcome!="rejected" {return Err(format!("expected rejection, received {outcome}"));}
                    let view=runtime.snapshot_view();
                    if view.snapshot.peers.iter().any(|p|p.id==peer&&matches!(p.trust,TrustState::Trusted|TrustState::Blocked)) {return Err("rejected invitation incorrectly trusted or blocked the peer".into());}
                    if view.device_network.member_ids.contains(&peer){return Err("rejected peer was admitted to the device network".into());}
                    println!("RESULT PASS role=controller expected_rejected=true request={request_id} trusted=false blocked=false native_input=false");return Ok(());
                },
                RuntimeEvent::InputDeliveryInvalidated {reason}=>return Err(format!("input delivery invalidated: {reason}")),
                RuntimeEvent::Error(error)=>return Err(error.to_string()),
                _=>{}
            }
        }
    }
}

async fn configure(args: &Args, runtime: &RuntimeHandle, peer: DeviceId) -> Result<(), String> {
    let settings = input_fixture_settings(args.role, peer);
    command(runtime, RuntimeCommand::SetInputSharingSettings(settings)).await?;
    command(
        runtime,
        RuntimeCommand::PublishDisplayTopology(topology(args.role)),
    )
    .await?;
    // This is an explicit headless protocol fixture, not a fabricated native
    // permission claim: no capture/injection adapter exists in this binary.
    command(
        runtime,
        RuntimeCommand::SetInputAvailability {
            available: args.role == Role::Controller,
            reason: Some("isolated headless protocol fixture; no native input".into()),
        },
    )
    .await
}

fn input_fixture_settings(role: Role, peer: DeviceId) -> InputSharingSettings {
    let mut settings = metadata_settings(role, peer);
    settings.enabled = role == Role::Controller;
    settings
}

async fn confirm_managed_fixture(
    runtime: &RuntimeHandle,
    peer: DeviceId,
    confirmed: &mut Option<deskfuse_core::InputActivationId>,
) -> Result<(), String> {
    let snapshot = runtime.input_snapshot();
    if snapshot.settings != InputSharingSettings::default() || !snapshot.scenes.is_empty() {
        return Err(
            "receiver persisted settings/scenes changed during automatic management".into(),
        );
    }
    if let Some(managed) = &snapshot.managed_control {
        if managed.controller != peer {
            return Err("unexpected managed controller".into());
        }
        if *confirmed != Some(managed.activation_id) {
            command(
                runtime,
                RuntimeCommand::SetManagedInputAvailability {
                    activation_id: managed.activation_id,
                    status: deskfuse_core::InputActivationStatus::Ready,
                    reason: Some("isolated native-ready protocol fixture; no native input".into()),
                },
            )
            .await?;
            *confirmed = Some(managed.activation_id);
            println!(
                "MANAGED_NATIVE_READY activation={} persisted_enabled=false persisted_role=Disabled persisted_authorized_controller=none native_input=false",
                managed.activation_id
            );
        }
    }
    Ok(())
}

fn metadata_settings(role: Role, peer: DeviceId) -> InputSharingSettings {
    InputSharingSettings {
        role: if role == Role::Controller {
            InputShareRole::Controller
        } else {
            InputShareRole::Disabled
        },
        participants: if role == Role::Controller {
            vec![peer]
        } else {
            vec![]
        },
        ..Default::default()
    }
}

fn metadata_evidence(
    args: &Args,
    snapshot: &InputSharingSnapshot,
    peer: DeviceId,
) -> Result<Option<ConnectionContext>, String> {
    if snapshot.settings.enabled
        || snapshot
            .links
            .iter()
            .any(|link| link.ready || link.control_session.is_some())
    {
        return Err(
            "metadata-only mode unexpectedly enabled sharing or acquired an input lease".into(),
        );
    }
    if snapshot.settings != metadata_settings(args.role, peer)
        || snapshot.local_topology.as_ref() != Some(&topology(args.role))
        || snapshot.remote_topologies.get(&peer) != Some(&topology(args.role.other()))
    {
        return Ok(None);
    }
    Ok(snapshot
        .links
        .iter()
        .find(|link| link.peer_id == peer)
        .and_then(|link| link.context.clone())
        .filter(ConnectionContext::input_eligible))
}

fn metadata_ack(args: &Args, kind: AckKind) -> Ack {
    Ack {
        run_id: args.run_id.clone(),
        kind,
        round: 0,
        session: args.role.label().into(),
        frames: 0,
        event_count: 0,
        last_sequence: 0,
    }
}

fn validate_metadata_ack(args: &Args, ack: &Ack) -> Result<(), String> {
    if ack.run_id != args.run_id
        || ack.session != args.role.other().label()
        || ack.round != 0
        || ack.frames != 0
        || ack.event_count != 0
        || ack.last_sequence != 0
        || !matches!(
            ack.kind,
            AckKind::MetadataReady | AckKind::MetadataFinish | AckKind::MetadataFinished
        )
    {
        return Err(format!("unexpected metadata receipt: {ack:?}"));
    }
    Ok(())
}

async fn metadata_only(
    args: &Args,
    runtime: &RuntimeHandle,
    events: &mut EventStream,
    peer: DeviceId,
) -> Result<(), String> {
    // Deliberately never call the normal input fixture configuration.
    command(
        runtime,
        RuntimeCommand::SetInputAvailability {
            available: false,
            reason: Some("metadata-only fixture; native input unavailable".into()),
        },
    )
    .await?;
    command(
        runtime,
        RuntimeCommand::SetInputSharingSettings(metadata_settings(args.role, peer)),
    )
    .await?;
    command(
        runtime,
        RuntimeCommand::PublishDisplayTopology(topology(args.role)),
    )
    .await?;
    let deadline = tokio::time::Instant::now() + args.timeout;
    let mut ticker = tokio::time::interval(Duration::from_millis(25));
    let mut sequence = 0;
    let mut local_ready = false;
    let mut peer_ready = false;
    let mut finish_sent = false;
    let mut finish_received = false;
    let mut finished_received = false;
    let mut complete_at = None;
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(deadline)=>return Err(format!("metadata-only timed out: local_ready={local_ready} peer_ready={peer_ready} snapshot={:?}",runtime.input_snapshot())),
            _=ticker.tick()=>{
                let evidence=metadata_evidence(args,&runtime.input_snapshot(),peer)?;
                if local_ready && evidence.is_none() && complete_at.is_none() {return Err("verified metadata or LAN context disappeared before completion".into());}
                if !local_ready && let Some(context)=evidence {
                    println!("METADATA_RECEIVED role={} peer={peer} displays={} enabled=false native_available=false context={context:?}",args.role.label(),topology(args.role.other()).displays.len());
                    send_ack(runtime,peer,&metadata_ack(args,AckKind::MetadataReady),&mut sequence).await?;
                    local_ready=true;
                }
                if local_ready && peer_ready && !finish_sent {
                    let kind=match args.role {
                        Role::Controller=>Some(AckKind::MetadataFinish),
                        Role::Receiver if finish_received=>Some(AckKind::MetadataFinished),
                        Role::Receiver=>None,
                    };
                    if let Some(kind)=kind {
                        send_ack(runtime,peer,&metadata_ack(args,kind),&mut sequence).await?;
                        finish_sent=true;
                        if args.role==Role::Receiver {complete_at=Some(tokio::time::Instant::now()+Duration::from_millis(300));}
                    }
                }
                if args.role==Role::Controller && finish_sent && finished_received && complete_at.is_none() {
                    complete_at=Some(tokio::time::Instant::now()+Duration::from_millis(300));
                }
                if complete_at.is_some_and(|at|tokio::time::Instant::now()>=at) {
                    println!("RESULT PASS role={} metadata_only=true bidirectional_topology=true peer_receipt=true enabled=false native_available=false control_sessions=0 input_events=0 native_input=false",args.role.label());
                    return Ok(());
                }
            },
            event=events.next()=>match event.ok_or("runtime event stream ended during metadata-only verification")? {
                RuntimeEvent::ClipboardReceived{peer_id,payload} if peer_id==peer && payload.mime==ACK_MIME=>{
                    let ack=Ack::decode(&payload.data)?;
                    validate_metadata_ack(args,&ack)?;
                    println!("METADATA_ACK role={} stage={} sender={}",args.role.label(),ack.kind.label(),ack.session);
                    match ack.kind {
                        AckKind::MetadataReady=>peer_ready=true,
                        AckKind::MetadataFinish if args.role==Role::Receiver=>finish_received=true,
                        AckKind::MetadataFinished if args.role==Role::Controller=>finished_received=true,
                        _=>return Err("metadata receipt used the wrong sender role".into()),
                    }
                },
                RuntimeEvent::SharedInputReceived{..}|RuntimeEvent::InputReceived{..}|RuntimeEvent::InputControlGranted{..}=>return Err("metadata-only mode received input or an input grant".into()),
                RuntimeEvent::InputDeliveryInvalidated {reason}=>return Err(format!("input delivery invalidated: {reason}")),
                RuntimeEvent::Error(error)=>return Err(error.to_string()),
                _=>{}
            }
        }
    }
}

async fn wait_ack(
    args: &Args,
    _runtime: &RuntimeHandle,
    events: &mut EventStream,
    peer: DeviceId,
    round: u32,
    session: InputSessionId,
    kind: AckKind,
) -> Result<Ack, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(deadline)=>return Err(format!("round {round}: no receiver {} acknowledgement",kind.label())),
            event=events.next()=>match event.ok_or("runtime event stream ended")? {
                RuntimeEvent::ClipboardReceived{peer_id,payload} if peer_id==peer&&payload.mime==ACK_MIME=>{
                    let ack=Ack::decode(&payload.data)?;
                    if ack.run_id!=args.run_id||ack.round!=round||ack.session!=session.to_string()||ack.kind!=kind{return Err(format!("unexpected acknowledgement: {ack:?}"));}
                    if ack.frames!=args.frames||ack.event_count!=1+u64::from(args.frames)*frame(round,0).len() as u64||ack.last_sequence==0{return Err(format!("receiver evidence mismatch: {ack:?}"));}
                    println!("ACK round={round} stage={} session={session} frames={} events={} last_sequence={}",kind.label(),ack.frames,ack.event_count,ack.last_sequence);
                    return Ok(ack);
                },
                RuntimeEvent::InputControlDenied{reason,..}=>return Err(format!("input control denied: {reason}")),
                RuntimeEvent::InputLinkUnavailable{peer_id,reason} if peer_id==peer=>return Err(format!("input link unavailable: {reason}")),
                RuntimeEvent::InputControlReleased{session_id,reason,..} if session_id==session && kind==AckKind::Frames=>return Err(format!("session released before frame verification: {reason}")),
                RuntimeEvent::InputDeliveryInvalidated {reason}=>return Err(format!("input delivery invalidated: {reason}")),
                RuntimeEvent::Error(error)=>return Err(error.to_string()),
                _=>{}
            }
        }
        // A runtime snapshot alone is never used as receiver acknowledgement.
    }
}

async fn controller(
    args: &Args,
    runtime: &RuntimeHandle,
    events: &mut EventStream,
    peer: DeviceId,
) -> Result<(), String> {
    let mut sessions = BTreeSet::new();
    let mut final_ack = None;
    for round in 1..=args.rounds {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let mut requested = false;
        let (session, initial_revision) = loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "round {round}: no authorized LAN lease; snapshot={:?}",
                    runtime.input_snapshot()
                ));
            }
            if let Ok(link) = verified_link(runtime, peer, None) {
                if !requested {
                    command(
                        runtime,
                        RuntimeCommand::RequestInputControl {
                            peer_id: peer,
                            target_display: target(round),
                        },
                    )
                    .await?;
                    requested = true;
                }
                if let Some(session) = link.control_session
                    && !sessions.contains(&session)
                {
                    if args.hybrid && link.motion_mode.as_deref() != Some("hybrid") {
                        return Err(format!(
                            "--hybrid requested but negotiated {:?}",
                            link.motion_mode
                        ));
                    }
                    println!("MOTION_MODE={:?}", link.motion_mode);
                    println!(
                        "LAN round={round} session={session} context={:?} lease_revision={}",
                        link.context, link.lease_revision
                    );
                    break (session, link.lease_revision);
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        sessions.insert(session);
        let SharedInputEvent::Motion { x, y } = entry(round) else {
            unreachable!()
        };
        command(
            runtime,
            RuntimeCommand::EnterInputControl {
                session_id: session,
                target_display: target(round),
                x,
                y,
            },
        )
        .await?;
        for index in 0..args.frames {
            let fixture = frame(round, index);
            let batches = if args.hybrid {
                // Let the Datagram worker deliver drag motion before the reliable
                // button-up anchor supersedes it. This is a correctness fixture,
                // not a rate-matched benchmark (use deskfuse-motion-benchmark).
                vec![
                    fixture[..7].to_vec(),
                    fixture[7..8].to_vec(),
                    fixture[8..].to_vec(),
                ]
            } else {
                vec![fixture]
            };
            for events in batches {
                command(
                    runtime,
                    RuntimeCommand::SendSharedInput {
                        session_id: session,
                        target_display: target(round),
                        events,
                        relative: None,
                    },
                )
                .await?;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        let ack = wait_ack(args, runtime, events, peer, round, session, AckKind::Frames).await?;
        let link = verified_link(runtime, peer, Some(session))?;
        if link.lease_revision <= initial_revision {
            return Err(format!(
                "round {round}: no observed grant/renew progress during frame delivery"
            ));
        }
        command(
            runtime,
            RuntimeCommand::LeaveInputControl {
                session_id: session,
            },
        )
        .await?;
        wait_ack(args, runtime, events, peer, round, session, AckKind::Left).await?;
        if verified_link(runtime, peer, Some(session))?.control_session != Some(session) {
            return Err("Leave destroyed the warm lease".into());
        }
        command(
            runtime,
            RuntimeCommand::ReleaseInputControl {
                session_id: session,
            },
        )
        .await?;
        wait_ack(
            args,
            runtime,
            events,
            peer,
            round,
            session,
            AckKind::Released,
        )
        .await?;
        let cleanup = tokio::time::Instant::now() + Duration::from_secs(2);
        while runtime
            .input_snapshot()
            .links
            .iter()
            .any(|l| l.control_session == Some(session))
        {
            if tokio::time::Instant::now() >= cleanup {
                return Err("released lease remained in snapshot".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // A released session must never produce another receiver input event.
        command(
            runtime,
            RuntimeCommand::SendSharedInput {
                session_id: session,
                target_display: target(round),
                events: frame(round, 0),
                relative: None,
            },
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        final_ack = Some(Ack {
            kind: AckKind::Finish,
            ..ack
        });
    }
    let mut sequence = 1;
    let final_ack = final_ack.ok_or("no completed round")?;
    send_ack(runtime, peer, &final_ack, &mut sequence).await?;
    let last_session = *sessions
        .iter()
        .find(|s| s.to_string() == final_ack.session)
        .ok_or("final session missing")?;
    wait_ack(
        args,
        runtime,
        events,
        peer,
        args.rounds,
        last_session,
        AckKind::Finished,
    )
    .await?;
    // Give the receiver its explicit completion fence before orderly shutdown.
    tokio::time::sleep(Duration::from_millis(250)).await;
    println!(
        "RESULT PASS role=controller rounds={} unique_sessions={} strict_sequences=true lease_renewal=true native_input=false",
        args.rounds,
        sessions.len()
    );
    Ok(())
}

#[derive(Debug)]
struct ReceivedRound {
    session: InputSessionId,
    round: u32,
    frames: u32,
    event_count: u64,
    last_sequence: u64,
    left: bool,
    hybrid_event_offset: usize,
    last_state_epoch: u64,
    pointer: SharedInputEvent,
}
impl ReceivedRound {
    fn receive(
        &mut self,
        target_display: &DisplayId,
        sequence: u64,
        events: &[SharedInputEvent],
        expected_frames: u32,
    ) -> Result<(), String> {
        if self.left || sequence <= self.last_sequence || target_display != &target(self.round) {
            return Err(
                "input arrived after Leave, out of sequence, or on the wrong display".into(),
            );
        }
        if self.frames >= expected_frames || events != frame(self.round, self.frames) {
            return Err(format!(
                "round {} frame {} has unexpected event content",
                self.round, self.frames
            ));
        }
        self.frames += 1;
        self.event_count += events.len() as u64;
        self.last_sequence = sequence;
        Ok(())
    }
    fn receive_hybrid(
        &mut self,
        target_display: &DisplayId,
        sequence: u64,
        state_epoch: u64,
        events: &[SharedInputEvent],
        expected_frames: u32,
    ) -> Result<(), String> {
        if self.left || target_display != &target(self.round) || state_epoch < self.last_state_epoch
        {
            return Err("hybrid input crossed leave/display/state barrier".into());
        }
        self.last_state_epoch = state_epoch;
        // Stream control_seq and Datagram motion_seq are intentionally independent.
        // Compare reliable sequence only for state transitions, never across channels.
        let transition = events
            .iter()
            .any(|event| !matches!(event, SharedInputEvent::Motion { .. }));
        if transition && sequence <= self.last_sequence {
            return Err("hybrid reliable sequence regressed".into());
        }
        if transition {
            self.last_sequence = sequence;
        }
        for event in events {
            if let SharedInputEvent::Motion { x, y } = event {
                if !x.is_finite() || !y.is_finite() {
                    return Err("nonfinite hybrid position".into());
                }
                self.pointer = event.clone();
                continue;
            }
            if self.frames >= expected_frames {
                return Err("extra hybrid input transition".into());
            }
            let expected = frame(self.round, self.frames);
            while matches!(
                expected.get(self.hybrid_event_offset),
                Some(SharedInputEvent::Motion { .. })
            ) {
                self.hybrid_event_offset += 1;
            }
            if expected.get(self.hybrid_event_offset) != Some(event) {
                return Err("hybrid state transition content/order mismatch".into());
            }
            let anchor = expected[..self.hybrid_event_offset]
                .iter()
                .rev()
                .find(|event| matches!(event, SharedInputEvent::Motion { .. }))
                .cloned()
                .unwrap_or_else(|| {
                    if self.frames == 0 {
                        entry(self.round)
                    } else {
                        frame(self.round, self.frames - 1)[7].clone()
                    }
                });
            if self.pointer != anchor {
                return Err("hybrid transition applied at incorrect pointer anchor".into());
            }
            self.hybrid_event_offset += 1;
            if self.hybrid_event_offset == expected.len() {
                self.frames += 1;
                self.hybrid_event_offset = 0;
                // Logical fixture event count. Extra transport anchors and snapshots
                // are not original captured events and must not inflate the receipt.
                self.event_count += expected.len() as u64;
            }
        }
        Ok(())
    }
    fn ack(&self, args: &Args, kind: AckKind) -> Ack {
        Ack {
            run_id: args.run_id.clone(),
            kind,
            round: self.round,
            session: self.session.to_string(),
            frames: self.frames,
            event_count: self.event_count,
            last_sequence: self.last_sequence,
        }
    }
}
async fn receiver(
    args: &Args,
    runtime: &RuntimeHandle,
    events: &mut EventStream,
    peer: DeviceId,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + args.timeout;
    let mut current: Option<ReceivedRound> = None;
    let mut completed = 0;
    let mut released = BTreeSet::new();
    let mut ack_sequence = 0;
    let mut confirmed_activation = None;
    let mut ticker = tokio::time::interval(Duration::from_millis(25));
    loop {
        tokio::select! {
            _=ticker.tick()=>confirm_managed_fixture(runtime,peer,&mut confirmed_activation).await?,
            _=tokio::time::sleep_until(deadline)=>return Err(format!("receiver timed out after {completed} rounds")),
            event=events.next()=>match event.ok_or("runtime event stream ended")? {
                RuntimeEvent::SharedInputReceived{peer_id,session_id,target_display,sequence,state_epoch,events}=>{
                    if peer_id!=peer||released.contains(&session_id){return Err("input from an unexpected peer or already released session".into());}
                    let link=verified_link(runtime,peer,Some(session_id))?;
                    if args.hybrid && link.motion_mode.as_deref()!=Some("hybrid") {return Err("receiver expected hybrid mode".into());}
                    if let Some(round)=current.as_mut(){
                        if round.session!=session_id{return Err("another session entered before prior release".into());}
                        let before = round.frames;
                        if args.hybrid { round.receive_hybrid(&target_display,sequence,state_epoch,&events,args.frames)?; }
                        else { round.receive(&target_display,sequence,&events,args.frames)?; }
                        if before<args.frames && round.frames==args.frames{send_ack(runtime,peer,&round.ack(args,AckKind::Frames),&mut ack_sequence).await?;}
                    }else{
                        let round=completed+1;
                        if round>args.rounds||sequence==0||target_display!=target(round)||events!=vec![entry(round)]{return Err("invalid round entry or unexpected input after release".into());}
                        println!("RECEIVE_ENTER round={round} session={session_id} sequence={sequence} context={:?}",link.context);
                        current=Some(ReceivedRound{session:session_id,round,frames:0,event_count:1,last_sequence:sequence,left:false,hybrid_event_offset:0,last_state_epoch:state_epoch,pointer:entry(round)});
                    }
                },
                RuntimeEvent::InputControlReleased{peer_id,session_id,reason} if peer_id==peer=>{
                    if let Some(round)=current.as_mut() && round.session==session_id {
                        if reason=="peer left display" {
                            if round.left||round.frames!=args.frames{return Err("Leave preceded complete receiver evidence".into());}
                            round.left=true;send_ack(runtime,peer,&round.ack(args,AckKind::Left),&mut ack_sequence).await?;
                        }else if reason=="peer released input control" {
                            if !round.left{return Err("Release arrived before Leave".into());}
                            send_ack(runtime,peer,&round.ack(args,AckKind::Released),&mut ack_sequence).await?;
                            released.insert(session_id);completed+=1;current=None;
                        }else{return Err(format!("active session ended unexpectedly: {reason}"));}
                    }
                },
                RuntimeEvent::ClipboardReceived{peer_id,payload} if peer_id==peer&&payload.mime==ACK_MIME=>{
                    let ack=Ack::decode(&payload.data)?;
                    if ack.run_id!=args.run_id||ack.kind!=AckKind::Finish||completed!=args.rounds||current.is_some(){return Err("invalid finish fence or incomplete receiver rounds".into());}
                    if ack.round!=args.rounds||ack.frames!=args.frames||ack.event_count!=1+u64::from(args.frames)*frame(args.rounds,0).len() as u64||!released.iter().any(|session|session.to_string()==ack.session){return Err("finish evidence does not match released receiver sessions".into());}
                send_ack(runtime,peer,&Ack{kind:AckKind::Finished,..ack},&mut ack_sequence).await?;
                tokio::time::sleep(Duration::from_millis(250)).await;
                confirm_managed_fixture(runtime,peer,&mut confirmed_activation).await?;
                println!("RESULT PASS role=receiver rounds={completed} unique_sessions={} strict_sequences=true released_session_rejected=true auto_managed=true persisted_enabled=false persisted_role=Disabled persisted_authorized_controller=none native_input=false",released.len());return Ok(());
                },
                RuntimeEvent::InputDeliveryInvalidated {reason} => {
                    command(runtime,RuntimeCommand::SetInputAvailability {available:false,reason:Some("probe delivery invalidated".into())}).await?;
                    return Err(format!("ordered input delivery invalidated; input revoked: {reason}"));
                },
                RuntimeEvent::InputReceived{..}=>return Err("legacy input unexpectedly reached probe".into()),
                RuntimeEvent::InputControlDenied{reason,..}=>return Err(format!("input denied: {reason}")),
                RuntimeEvent::InputLinkUnavailable{peer_id,reason} if peer_id==peer&&current.is_some()=>return Err(format!("active input link unavailable: {reason}")),
                RuntimeEvent::Error(error)=>return Err(error.to_string()),
                _=>{}
            }
        }
    }
}

async fn execute(args: &Args) -> Result<(), String> {
    let identity_provider = MemoryIdentityProvider::new();
    let identity = identity_provider
        .load_or_create()
        .map_err(|e| e.to_string())?;
    let local = identity.device_id();
    let mut nonce = [0; 32];
    nonce[..16].copy_from_slice(deskfuse_core::RequestId::new().into_uuid().as_bytes());
    nonce[16..].copy_from_slice(deskfuse_core::RequestId::new().into_uuid().as_bytes());
    let hello = PeerHello::signed(&identity, args.name(args.role), args.capabilities(), nonce)
        .map_err(|e| e.to_string())?;
    let transport = QuinnTransport::bind(
        hello,
        QuinnTransportConfig::new(
            args.bind,
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            "deskfuse.local",
            include_bytes!("../../../../resources/deskfuse-transport-cert.der").to_vec(),
            include_bytes!("../../../../resources/deskfuse-transport-key.der").to_vec(),
            include_bytes!("../../../../resources/deskfuse-transport-cert.der").to_vec(),
        ),
    )
    .await
    .map_err(|e| e.to_string())?;
    let discovery = MemoryDiscovery::new();
    if let Some((peer, endpoint)) = &args.peer {
        discovery.push(DiscoveryEvent::PeerHint {
            peer_id: *peer,
            display_name: args.name(args.role.other()),
            capabilities: args.capabilities(),
            routes: vec![RouteCandidate::new(
                endpoint.clone(),
                RouteSource::Manual,
                RouteHealth::Unknown,
                args.capabilities(),
            )],
        });
    }
    let runtime = RuntimeBuilder::new()
        .config(RuntimeConfig {
            display_name: args.name(args.role),
            capabilities: args.capabilities(),
            event_capacity: 2048,
            ..Default::default()
        })
        .with_identity_provider(identity_provider)
        .with_trust_store(MemoryTrustStore::new())
        .with_store(MemoryRuntimeStore::new())
        .with_discovery(discovery.clone())
        .with_control_transport(transport)
        .build()
        .map_err(|e| e.to_string())?;
    println!("DEVICE_ID={local}");
    println!(
        "LISTEN={} RUN_ID={} ROLE={} NATIVE_INPUT=false STORE=memory",
        args.bind,
        args.run_id,
        args.role.label()
    );
    let mut events = combined_events(&runtime);
    let result = async {
        if args.expect_rejected {
            return expect_rejection(args, &runtime, &mut events).await;
        }
        let peer = bootstrap(args, &runtime, &mut events, &discovery).await?;
        if args.metadata_only {
            return metadata_only(args, &runtime, &mut events, peer).await;
        }
        configure(args, &runtime, peer).await?;
        match args.role {
            Role::Controller => controller(args, &runtime, &mut events, peer).await,
            Role::Receiver => receiver(args, &runtime, &mut events, peer).await,
        }
    }
    .await;
    let _ = command(
        &runtime,
        RuntimeCommand::SetInputAvailability {
            available: false,
            reason: Some("probe ended".into()),
        },
    )
    .await;
    let shutdown = runtime.shutdown_and_wait().await.map_err(|e| e.to_string());
    result.and(shutdown)
}
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), String> {
    let raw = std::env::args().skip(1).collect::<Vec<_>>();
    if raw.iter().any(|arg| arg == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    let args = Args::parse(&raw)?;
    let outcome = tokio::time::timeout(args.timeout + Duration::from_secs(10), execute(&args))
        .await
        .map_err(|_| "overall probe timeout".to_owned())?;
    if let Err(error) = &outcome {
        eprintln!("RESULT FAIL role={} error={error}", args.role.label());
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(role: &str) -> Args {
        let mut raw = vec![
            "--role".into(),
            role.into(),
            "--run-id".into(),
            "isolated-test".into(),
        ];
        if role == "controller" {
            raw.extend([
                "--peer".into(),
                format!(
                    "{}@10.0.0.88:43424",
                    DeviceId::from_public_key([2; 32]).unwrap()
                ),
            ]);
        }
        Args::parse(&raw).unwrap()
    }
    #[test]
    fn automatic_approval_is_scoped_to_test_role_run_and_device() {
        let args = args("receiver");
        let id = DeviceId::from_public_key([1; 32]).unwrap();
        assert!(args.approves("deskfuse-input-probe:isolated-test:controller", id, None));
        assert!(!args.approves("DeskFuse", id, None));
        assert!(!args.approves("deskfuse-input-probe:other:controller", id, None));
        assert!(!args.approves("deskfuse-input-probe:isolated-test:receiver", id, None));
        assert!(!args.approves(
            "deskfuse-input-probe:isolated-test:controller",
            id,
            Some(DeviceId::from_public_key([2; 32]).unwrap())
        ));
    }
    #[test]
    fn receiver_checks_content_monotonic_sequence_and_leave_boundary() {
        let mut round = ReceivedRound {
            session: InputSessionId::new(),
            round: 1,
            frames: 0,
            event_count: 1,
            last_sequence: 1,
            left: false,
            hybrid_event_offset: 0,
            last_state_epoch: 1,
            pointer: entry(1),
        };
        assert!(round.receive(&target(1), 4, &frame(1, 0), 32).is_ok());
        assert!(round.receive(&target(1), 4, &frame(1, 1), 32).is_err());
        assert!(round.receive(&target(2), 5, &frame(1, 1), 32).is_err());
        let mut wrong = frame(1, 1);
        wrong.pop();
        assert!(round.receive(&target(1), 5, &wrong, 32).is_err());
        round.left = true;
        assert!(round.receive(&target(1), 6, &frame(1, 1), 32).is_err());
    }
    #[test]
    fn hybrid_receipts_verify_anchors_without_comparing_motion_and_control_sequences() {
        let mut round = ReceivedRound {
            session: InputSessionId::new(),
            round: 1,
            frames: 0,
            event_count: 1,
            last_sequence: 1,
            left: false,
            hybrid_event_offset: 0,
            last_state_epoch: 1,
            pointer: entry(1),
        };
        let fixture = frame(1, 0);
        let mut down = vec![entry(1)];
        down.extend_from_slice(&fixture[..7]);
        round.receive_hybrid(&target(1), 10, 10, &down, 16).unwrap();
        // A lower motion sequence is normal: it does not advance control_seq.
        round
            .receive_hybrid(&target(1), 2, 10, &fixture[7..8], 16)
            .unwrap();
        round
            .receive_hybrid(&target(1), 11, 11, &fixture[7..], 16)
            .unwrap();
        assert_eq!(round.frames, 1);
        assert_eq!(round.event_count, 10);
        assert_eq!(round.last_sequence, 11);
        assert!(
            round
                .receive_hybrid(&target(1), 20, 9, &fixture[7..8], 16)
                .is_err()
        );
        let next = frame(1, 1);
        let mut bad = vec![entry(1)];
        bad.extend_from_slice(&next[..7]);
        assert!(round.receive_hybrid(&target(1), 12, 12, &bad, 16).is_err());
    }
    #[test]
    fn hybrid_capability_is_opt_in_and_input_edges_have_one_source() {
        let default = args("receiver");
        assert!(
            !default
                .capabilities()
                .contains(Capabilities::INPUT_MOTION_DATAGRAM_V1)
        );
        let opted =
            Args::parse(&["--role", "receiver", "--run-id", "test", "--hybrid"].map(String::from))
                .unwrap();
        assert!(
            opted
                .capabilities()
                .contains(Capabilities::INPUT_MOTION_DATAGRAM_V1)
        );
        assert!(is_ordered_input_event(
            &RuntimeEvent::InputDeliveryInvalidated {
                reason: "fixture".into()
            }
        ));
        assert!(!is_ordered_input_event(
            &RuntimeEvent::InputSharingSettingsChanged(InputSharingSettings::default())
        ));
    }
    #[test]
    fn ack_round_trip_carries_receiver_evidence() {
        let ack = Ack {
            run_id: "test".into(),
            kind: AckKind::Frames,
            round: 3,
            session: InputSessionId::new().to_string(),
            frames: 32,
            event_count: 289,
            last_sequence: 40,
        };
        assert_eq!(Ack::decode(&ack.encode()).unwrap(), ack);
        assert!(Ack::decode(b"bad|ack").is_err());
    }
    #[test]
    fn final_fence_response_has_an_independent_payload() {
        let request = Ack {
            run_id: "test".into(),
            kind: AckKind::Finish,
            round: 3,
            session: InputSessionId::new().to_string(),
            frames: 32,
            event_count: 867,
            last_sequence: 40,
        };
        let response = Ack {
            kind: AckKind::Finished,
            ..request.clone()
        };
        assert_ne!(request.encode(), response.encode());
        assert_eq!(Ack::decode(&response.encode()).unwrap(), response);
    }
    #[test]
    fn rejection_only_mode_is_controller_only_and_pins_its_target() {
        let mut raw = vec![
            "--role".into(),
            "controller".into(),
            "--run-id".into(),
            "ui-reject".into(),
            "--peer".into(),
            format!("{}@10.0.0.44:42424", DeviceId::from_bytes([2; 32])),
            "--expect-rejected".into(),
        ];
        assert!(Args::parse(&raw).unwrap().expect_rejected);
        raw[1] = "receiver".into();
        assert!(Args::parse(&raw).is_err());
    }
    #[test]
    fn metadata_only_mode_keeps_both_sides_disabled_and_requires_a_distinct_mode() {
        assert!(!args("controller").metadata_only);
        let mut raw = vec![
            "--role".into(),
            "receiver".into(),
            "--run-id".into(),
            "metadata-test".into(),
            "--metadata-only".into(),
        ];
        assert!(Args::parse(&raw).unwrap().metadata_only);
        raw[1] = "controller".into();
        let peer = DeviceId::from_bytes([2; 32]);
        raw.extend(["--peer".into(), format!("{peer}@10.0.0.88:43424")]);
        assert!(Args::parse(&raw).unwrap().metadata_only);
        raw.push("--expect-rejected".into());
        assert!(Args::parse(&raw).is_err());
        let controller = metadata_settings(Role::Controller, peer);
        assert!(!controller.enabled);
        assert_eq!(controller.role, InputShareRole::Controller);
        assert_eq!(controller.participants, vec![peer]);
        assert_eq!(
            metadata_settings(Role::Receiver, peer),
            InputSharingSettings::default()
        );
        assert_eq!(
            input_fixture_settings(Role::Receiver, peer),
            InputSharingSettings::default()
        );
        let automatic_controller = input_fixture_settings(Role::Controller, peer);
        assert!(automatic_controller.enabled);
        assert_eq!(automatic_controller.participants, vec![peer]);
    }
    #[test]
    fn metadata_evidence_rejects_control_sessions_and_requires_both_exact_topologies() {
        let args = args("controller");
        let peer = args.peer.as_ref().unwrap().0;
        let mut snapshot = InputSharingSnapshot {
            settings: metadata_settings(args.role, peer),
            local_topology: Some(topology(args.role)),
            remote_topologies: std::collections::BTreeMap::from([(
                peer,
                topology(args.role.other()),
            )]),
            links: vec![InputPeerStatus {
                motion_mode: None,
                motion_fallback_reason: None,
                peer_id: peer,
                context: Some(ConnectionContext {
                    session_id: deskfuse_core::TransportSessionId::new(),
                    local_addr: "10.0.0.44:43424".parse().unwrap(),
                    remote_addr: "10.0.0.88:43424".parse().unwrap(),
                    interface_name: Some("en0".into()),
                    interface_kind: InterfaceKind::WiFi,
                    scope: LinkScope::Lan,
                }),
                ready: false,
                reason: Some("Input sharing disabled".into()),
                control_session: None,
                lease_revision: 0,
            }],
            ..Default::default()
        };
        assert!(metadata_evidence(&args, &snapshot, peer).unwrap().is_some());
        snapshot.links[0].control_session = Some(InputSessionId::new());
        assert!(metadata_evidence(&args, &snapshot, peer).is_err());
        snapshot.links[0].control_session = None;
        snapshot.remote_topologies.clear();
        assert!(metadata_evidence(&args, &snapshot, peer).unwrap().is_none());
    }
    #[test]
    fn metadata_receipts_are_role_bound_and_distinct_from_input_fences() {
        let controller = args("controller");
        let receiver = args("receiver");
        let ready = metadata_ack(&receiver, AckKind::MetadataReady);
        assert_eq!(Ack::decode(&ready.encode()).unwrap(), ready);
        assert!(validate_metadata_ack(&controller, &ready).is_ok());
        assert!(validate_metadata_ack(&receiver, &ready).is_err());
        assert!(
            validate_metadata_ack(&controller, &metadata_ack(&receiver, AckKind::Finished))
                .is_err()
        );
        assert_ne!(
            metadata_ack(&controller, AckKind::MetadataFinish).encode(),
            metadata_ack(&receiver, AckKind::MetadataFinished).encode()
        );
    }
    #[test]
    fn bounded_args_and_valid_synthetic_fixtures() {
        let args = args("receiver");
        assert_eq!(args.bind.port(), 43424);
        assert_eq!(args.rounds, 3);
        assert!(topology(Role::Receiver).validate().is_ok());
        for event in frame(32, 255) {
            event.validate().unwrap();
        }
        assert!(
            Args::parse(&[
                "--role".into(),
                "controller".into(),
                "--run-id".into(),
                "test".into()
            ])
            .is_err()
        );
    }
}
