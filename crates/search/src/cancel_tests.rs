use super::*;
#[test]
fn active_work_is_cancelled_and_completion_ids_cannot_be_reused() {
    let registry = Registry::default();
    let guard = registry.register("first").unwrap();
    assert!(!guard.token.is_cancelled());
    assert!(registry.register("first").is_err());
    registry.cancel("first").unwrap();
    assert!(guard.token.is_cancelled());
    drop(guard);
    assert!(registry.register("first").is_err());
    let completed = registry.register("complete").unwrap();
    drop(completed);
    assert!(registry.register("complete").is_err());
}
#[test]
fn pre_registration_cancel_is_consumed_without_starting_work() {
    let registry = Registry::default();
    registry.cancel("race").unwrap();
    assert_eq!(registry.register("race").err().unwrap().code(),"request_cancelled");
}

#[test]
fn full_registry_refuses_new_ids_without_evicting_cancellation_tombstones_or_active_work() {
    let registry = Registry::default();
    let active = registry.register("active").unwrap();
    for index in 0..4095 { registry.cancel(&format!("cancelled-{index}")).unwrap(); }
    assert_eq!(registry.register("overflow-work").err().unwrap().status(),429);
    assert_eq!(registry.cancel("overflow-cancel").unwrap_err().status(),429);
    assert_eq!(registry.register("cancelled-0").err().unwrap().code(),"request_cancelled");
    assert!(!active.token.is_cancelled());
    registry.cancel("active").unwrap();
    assert!(active.token.is_cancelled(),"a full registry must still allow cancellation of registered work");
    registry.cancel("cancelled-0").unwrap();
}
