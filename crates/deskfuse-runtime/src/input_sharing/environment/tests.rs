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
