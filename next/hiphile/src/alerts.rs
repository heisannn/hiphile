use anti_webui_md3e::components::{Snackbar, Snackbars};
use hiphile_core::alerts::{Alert, AlertKind, Alerts};
use hiphile_core::navigation::Destination;
use next_host::notices;

/// Tells in `snackbars` each of `alerts`, those told before first, for as long as the current
/// owner lives, except those the screen of the destination `on_show` tells of itself.
pub(crate) fn tell_alerts(
    alerts: Alerts,
    snackbars: Snackbars,
    on_show: impl Fn() -> Option<Destination> + 'static,
) {
    let Alerts { earlier, later, .. } = alerts;
    let tell = move |alert: Alert| {
        if alert.told_on.is_some() && alert.told_on == on_show() {
            return;
        }
        snackbars.show(match alert.kind {
            AlertKind::Error => Snackbar::new(alert.message).long(),
            AlertKind::Info => Snackbar::new(alert.message),
        });
    };
    for alert in earlier {
        tell(alert);
    }
    notices(later, tell);
}
