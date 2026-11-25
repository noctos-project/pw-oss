//! Device discovery, hot-plug and routes.
//!
//! First cut: the catalog is static (the default device node, when present),
//! nothing is hot-plugged, and there are no hardware routes, so volume is
//! applied in software by the graph. Enumeration through SNDCTL_SYSINFO /
//! SNDCTL_AUDIOINFO and the OSS 4 mixer come later.

use std::os::fd::RawFd;

use crate::backend::{
    CatalogError, CatalogGroupSnapshot, CatalogRescan, DeviceKey, DeviceSnapshot, RouteChange,
    RouteKey, RouteSnapshot, RouteUpdate, RouteWatchPolicy,
};

use super::identity::{DEVICE_API, DEVICE_DESCRIPTION, DEVICE_PATH};

pub(crate) const DEFAULT_DEVICE: &str = "/dev/dsp";

pub(crate) struct DeviceCatalog {
    groups: Vec<CatalogGroupSnapshot>,
}

impl crate::backend::DeviceCatalog for DeviceCatalog {
    fn open_error_context() -> &'static str {
        "can't read the illumos audio device list"
    }

    fn refresh_error_context() -> &'static str {
        "can't refresh the illumos audio device list"
    }

    fn scan() -> Result<Self, CatalogError> {
        let mut groups = Vec::new();
        if std::fs::metadata(DEFAULT_DEVICE).is_ok() {
            groups.push(CatalogGroupSnapshot {
                key: DeviceKey::qualified(DEVICE_API, DEFAULT_DEVICE),
                object_id: 0,
                properties: vec![
                    (DEVICE_PATH.to_string(), DEFAULT_DEVICE.to_string()),
                    (DEVICE_DESCRIPTION.to_string(), "illumos audio".to_string()),
                ],
            });
        }
        Ok(Self { groups })
    }

    fn snapshots(&self) -> Vec<CatalogGroupSnapshot> {
        self.groups.clone()
    }
}

/// A descriptor that never becomes readable: the shared monitor wants
/// something to watch, and illumos has no device arrival events wired up.
pub(crate) struct HotplugMonitor {
    read_end: RawFd,
    write_end: RawFd,
}

impl crate::backend::HotplugMonitor<DeviceCatalog> for HotplugMonitor {
    fn open() -> Result<Self, std::io::Error> {
        let mut fds = [-1 as RawFd; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
        for fd in fds {
            unsafe {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                let flags = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        Ok(Self {
            read_end: fds[0],
            write_end: fds[1],
        })
    }

    fn fd(&self) -> RawFd {
        self.read_end
    }

    fn read_catalog_rescan(
        &mut self,
        _catalog: &mut DeviceCatalog,
    ) -> (bool, Option<CatalogRescan>) {
        // Nothing is ever written to the pipe; drain defensively.
        let mut scratch = [0u8; 64];
        while unsafe { libc::read(self.read_end, scratch.as_mut_ptr().cast(), scratch.len()) } > 0 {
        }
        (true, None)
    }
}

impl Drop for HotplugMonitor {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.read_end);
            libc::close(self.write_end);
        }
    }
}

#[derive(Default)]
pub(crate) struct RouteController;

impl crate::backend::RouteController<HotplugMonitor> for RouteController {
    fn probe(_snapshot: &DeviceSnapshot) -> (Self, Vec<RouteSnapshot>) {
        (Self, Vec::new())
    }

    fn refresh_all(&mut self, _routes: &mut [RouteSnapshot]) {}

    fn poll(&mut self, _routes: &mut [RouteSnapshot]) -> Vec<RouteChange> {
        Vec::new()
    }

    fn apply(
        &mut self,
        _routes: &mut [RouteSnapshot],
        _key: RouteKey,
        _update: RouteUpdate,
    ) -> RouteChange {
        RouteChange::default()
    }

    fn watch_policy(&self) -> RouteWatchPolicy {
        RouteWatchPolicy {
            poll_interval_ns: None,
            event_driven: false,
        }
    }

    fn read_hotplug(
        &self,
        _routes: &[RouteSnapshot],
        _monitor: &mut HotplugMonitor,
    ) -> (bool, bool) {
        (true, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::HotplugMonitor as _;

    #[test]
    fn the_hotplug_descriptor_is_valid_and_silent() {
        let mut monitor = HotplugMonitor::open().expect("a pipe can be created");
        assert!(monitor.fd() >= 0);
        let mut pfd = libc::pollfd {
            fd: monitor.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&raw mut pfd, 1, 0) }, 0);
        let mut catalog = DeviceCatalog { groups: Vec::new() };
        assert!(matches!(
            monitor.read_catalog_rescan(&mut catalog),
            (true, None)
        ));
    }
}
