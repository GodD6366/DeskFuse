use super::*;
use crate::input_sharing::tests::setup;

fn settle(actor: &mut InputActor, ssid: Option<&str>) {
    actor.observe_environment(ssid.map(str::to_owned));
    actor.environment.pending = Some((
        ssid.map(str::to_owned),
        Instant::now() - Duration::from_secs(11),
    ));
    actor.tick_environment();
}

fn bind_layout(
    actor: &mut InputActor,
    peer: DeviceId,
    ssid: &str,
    x: f64,
) -> Vec<DisplayPlacementSetting> {
    let mut scene = InputSharingScene::new(ssid.into(), actor.snapshot.settings.clone()).unwrap();
    scene.ssid = Some(ssid.into());
    scene.settings.placements = vec![DisplayPlacementSetting {
        display: DisplayKey {
            device_id: peer,
            display_id: DisplayId::from("display"),
        },
        x,
        y: 0.0,
    }];
    let placements = scene.settings.placements.clone();
    actor.snapshot.scenes.push(scene);
    placements
}

#[test]
fn wifi_diagnostics_explain_restore_and_deduplicate_unchanged_observations() {
    let (mut actor, peer, _, _, _) = setup(true);
    let mut events = actor.events.subscribe();
    bind_layout(&mut actor, peer, "office", -1000.0);
    settle(&mut actor, Some("office"));
    actor.observe_environment(Some("office".into()));
    actor.environment.last_sent = None;
    actor.tick_environment();
    actor.apply_environment_layout(Some("missing"));

    let mut diagnostics = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let RuntimeEvent::InputDiagnostic { stage, detail } = event {
            diagnostics.push((stage, detail));
        }
    }
    assert_eq!(
        diagnostics
            .iter()
            .filter(|(stage, _)| *stage == "wifi_observed")
            .count(),
        1,
    );
    assert!(diagnostics.iter().any(|(stage, detail)| {
        *stage == "wifi_environment_state"
            && detail.contains("effective=Some(\"office\")")
            && detail.contains("layout_pending=false")
    }));
    for outcome in ["applied", "no_saved_scene"] {
        assert!(diagnostics.iter().any(|(stage, detail)| {
            *stage == "wifi_layout_restore" && detail.contains(&format!("outcome={outcome}"))
        }));
    }
}

#[test]
fn wifi_rules_enter_leave_and_restore_controller_without_rewriting_settings() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    let settings = actor.snapshot.settings.clone();
    actor.environment.rules.insert("home".into(), local);
    actor.environment.rules.insert("office".into(), peer);
    settle(&mut actor, Some("home"));
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    assert!(actor.snapshot.effective_settings().enabled);
    settle(&mut actor, Some("unknown"));
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    assert!(!actor.snapshot.environment.local_suspended);
    assert!(actor.snapshot.effective_settings().enabled);
    settle(&mut actor, Some("office"));
    assert_eq!(actor.snapshot.environment.selected, Some(peer));
    assert!(!actor.snapshot.effective_settings().enabled);
    settle(&mut actor, None);
    assert!(!actor.snapshot.environment.local_suspended);
    settle(&mut actor, Some("home"));
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    assert!(!actor.snapshot.environment.local_suspended);
    assert_eq!(actor.snapshot.settings, settings);
}

#[test]
fn wifi_layout_restores_after_disconnected_transition_resumes_controller() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.environment.rules.insert("home".into(), local);
    actor.environment.rules.insert("office".into(), local);
    let expected = bind_layout(&mut actor, peer, "office", -1000.0);
    let original = actor.snapshot.settings.clone();

    settle(&mut actor, Some("home"));
    settle(&mut actor, None);
    assert!(!actor.snapshot.environment.local_suspended);
    settle(&mut actor, Some("office"));

    assert!(!actor.snapshot.environment.local_suspended);
    assert_eq!(actor.snapshot.settings.placements, expected);
    assert_eq!(actor.snapshot.settings.enabled, original.enabled);
    assert_eq!(actor.snapshot.settings.role, original.role);
    assert_eq!(actor.snapshot.settings.participants, original.participants);
    assert_eq!(
        actor.snapshot.settings.authorized_controller,
        original.authorized_controller
    );
    // A background save must not replace the office scene with the old layout.
    actor.refresh_environment_scene();
    assert_eq!(actor.snapshot.scenes[0].settings.placements, expected);
}

#[test]
fn wifi_layout_restores_added_devices_and_their_activation_intents() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    let office_peer = DeviceId::from_public_key([7; 32]).unwrap();
    actor.environment.rules.insert("home".into(), local);
    actor.environment.rules.insert("office".into(), local);
    bind_layout(&mut actor, peer, "home", 1000.0);
    bind_layout(&mut actor, office_peer, "office", -1000.0);
    actor.snapshot.scenes[1].settings.participants = vec![office_peer];

    settle(&mut actor, Some("office"));
    assert_eq!(actor.snapshot.settings.participants, vec![office_peer]);
    assert!(actor.controller_intents.contains_key(&office_peer));
    assert!(!actor.controller_intents.contains_key(&peer));

    settle(&mut actor, Some("home"));
    assert_eq!(actor.snapshot.settings.participants, vec![peer]);
    assert!(actor.controller_intents.contains_key(&peer));
    assert!(!actor.controller_intents.contains_key(&office_peer));
    assert!(actor.snapshot.settings.enabled);
    // Adding a device at home is remembered for home only.
    actor.snapshot.settings.participants.push(office_peer);
    assert!(actor.refresh_environment_scene());
    assert_eq!(
        actor.snapshot.scenes[0].settings.participants,
        vec![peer, office_peer]
    );
    assert_eq!(
        actor.snapshot.scenes[1].settings.participants,
        vec![office_peer]
    );
}

#[tokio::test]
async fn wifi_layout_waits_for_local_handover_without_overwriting_saved_scene() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.environment.rules.insert("home".into(), local);
    actor.environment.rules.insert("office".into(), peer);
    let home = bind_layout(&mut actor, peer, "home", 1000.0);
    let office = bind_layout(&mut actor, peer, "office", -1000.0);
    settle(&mut actor, Some("home"));
    assert_eq!(actor.snapshot.settings.placements, home);
    settle(&mut actor, Some("office"));
    assert!(actor.snapshot.environment.local_suspended);
    assert_eq!(actor.snapshot.settings.placements, home);
    assert!(!actor.refresh_environment_scene());
    assert_eq!(actor.snapshot.scenes[1].settings.placements, office);

    actor.choose_environment_controller(local, false).await;
    assert!(!actor.snapshot.environment.local_suspended);
    assert_eq!(actor.snapshot.settings.placements, office);
    // Restore once: subsequent ticks must not undo a user's layout edit.
    actor.snapshot.settings.placements[0].x = -2000.0;
    actor.tick_environment();
    assert_eq!(actor.snapshot.settings.placements[0].x, -2000.0);
}

#[test]
fn wifi_transition_does_not_save_new_topology_into_previous_network_scene() {
    let (mut actor, peer, _, _, _) = setup(true);
    let home = bind_layout(&mut actor, peer, "home", 1000.0);
    settle(&mut actor, Some("home"));
    actor.observe_environment(Some("office".into()));
    actor.tick_environment();
    actor.snapshot.settings.placements[0].x = -1000.0;

    assert!(actor.refresh_environment_scene());
    assert_eq!(actor.snapshot.scenes[0].settings.placements, home);
    let office = actor
        .snapshot
        .scenes
        .iter()
        .find(|s| s.ssid.as_deref() == Some("office"))
        .unwrap();
    assert_eq!(office.settings.placements[0].x, -1000.0);
}

#[test]
fn wifi_authorization_and_matching_rule_never_enable_the_sharing_switch() {
    let (mut actor, _, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.snapshot.settings.enabled = false;
    actor.environment.rules.insert("home".into(), local);
    settle(&mut actor, Some("home"));
    assert!(!actor.snapshot.settings.enabled);
    assert!(!actor.snapshot.effective_settings().enabled);
}

#[test]
fn wifi_change_waits_only_for_active_input_to_end() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.environment.rules.insert("home".into(), local);
    settle(&mut actor, Some("home"));
    actor.environment.rules.insert("office".into(), peer);
    actor.request(peer, "display".into());
    assert!(!actor.leases.is_empty());
    for lease in actor.leases.values_mut() {
        lease.entered = true;
    }
    actor.observe_environment(Some("office".into()));
    actor.tick_environment();
    assert!(actor.environment.pending.is_some());
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    for lease in actor.leases.values_mut() {
        lease.entered = false;
    }
    actor.tick_environment();
    assert_eq!(actor.snapshot.environment.selected, Some(peer));
    assert!(actor.leases.is_empty());
}

#[tokio::test]
async fn wifi_cannot_save_a_rule_for_the_old_network_during_transition() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.environment.rules.insert("home".into(), peer);
    settle(&mut actor, Some("home"));
    actor.observe_environment(None);
    actor.tick_environment();
    assert!(actor.environment.pending.is_some());
    actor.choose_environment_controller(local, true).await;
    assert_eq!(actor.environment.rules.get("home"), Some(&peer));
    assert!(!actor.environment.rules.contains_key("office"));
    assert!(actor.environment.temporary.is_none());
}

#[tokio::test]
async fn wifi_temporary_override_expires_on_the_next_network() {
    let (mut actor, _, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.environment.rules.insert("home".into(), local);
    settle(&mut actor, Some("unknown"));
    actor.choose_environment_controller(local, false).await;
    assert!(!actor.snapshot.environment.local_suspended);
    // 静默记忆：本次生效的主控自动成为当前 Wi-Fi 的保存规则。
    assert_eq!(actor.environment.rules.get("unknown"), Some(&local));
    settle(&mut actor, Some("another"));
    assert!(!actor.snapshot.environment.local_suspended);
    assert!(actor.environment.temporary.is_none());
}

#[test]
fn wifi_effective_controller_is_remembered_silently() {
    let (mut actor, _, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    settle(&mut actor, Some("home"));
    assert_eq!(actor.environment.rules.get("home"), Some(&local));
    assert!(actor.environment.dirty);
}

#[test]
fn wifi_rule_for_local_controller_is_cleared_when_controller_turns_off() {
    let (mut actor, _, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.snapshot.settings.enabled = false;
    actor.snapshot.settings.role = InputShareRole::Disabled;
    actor.environment.rules.insert("home".into(), local);
    settle(&mut actor, Some("home"));
    assert!(!actor.environment.rules.contains_key("home"));
    assert!(actor.environment.dirty);
}

#[test]
fn remote_wifi_vote_cannot_override_local_controller_without_local_rule() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    settle(&mut actor, Some("home"));
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    actor.environment.peers.insert(
        peer,
        Presence {
            controller: true,
            ssid: Some("home".into()),
            preferred: Some(peer),
            claimed_at_ms: None,
            seen: Instant::now(),
        },
    );
    actor.tick_environment();
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    assert!(!actor.snapshot.environment.local_suspended);
    assert!(actor.snapshot.environment.conflict);
}

fn claim_presence(actor: &mut InputActor, peer: DeviceId, claimed_at_ms: u64) {
    actor.environment.peers.insert(
        peer,
        Presence {
            controller: true,
            ssid: Some("home".into()),
            preferred: Some(peer),
            claimed_at_ms: Some(claimed_at_ms),
            seen: Instant::now(),
        },
    );
}

#[test]
fn later_controller_claim_demotes_the_earlier_controller() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.controller_claimed_at_ms = Some(1_000);
    settle(&mut actor, Some("home"));
    // 对端更晚被设为主控：本机自动让位，不再形成双主控僵局。
    claim_presence(&mut actor, peer, 2_000);
    actor.tick_environment();
    let environment = &actor.snapshot.environment;
    assert_eq!(environment.selected, Some(peer));
    assert!(environment.local_suspended);
    assert!(!environment.conflict);
    assert!(!actor.snapshot.effective_settings().enabled);
    // 本机随后再次被设为主控，重新夺回。
    actor.controller_claimed_at_ms = Some(3_000);
    actor.tick_environment();
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    assert!(!actor.snapshot.environment.local_suspended);
}

#[test]
fn earlier_remote_claim_cannot_take_over_the_newer_controller() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.controller_claimed_at_ms = Some(5_000);
    settle(&mut actor, Some("home"));
    claim_presence(&mut actor, peer, 4_000);
    actor.tick_environment();
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    assert!(!actor.snapshot.environment.local_suspended);
    assert!(!actor.snapshot.environment.conflict);
}

#[test]
fn snapshot_separates_local_observation_from_peer_reported_ssids() {
    let (mut actor, peer, context, transport, _) = setup(false);
    transport.capabilities.lock().unwrap().insert(
        context.session_id,
        Capabilities::INPUT_SHARING
            | Capabilities::INPUT_ACTIVATION
            | Capabilities::INPUT_ENVIRONMENT,
    );
    // 本机读不到 SSID（缺少定位授权或走有线）时，场景判定可采用对端上报的唯一标识，
    // 但快照必须如实保留本机未读取到的事实，界面才能区分两者。
    actor.observe_environment(None);
    crate::input_sharing::tests::receive(
        &mut actor,
        peer,
        &context,
        InputShareMessage::ControllerPresence {
            controller: true,
            ssid: Some("office".into()),
            preferred: Some(peer),
            claimed_at_ms: None,
        },
    );
    actor.tick_environment();
    let environment = &actor.snapshot.environment;
    assert_eq!(environment.observed_ssid, None);
    assert_eq!(
        environment.peer_ssids,
        BTreeMap::from([(peer, "office".to_owned())])
    );

    settle(&mut actor, Some("home"));
    let environment = &actor.snapshot.environment;
    assert_eq!(environment.observed_ssid.as_deref(), Some("home"));
    assert_eq!(environment.ssid.as_deref(), Some("home"));
    assert_eq!(
        environment.peer_ssids.get(&peer).map(String::as_str),
        Some("office")
    );
}

#[tokio::test]
async fn forgetting_a_peer_removes_every_input_sharing_reference() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    let placements = bind_layout(&mut actor, peer, "office", -1000.0);
    actor.snapshot.settings.placements = placements;
    actor.environment.rules.insert("office".into(), peer);
    actor.environment.rules.insert("home".into(), local);
    assert!(actor.snapshot.settings.participants.contains(&peer));
    assert!(actor.controller_intents.contains_key(&peer));

    actor
        .command(RuntimeCommand::ForgetInputPeer { peer_id: peer })
        .await;

    let settings = &actor.snapshot.settings;
    assert!(!settings.participants.contains(&peer));
    assert!(
        settings
            .placements
            .iter()
            .all(|p| p.display.device_id != peer)
    );
    assert!(!actor.controller_intents.contains_key(&peer));
    assert!(actor.snapshot.scenes.iter().all(|scene| {
        !scene.settings.participants.contains(&peer)
            && scene
                .settings
                .placements
                .iter()
                .all(|p| p.display.device_id != peer)
    }));
    // 指向已忘记设备的主控规则被删除，其他网络的规则保留。
    assert_eq!(
        actor.environment.rules,
        BTreeMap::from([("home".to_owned(), local)])
    );
    let persisted = actor.store.get_input_activation_policy().unwrap().unwrap();
    assert!(!persisted.controller_intents.contains_key(&peer));
    assert!(persisted.environment_rules.values().all(|id| *id != peer));
}

#[tokio::test]
async fn forgetting_an_unreferenced_peer_leaves_settings_untouched() {
    let (mut actor, _, _, _, _) = setup(true);
    let before = actor.snapshot.settings.clone();
    let stranger = DeviceId::from_public_key([42; 32]).unwrap();
    actor
        .command(RuntimeCommand::ForgetInputPeer { peer_id: stranger })
        .await;
    assert_eq!(actor.snapshot.settings, before);
}

#[test]
fn local_wifi_applies_immediately_without_blocking_layout() {
    let (mut actor, peer, _, _, _) = setup(true);
    let expected = bind_layout(&mut actor, peer, "home", -1000.0);
    actor.observe_environment(Some("home".into()));
    actor.tick_environment();
    assert_eq!(actor.snapshot.environment.ssid.as_deref(), Some("home"));
    assert!(!actor.environment_layout_pending());
    assert!(actor.snapshot.can_configure_layout());
    assert_eq!(actor.snapshot.settings.placements, expected);
}

#[test]
fn wired_startup_with_wifi_rules_keeps_controller_and_layout() {
    let (mut actor, peer, _, _, _) = setup(true);
    actor.environment.rules.insert("office".into(), peer);
    let settings = actor.snapshot.settings.clone();
    actor.observe_environment(None);
    actor.tick_environment();
    assert!(actor.snapshot.can_configure_layout());
    assert!(!actor.snapshot.environment.local_suspended);
    assert_eq!(actor.snapshot.settings, settings);
    assert!(!actor.environment_layout_pending());
}
