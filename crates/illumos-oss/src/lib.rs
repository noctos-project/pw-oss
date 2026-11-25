//! PipeWire SPA plugin for illumos audio devices (OSS interface).
//!
//! Built on the backend-neutral machinery of this repository; see
//! `illumos_oss` for what is illumos-specific.

#![warn(unreachable_pub)]
#![warn(clippy::uninlined_format_args)]
#![warn(clippy::semicolon_if_nothing_returned)]

use libspa::sys::spa_handle_factory;
use std::ffi::c_int;

mod illumos_oss;
use spa_kitchen_sink_core::{backend, device, monitor, node, spa};
pub(crate) use spa_kitchen_sink_core::{debug, info};

type SelectedBackend = illumos_oss::IllumosOss;

const MONITOR_FACTORY: spa_handle_factory =
    monitor::factory::<SelectedBackend>(illumos_oss::MONITOR_FACTORY_NAME.as_ptr());
const DEVICE_FACTORY: spa_handle_factory =
    device::factory::<SelectedBackend>(illumos_oss::DEVICE_FACTORY_NAME.as_ptr());
const SINK_FACTORY: spa_handle_factory =
    node::sink_factory::<SelectedBackend>(illumos_oss::SINK_FACTORY_NAME.as_ptr());
const SOURCE_FACTORY: spa_handle_factory =
    node::source_factory::<SelectedBackend>(illumos_oss::SOURCE_FACTORY_NAME.as_ptr());

/// SPA plugin entry point: enumerate the factories this library provides.
///
/// # Safety
/// `factory` and `index` must be valid pointers supplied by the SPA host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spa_handle_factory_enum(
    factory: *mut *const spa_handle_factory,
    index: *mut u32,
) -> c_int {
    assert!(!factory.is_null());
    assert!(!index.is_null());
    let factories: [&'static spa_handle_factory; 4] = [
        &MONITOR_FACTORY,
        &DEVICE_FACTORY,
        &SINK_FACTORY,
        &SOURCE_FACTORY,
    ];
    unsafe {
        match factories.get(*index as usize) {
            Some(found) => {
                *factory = *found;
                *index += 1;
                1
            }
            None => 0,
        }
    }
}

use libspa::sys::{SPA_LOG_LEVEL_NONE, SPA_VERSION_LOG_TOPIC, spa_log_topic};

// The host adjusts each topic's level when it is first used. The optional
// `spa_log_topic_enum` export (bulk registration through a linker section) is
// left out: it relies on GNU ld's `__start_`/`__stop_` section symbols, which
// the illumos link-editor does not provide.
static mut DEVICE_TOPIC: spa_log_topic = spa_log_topic {
    version: SPA_VERSION_LOG_TOPIC,
    topic: illumos_oss::DEVICE_LOG_TOPIC.as_ptr(),
    level: SPA_LOG_LEVEL_NONE,
    has_custom_level: false,
};
static mut SINK_TOPIC: spa_log_topic = spa_log_topic {
    version: SPA_VERSION_LOG_TOPIC,
    topic: illumos_oss::SINK_LOG_TOPIC.as_ptr(),
    level: SPA_LOG_LEVEL_NONE,
    has_custom_level: false,
};
static mut SOURCE_TOPIC: spa_log_topic = spa_log_topic {
    version: SPA_VERSION_LOG_TOPIC,
    topic: illumos_oss::SOURCE_LOG_TOPIC.as_ptr(),
    level: SPA_LOG_LEVEL_NONE,
    has_custom_level: false,
};
static mut MONITOR_TOPIC: spa_log_topic = spa_log_topic {
    version: SPA_VERSION_LOG_TOPIC,
    topic: illumos_oss::MONITOR_LOG_TOPIC.as_ptr(),
    level: SPA_LOG_LEVEL_NONE,
    has_custom_level: false,
};
