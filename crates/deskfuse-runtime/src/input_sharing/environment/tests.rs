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
    assert_eq!(actor.snapshot.environment.selected, None);
    assert!(actor.snapshot.environment.local_suspended);
    assert!(!actor.snapshot.effective_settings().enabled);
    settle(&mut actor, Some("office"));
    assert_eq!(actor.snapshot.environment.selected, Some(peer));
    assert!(!actor.snapshot.effective_settings().enabled);
    settle(&mut actor, None);
    assert!(actor.snapshot.environment.local_suspended);
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
    assert!(actor.snapshot.environment.local_suspended);
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

    assert!(!actor.refresh_environment_scene());
    assert_eq!(actor.snapshot.scenes[0].settings.placements, home);
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
fn wifi_change_waits_for_stability_and_active_input_to_end() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.environment.rules.insert("home".into(), local);
    settle(&mut actor, Some("home"));
    actor.observe_environment(Some("unknown".into()));
    actor.tick_environment();
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    actor.request(peer, "display".into());
    assert!(!actor.leases.is_empty());
    for lease in actor.leases.values_mut() {
        lease.entered = true;
    }
    settle(&mut actor, Some("unknown"));
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    for lease in actor.leases.values_mut() {
        lease.entered = false;
    }
    actor.tick_environment();
    assert_eq!(actor.snapshot.environment.selected, None);
    assert!(actor.leases.is_empty());
}

#[tokio::test]
async fn wifi_cannot_save_a_rule_for_the_old_network_during_transition() {
    let (mut actor, peer, _, _, _) = setup(true);
    let local = actor.network.borrow().snapshot.local_device_id;
    actor.environment.rules.insert("home".into(), peer);
    settle(&mut actor, Some("home"));
    actor.observe_environment(Some("office".into()));
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
    assert!(actor.snapshot.environment.local_suspended);
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
            seen: Instant::now(),
        },
    );
    actor.tick_environment();
    assert_eq!(actor.snapshot.environment.selected, Some(local));
    assert!(!actor.snapshot.environment.local_suspended);
    assert!(actor.snapshot.environment.conflict);
}
