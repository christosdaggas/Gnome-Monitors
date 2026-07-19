//! Monitor identification overlays via GNOME Shell.
//!
//! GNOME Settings uses `org.gnome.Shell.ShowMonitorLabels(a{sv})` (dict of
//! connector name → int32 label number) and `HideMonitorLabels()`; both were
//! verified present on the installed Shell 50.3. This is the supported way to
//! identify displays under Wayland — no private Shell APIs, no screen
//! capture.

use std::collections::HashMap;

use zvariant::OwnedValue;

use crate::error::BackendError;

#[zbus::proxy(
    interface = "org.gnome.Shell",
    default_service = "org.gnome.Shell",
    default_path = "/org/gnome/Shell"
)]
trait Shell {
    fn show_monitor_labels(&self, params: HashMap<String, OwnedValue>) -> zbus::Result<()>;
    fn hide_monitor_labels(&self) -> zbus::Result<()>;
}

/// Shows numbered identification labels on monitors.
#[derive(Debug, Clone)]
pub struct MonitorLabeler {
    proxy: ShellProxy<'static>,
}

impl MonitorLabeler {
    pub async fn connect() -> Result<Self, BackendError> {
        let connection = zbus::Connection::session().await?;
        Self::with_connection(&connection).await
    }

    pub async fn with_connection(connection: &zbus::Connection) -> Result<Self, BackendError> {
        let proxy = ShellProxy::builder(connection)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        Ok(Self { proxy })
    }

    /// Shows a numbered label on each given connector. GNOME Shell keeps the
    /// labels up until [`MonitorLabeler::hide`] is called.
    pub async fn show(&self, numbers: &[(String, i32)]) -> Result<(), BackendError> {
        let mut params: HashMap<String, OwnedValue> = HashMap::new();
        for (connector, number) in numbers {
            params.insert(
                connector.clone(),
                zvariant::Value::from(*number)
                    .try_to_owned()
                    .map_err(|e| BackendError::Encode(e.to_string()))?,
            );
        }
        self.proxy.show_monitor_labels(params).await?;
        Ok(())
    }

    pub async fn hide(&self) -> Result<(), BackendError> {
        self.proxy.hide_monitor_labels().await?;
        Ok(())
    }
}
