//! The illumos backend: the audio framework's OSS interface on `/dev/dsp`.

mod abi;
mod catalog;
mod identity;
mod stream;

pub(crate) use catalog::{DeviceCatalog, HotplugMonitor, RouteController};
pub(crate) use identity::{
    DEVICE_API, DEVICE_FACTORY_NAME, DEVICE_LOG_TOPIC, DIAGNOSTIC_TAG, MONITOR_FACTORY_NAME,
    MONITOR_LOG_TOPIC, OssDeviceInit, OssNodeInit, OssNodeProperties, REBUILD_THREAD_PREFIX,
    SINK_FACTORY_NAME, SINK_LOG_TOPIC, SOURCE_COMMAND_PREFIX, SOURCE_FACTORY_NAME,
    SOURCE_LOG_TOPIC, STREAM_PATH, clock_name, hotplug_diagnostic,
};
pub(crate) use stream::OssStream;

use crate::backend::{
    ChannelLayout, ChannelMapError, ChannelSet, ConfigurationFlags, ConversionPath, RateSet,
    StreamCaps, StreamConfig, StreamConfiguration,
};

/// What every illumos audio device accepts from a client: the framework
/// converts rate, format and channel count in the kernel.
pub(crate) fn caps() -> StreamCaps {
    StreamCaps {
        configurations: vec![StreamConfiguration {
            formats: stream::FORMATS.iter().map(|(_, spa)| *spa).collect(),
            // Named layouts, stereo first: the shared node code only admits a
            // channel map it was offered, and a bare channel-count range
            // would leave the ports without speaker positions.
            channels: ChannelSet::Discrete(vec![
                ChannelLayout {
                    channels: 2,
                    positions: Some(vec![
                        libspa::sys::SPA_AUDIO_CHANNEL_FL,
                        libspa::sys::SPA_AUDIO_CHANNEL_FR,
                    ]),
                },
                ChannelLayout {
                    channels: 1,
                    positions: Some(vec![libspa::sys::SPA_AUDIO_CHANNEL_MONO]),
                },
            ]),
            rates: RateSet::Range {
                min: 8_000,
                max: 192_000,
            },
            preferred_rate: Some(48_000),
            rate_tolerance: 0,
            conversion: ConversionPath::Kernel,
            flags: ConfigurationFlags::default(),
        }],
        preferred: 0,
    }
}

pub(crate) enum IllumosOss {}

impl crate::backend::Backend for IllumosOss {
    type Capture = OssStream;
    type Playback = OssStream;
    type Properties = OssNodeProperties;
    type DeviceInit = OssDeviceInit;
    type NodeInit = OssNodeInit;
    type Catalog = DeviceCatalog;
    type Hotplug = HotplugMonitor;
    type Routes = RouteController;

    const DEVICE_API: &'static str = DEVICE_API;
    const DIAGNOSTIC_TAG: &'static str = DIAGNOSTIC_TAG;
    const REBUILD_THREAD_PREFIX: &'static str = REBUILD_THREAD_PREFIX;
    const SOURCE_COMMAND_PREFIX: &'static str = SOURCE_COMMAND_PREFIX;
    const STREAM_PATH: &'static str = STREAM_PATH;
    const DEVICE_FACTORY_NAME: &'static std::ffi::CStr = DEVICE_FACTORY_NAME;
    const SINK_FACTORY_NAME: &'static std::ffi::CStr = SINK_FACTORY_NAME;
    const SOURCE_FACTORY_NAME: &'static std::ffi::CStr = SOURCE_FACTORY_NAME;

    fn clock_name(stream_path: &str) -> std::ffi::CString {
        clock_name(stream_path)
    }

    fn fallback_caps() -> StreamCaps {
        caps()
    }

    fn hotplug_diagnostic(kind: crate::backend::HotplugDiagnostic) -> &'static str {
        hotplug_diagnostic(kind)
    }

    fn probe_caps(_path: &str, _playback: bool) -> Option<StreamCaps> {
        Some(caps())
    }

    fn validate_config(caps: &StreamCaps, config: &StreamConfig) -> Result<(), ChannelMapError> {
        if caps.admits(
            config.format.0,
            config.channels,
            (!config.positions.is_empty()).then_some(config.positions.as_slice()),
            config.rate,
        ) {
            Ok(())
        } else {
            Err(ChannelMapError::Unsupported)
        }
    }

    fn device_log_topic() -> std::ptr::NonNull<libspa::sys::spa_log_topic> {
        std::ptr::NonNull::new(&raw mut crate::DEVICE_TOPIC)
            .expect("the device topic has a static address")
    }

    fn monitor_log_topic() -> std::ptr::NonNull<libspa::sys::spa_log_topic> {
        std::ptr::NonNull::new(&raw mut crate::MONITOR_TOPIC)
            .expect("the monitor topic has a static address")
    }

    fn sink_log_topic() -> std::ptr::NonNull<libspa::sys::spa_log_topic> {
        std::ptr::NonNull::new(&raw mut crate::SINK_TOPIC)
            .expect("the sink topic has a static address")
    }

    fn source_log_topic() -> std::ptr::NonNull<libspa::sys::spa_log_topic> {
        std::ptr::NonNull::new(&raw mut crate::SOURCE_TOPIC)
            .expect("the source topic has a static address")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend as _;

    #[test]
    fn the_usual_stereo_config_is_admitted_and_float_is_not() {
        let caps = caps();
        let mut config = StreamConfig {
            format: libspa::param::audio::AudioFormat(libspa::sys::SPA_AUDIO_FORMAT_S16_LE),
            rate: 48_000,
            channels: 2,
            positions: Vec::new(),
            flags: 0,
            stride: 4,
        };
        assert!(IllumosOss::validate_config(&caps, &config).is_ok());
        config.positions = vec![
            libspa::sys::SPA_AUDIO_CHANNEL_FL,
            libspa::sys::SPA_AUDIO_CHANNEL_FR,
        ];
        assert!(IllumosOss::validate_config(&caps, &config).is_ok());
        // A map the device was never offered with is refused.
        config.positions = vec![
            libspa::sys::SPA_AUDIO_CHANNEL_AUX0,
            libspa::sys::SPA_AUDIO_CHANNEL_AUX1,
        ];
        assert!(IllumosOss::validate_config(&caps, &config).is_err());
        config.positions = Vec::new();
        config.format = libspa::param::audio::AudioFormat(libspa::sys::SPA_AUDIO_FORMAT_F32_LE);
        assert!(IllumosOss::validate_config(&caps, &config).is_err());
    }
}
