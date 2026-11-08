use super::{update_notice, update_screen_waits, update_watch_due, UpdateNotice, UPDATE_WATCH};
use reborn_core::PlatformTask::{Refresh, ReportSend, UpdateApply, UpdateStage};

#[test]
fn the_update_screen_never_stands_without_an_update_task() {
    // Update Now while a status observation ran: the stage task was
    // dropped and the screen stood at step 1 until a restart.
    assert!(update_screen_waits(true, false, None, None));
    assert!(update_screen_waits(true, false, Some(Refresh), None));
    assert!(update_screen_waits(true, false, Some(ReportSend), None));
    // Queued behind another task, running, or restarting to install: fine.
    assert!(!update_screen_waits(
        true,
        false,
        Some(Refresh),
        Some(UpdateStage)
    ));
    assert!(!update_screen_waits(true, false, Some(UpdateStage), None));
    assert!(!update_screen_waits(true, false, Some(UpdateApply), None));
    assert!(!update_screen_waits(true, true, None, None));
    assert!(!update_screen_waits(false, false, None, None));
}

#[test]
fn an_update_is_announced_once_per_version_and_only_when_asked() {
    // Y2Reborn #97: opt-in notifications; a restart or a new Wi-Fi connection does not repeat one.
    use reborn_core::{platform::UpdatePhase, Screen, Settings};
    let available = |v: &str| UpdatePhase::Available { version: v.into() };
    let mut settings = Settings::default();
    assert!(!settings.update_notifications);
    assert_eq!(
        update_notice(&settings, &available("9.3.19"), Screen::Home),
        UpdateNotice::None
    );
    settings.update_notifications = true;
    assert_eq!(
        update_notice(&settings, &available("9.3.19"), Screen::Home),
        UpdateNotice::Show("9.3.19".into())
    );
    // On Software Update it is on screen already: kept without a toast.
    assert_eq!(
        update_notice(&settings, &available("9.3.19"), Screen::Update),
        UpdateNotice::Record("9.3.19".into())
    );
    settings.update_announced = "9.3.19".into();
    assert_eq!(
        update_notice(&settings, &available("9.3.19"), Screen::Home),
        UpdateNotice::None
    );
    assert_eq!(
        update_notice(&settings, &available("9.3.20"), Screen::NowPlaying),
        UpdateNotice::Show("9.3.20".into())
    );
    for phase in [
        UpdatePhase::UpToDate,
        UpdatePhase::Unknown,
        UpdatePhase::Checking,
    ] {
        assert_eq!(
            update_notice(&settings, &phase, Screen::Home),
            UpdateNotice::None
        );
    }
}

#[test]
fn with_notifications_on_any_screen_looks_for_the_daily_check_while_online() {
    // The daily check runs once Wi-Fi is online; before, only Settings pages observed the
    // platform, so its answer reached Reborn only when Settings was opened (Y2Reborn #97).
    use std::time::Duration;
    let mut settings = reborn_core::Settings::default();
    assert!(!update_watch_due(&settings, true, UPDATE_WATCH));
    settings.update_notifications = true;
    assert!(update_watch_due(&settings, true, UPDATE_WATCH));
    assert!(!update_watch_due(&settings, false, UPDATE_WATCH));
    assert!(!update_watch_due(
        &settings,
        true,
        UPDATE_WATCH - Duration::from_secs(1)
    ));
    assert!(UPDATE_WATCH <= Duration::from_secs(300));
}
