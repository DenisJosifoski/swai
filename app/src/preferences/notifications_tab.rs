//! SWAI — Notifications Preferences Tab.
//!
//! Desktop notification and log-following behavior.

use adw::prelude::*;
use adw::{PreferencesGroup, PreferencesPage, SwitchRow};

use swai_core::config::Config;

/// Widget handles for the Notifications tab.
pub struct NotificationsWidgets {
    pub enable_notifications_switch: SwitchRow,
    pub notify_on_switch_switch: SwitchRow,
    pub auto_follow_switch: SwitchRow,
}

/// Build the Notifications preferences page.
pub fn build_notifications_tab(config: &Config) -> (PreferencesPage, NotificationsWidgets) {
    let page = PreferencesPage::new();
    page.set_title("Notifications");

    let group = PreferencesGroup::new();
    group.set_title("Notification Settings");

    let enable_switch = add_enable_notifications_row(&group, config);
    let notify_switch = add_notify_on_switch_row(&group, config);
    let auto_follow_switch = add_auto_follow_logs_row(&group, config);

    page.add(&group);

    let widgets = NotificationsWidgets {
        enable_notifications_switch: enable_switch,
        notify_on_switch_switch: notify_switch,
        auto_follow_switch,
    };

    (page, widgets)
}

/// Add a switch row for enabling desktop notifications.
pub fn add_enable_notifications_row(parent: &PreferencesGroup, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Enable desktop notifications")
        .build();

    let enable = config.enable_notifications();
    row.set_active(enable);

    parent.add(&row);
    row
}

/// Add a switch row for notifying on model switch.
pub fn add_notify_on_switch_row(parent: &PreferencesGroup, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder().title("Notify on model switch").build();

    let notify = config.notify_on_switch();
    row.set_active(notify);

    parent.add(&row);
    row
}

/// Add a switch row for auto-following active model in logs.
pub fn add_auto_follow_logs_row(parent: &PreferencesGroup, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Auto-follow active model in logs")
        .build();

    let auto_follow = config.auto_follow_logs();
    row.set_active(auto_follow);

    parent.add(&row);
    row
}
