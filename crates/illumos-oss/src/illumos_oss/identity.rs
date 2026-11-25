//! Names, creation-time options and (absent) backend properties.

use std::ffi::{CStr, CString};

use crate::backend::{
    BackendProperties, BackendPropertyDescriptor, DeviceInit, DeviceSnapshot, EndpointKey,
    EndpointSnapshot, HotplugDiagnostic, InitItemResult, NodeInit, NodeInitValues, StreamDirection,
    StreamLocator,
};

pub(crate) const DEVICE_API: &str = "illumos-oss";
pub(crate) const DIAGNOSTIC_TAG: &str = DEVICE_API;

pub(crate) const MONITOR_FACTORY_NAME: &CStr = c"illumos-oss.monitor";
pub(crate) const DEVICE_FACTORY_NAME: &CStr = c"illumos-oss.device";
pub(crate) const SINK_FACTORY_NAME: &CStr = c"illumos-oss.sink";
pub(crate) const SOURCE_FACTORY_NAME: &CStr = c"illumos-oss.source";

pub(crate) const DEVICE_LOG_TOPIC: &CStr = c"spa.oss.device";
pub(crate) const MONITOR_LOG_TOPIC: &CStr = c"spa.oss.monitor";
pub(crate) const SINK_LOG_TOPIC: &CStr = c"spa.oss.sink";
pub(crate) const SOURCE_LOG_TOPIC: &CStr = c"spa.oss.source";
pub(crate) const REBUILD_THREAD_PREFIX: &str = "pw-oss";
pub(crate) const SOURCE_COMMAND_PREFIX: &str = "oss-source: ";

/// Node property naming the device node to open, e.g. `/dev/dsp`.
pub(crate) const STREAM_PATH: &str = "api.illumos-oss.dsp-path";
/// Accepted for configuration compatibility; this backend always uses the
/// shared timer path because illumos has no stream wake events to offer.
pub(crate) const FORCE_TIMER: &str = "api.illumos-oss.force-timer";

/// Device-factory properties written by the catalog.
pub(crate) const DEVICE_PATH: &str = "api.illumos-oss.path";
pub(crate) const DEVICE_DESCRIPTION: &str = "api.illumos-oss.description";
pub(crate) const DEVICE_CAPTURE: &str = "api.illumos-oss.capture";

pub(crate) fn clock_name(stream_path: &str) -> CString {
    let name: String = format!("api.illumos-oss.{stream_path}")
        .chars()
        .filter(|c| *c != '\0')
        .collect();
    CString::new(name).expect("interior NULs were removed")
}

pub(crate) fn hotplug_diagnostic(kind: HotplugDiagnostic) -> &'static str {
    match kind {
        HotplugDiagnostic::MonitorDetachAbort => "aborting: the device monitor source is stuck",
        HotplugDiagnostic::MonitorLost => "lost the device monitor",
        HotplugDiagnostic::MonitorOpen => "can't open the device monitor",
        HotplugDiagnostic::MonitorWatch => "can't watch the device monitor",
        HotplugDiagnostic::RouteDetachAbort => "aborting: the route monitor source is stuck",
        HotplugDiagnostic::RouteLost => "lost the route monitor",
        HotplugDiagnostic::RouteNudge => "can't nudge the route refresh",
        HotplugDiagnostic::RouteOpen => "can't open the route monitor",
        HotplugDiagnostic::RouteOpenFallback => "route changes will not be noticed",
        HotplugDiagnostic::RouteTimerDetachAbort => "aborting: the route timer source is stuck",
        HotplugDiagnostic::RouteTimerArm => "can't arm the route timer",
        HotplugDiagnostic::RouteTimerCreate => "can't create the route timer",
        HotplugDiagnostic::RouteTimerWatch => "can't watch the route timer",
        HotplugDiagnostic::RouteWatch => "can't watch the route monitor",
    }
}

/// This backend exposes no tunable node properties yet.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OssNodeProperties;

/// There is nothing to update; the type exists to satisfy the contract.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NoUpdate {}

impl BackendProperties for OssNodeProperties {
    type Update = NoUpdate;

    fn new(_playback: bool) -> Self {
        Self
    }

    fn descriptors(&self) -> &'static [BackendPropertyDescriptor] {
        &[]
    }

    fn values(&self) -> Vec<(&'static str, u32)> {
        Vec::new()
    }

    fn decode_params(_value: &libspa::pod::Value) -> Vec<Self::Update> {
        Vec::new()
    }

    fn apply(&mut self, update: Self::Update) -> bool {
        match update {}
    }

    fn reset(&mut self) -> bool {
        false
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    if ["1", "true", "yes", "on"]
        .iter()
        .any(|accepted| value.eq_ignore_ascii_case(accepted))
    {
        Some(true)
    } else if ["0", "false", "no", "off"]
        .iter()
        .any(|accepted| value.eq_ignore_ascii_case(accepted))
    {
        Some(false)
    } else {
        None
    }
}

pub(crate) struct OssNodeInit {
    path: Option<String>,
    force_timer: bool,
    properties: OssNodeProperties,
}

impl NodeInit for OssNodeInit {
    type Properties = OssNodeProperties;

    fn new(playback: bool) -> Self {
        Self {
            path: None,
            force_timer: false,
            properties: OssNodeProperties::new(playback),
        }
    }

    fn force_timer_key() -> Option<&'static str> {
        Some(FORCE_TIMER)
    }

    fn parse(&mut self, key: &str, value: &str) -> InitItemResult {
        match key {
            STREAM_PATH => {
                self.path = Some(value.to_string());
                InitItemResult::Handled
            }
            FORCE_TIMER => match parse_bool(value) {
                Some(force) => {
                    self.force_timer = force;
                    InitItemResult::Handled
                }
                None => InitItemResult::InvalidBoolean,
            },
            _ => InitItemResult::Unknown,
        }
    }

    fn into_values(self) -> NodeInitValues<Self::Properties> {
        NodeInitValues {
            stream_path: self.path,
            force_timer: self.force_timer,
            properties: self.properties,
        }
    }
}

#[derive(Default)]
pub(crate) struct OssDeviceInit {
    path: Option<String>,
    description: Option<String>,
    capture: bool,
}

impl DeviceInit for OssDeviceInit {
    fn parse(&mut self, key: &str, value: &str) {
        match key {
            DEVICE_PATH => self.path = Some(value.to_string()),
            DEVICE_DESCRIPTION => self.description = Some(value.to_string()),
            DEVICE_CAPTURE => self.capture = parse_bool(value).unwrap_or(false),
            _ => {}
        }
    }

    fn parent_name(&self) -> Option<&str> {
        self.path.as_deref()
    }

    fn is_complete(&self) -> bool {
        self.path.is_some()
    }

    fn snapshot(&self) -> Option<DeviceSnapshot> {
        let path = self.path.as_deref()?;
        let description = self
            .description
            .clone()
            .unwrap_or_else(|| format!("illumos audio ({path})"));
        let short = path.rsplit('/').next().unwrap_or(path);
        let mut endpoints = vec![EndpointSnapshot {
            key: EndpointKey::qualified(DEVICE_API, &format!("{path}:playback")),
            object_id: 0,
            direction: StreamDirection::Playback,
            name: format!("{short}.playback"),
            description: description.clone(),
            locator: StreamLocator::new(DEVICE_API, path),
        }];
        if self.capture {
            endpoints.push(EndpointSnapshot {
                key: EndpointKey::qualified(DEVICE_API, &format!("{path}:capture")),
                object_id: 1,
                direction: StreamDirection::Capture,
                name: format!("{short}.capture"),
                description: description.clone(),
                locator: StreamLocator::new(DEVICE_API, path),
            });
        }
        Some(DeviceSnapshot {
            description,
            endpoints,
        })
    }

    fn missing_selector_diagnostic() -> &'static str {
        "api.illumos-oss.path should name an audio device node"
    }

    fn snapshot_diagnostic() -> &'static str {
        "can't describe the illumos audio device"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_init_reads_its_two_keys() {
        let mut init = OssNodeInit::new(true);
        assert!(matches!(
            init.parse(STREAM_PATH, "/dev/dsp"),
            InitItemResult::Handled
        ));
        assert!(matches!(
            init.parse(FORCE_TIMER, "maybe"),
            InitItemResult::InvalidBoolean
        ));
        assert!(matches!(
            init.parse("node.name", "x"),
            InitItemResult::Unknown
        ));
        let values = init.into_values();
        assert_eq!(values.stream_path.as_deref(), Some("/dev/dsp"));
        assert!(!values.force_timer);
    }

    #[test]
    fn device_init_describes_playback_and_optional_capture() {
        let mut init = OssDeviceInit::default();
        assert!(!init.is_complete());
        init.parse(DEVICE_PATH, "/dev/dsp");
        assert!(init.is_complete());
        let snapshot = init.snapshot().expect("a path was given");
        assert_eq!(snapshot.endpoints.len(), 1);
        assert_eq!(snapshot.endpoints[0].locator.value, "/dev/dsp");
        init.parse(DEVICE_CAPTURE, "true");
        assert_eq!(init.snapshot().expect("still complete").endpoints.len(), 2);
    }
}
