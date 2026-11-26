//! Playback and capture on an illumos audio device node through the OSS API.
//!
//! The illumos audio framework converts sample rate, format and channel count
//! in the kernel for each client, so the device accepts what the graph asks
//! for and this code stays small: open, negotiate, non-blocking read/write,
//! and report queue depth and under/overruns. Wake-ups come from the shared
//! timer path; illumos has no stream readiness events richer than poll(2).

use std::ffi::{CString, c_int};
use std::os::fd::RawFd;

use crate::backend::{
    AdjustmentFlags, AppliedBufferGeometry, BufferConstraints, CaptureBufferGeometry,
    CaptureBufferRequest, CaptureOperations, CaptureRetune, ConfigureOutcome, CounterUpdate,
    DeliveryQuantum, IoStatus, ObservationQuality, PauseOutcome, PlaybackBufferGeometry,
    PlaybackBufferRequest, PlaybackOperations, PlaybackRetune, ReadOutcome, StreamConfig,
    StreamError, StreamIdentity, StreamLifecycle, WakeBufferState, WakeDiagnostic, WakeDriver,
    WakeError, WakeEvent, WriteOutcome, XrunCounter, XrunObservation, XrunUnit,
};
use crate::spa::Log;

use super::abi::{
    self, AFMT_S16_LE, AFMT_S24_LE, AFMT_S32_LE, AudioBufInfo, AudioErrInfo, SNDCTL_DSP_CHANNELS,
    SNDCTL_DSP_GETERROR, SNDCTL_DSP_GETISPACE, SNDCTL_DSP_GETOSPACE, SNDCTL_DSP_HALT,
    SNDCTL_DSP_SETFMT, SNDCTL_DSP_SETFRAGMENT, SNDCTL_DSP_SPEED,
};
use super::identity::OssNodeProperties;

/// (illumos AFMT, SPA format), in order of preference.
pub(crate) const FORMATS: &[(c_int, u32)] = &[
    (AFMT_S16_LE, libspa::sys::SPA_AUDIO_FORMAT_S16_LE),
    (AFMT_S32_LE, libspa::sys::SPA_AUDIO_FORMAT_S32_LE),
    (AFMT_S24_LE, libspa::sys::SPA_AUDIO_FORMAT_S24_32_LE),
];

const SILENCE: [u8; 16 * 1024] = [0; 16 * 1024];

/// Buffer size hint given to the kernel before the format is set.
///
/// An OSS client on illumos gets five 10 ms fragments (50 ms) unless it asks
/// otherwise, which is barely more than one graph quantum. The request is
/// "fragment count << 16 | log2(fragment bytes)"; the kernel only uses the
/// product, as a byte total that must stay below its fixed 64 KiB client
/// buffer, and then lays out its own 10 ms fragments within it.
const BUFFER_HINT: c_int = (15 << 16) | 12; // 15 * 4096 = 61440 bytes

/// What `playback_geometry` needs, per period: two periods plus a target of
/// one and a quarter.
const PERIODS_NEEDED_NUM: u32 = 13;
const PERIODS_NEEDED_DEN: u32 = 4;

/// The largest graph quantum, in frames, a device buffer of this size can
/// sustain.
pub(crate) fn quantum_cap_frames(capacity_bytes: u32, stride: u32) -> Option<u32> {
    if capacity_bytes == 0 || stride == 0 {
        return None;
    }
    let frames = capacity_bytes / stride;
    Some((frames.saturating_mul(PERIODS_NEEDED_DEN) / PERIODS_NEEDED_NUM).max(1))
}

fn afmt_for(spa_format: u32) -> Option<c_int> {
    FORMATS
        .iter()
        .find(|(_, spa)| *spa == spa_format)
        .map(|(afmt, _)| *afmt)
}

fn spa_for(afmt: c_int) -> Option<u32> {
    FORMATS
        .iter()
        .find(|(candidate, _)| *candidate == afmt)
        .map(|(_, spa)| *spa)
}

fn io_status(errno: c_int) -> IoStatus {
    match errno {
        libc::EAGAIN | libc::EINTR => IoStatus::WouldBlock,
        libc::ENODEV | libc::ENXIO => IoStatus::Disconnected,
        other => IoStatus::Fatal(StreamError::from_native_code(other)),
    }
}

/// Playback target sizes, in bytes, for a device buffer of `capacity_bytes`
/// delivering in `quantum_bytes` steps.
pub(crate) fn playback_geometry(
    request: PlaybackBufferRequest,
    capacity_bytes: u32,
    quantum_bytes: u32,
    current_fill_bytes: u32,
) -> PlaybackBufferGeometry {
    let desired = (request.period_bytes / 8).saturating_mul(10);
    let write_max = request.period_bytes.max(request.maximum_write_bytes);
    let minimum_fill_bytes = request
        .period_bytes
        .saturating_add((request.period_bytes / 4).max(quantum_bytes));
    let required_capacity_bytes = request
        .period_bytes
        .saturating_mul(2)
        .saturating_add(desired)
        .max(
            minimum_fill_bytes
                .saturating_add(write_max)
                .saturating_add(quantum_bytes),
        );
    let ceiling = capacity_bytes
        .saturating_sub(write_max.saturating_add(quantum_bytes))
        .max(request.period_bytes);
    let target_goal_bytes = desired
        .max(minimum_fill_bytes)
        .min(ceiling)
        .max(request.period_bytes);
    let predicted = current_fill_bytes
        .saturating_add(request.write_bytes)
        .saturating_sub(request.period_bytes);
    PlaybackBufferGeometry {
        capacity_bytes,
        quantum_bytes,
        target_fill_bytes: target_goal_bytes.min(predicted),
        target_goal_bytes,
        minimum_fill_bytes,
        required_capacity_bytes,
        delay_capped: target_goal_bytes < desired.max(minimum_fill_bytes),
    }
}

pub(crate) fn capture_geometry(
    request: CaptureBufferRequest,
    capacity_bytes: u32,
    quantum_bytes: u32,
) -> CaptureBufferGeometry {
    let target_fill_bytes = request.period_bytes.saturating_add(quantum_bytes / 2);
    let ring_peak = capacity_bytes.saturating_sub(quantum_bytes);
    let minimum_peak = target_fill_bytes
        .saturating_add(quantum_bytes)
        .min(ring_peak);
    let peak_fill_bytes = target_fill_bytes
        .saturating_add(quantum_bytes / 2)
        .saturating_add(request.period_bytes / 2)
        .min(ring_peak)
        .max(minimum_peak);
    let required_capacity_bytes = target_fill_bytes
        .saturating_add(quantum_bytes.saturating_mul(2))
        .max(request.period_bytes.saturating_mul(2));
    CaptureBufferGeometry {
        capacity_bytes,
        quantum_bytes,
        target_fill_bytes,
        peak_fill_bytes,
        required_capacity_bytes,
        device_lost: false,
    }
}

/// illumos offers no stream wake events; the shared timer path is used.
pub(crate) struct NoWakeDriver;

impl WakeDriver for NoWakeDriver {
    fn notification_fd(&self) -> RawFd {
        -1
    }

    fn unregister_stream(&mut self) -> Result<(), WakeError> {
        Ok(())
    }

    fn arm_timer(&self, _delay_ns: u64) -> Result<(), WakeError> {
        Err(WakeError::new("no native wake source on illumos"))
    }

    fn next_event(&self) -> Result<Option<WakeEvent>, WakeError> {
        Ok(None)
    }
}

/// One direction of one open device node.
pub(crate) struct OssStream {
    path: String,
    fd: RawFd,
    running: bool,
    stride: u32,
    /// Bytes of the current frame already handed to (or taken from) the device.
    frame_off: u32,
    /// Capture bytes still to discard to get back onto a frame boundary.
    read_skip: u32,
    capacity_bytes: u32,
    fragment_bytes: u32,
    period_bytes: u32,
}

impl OssStream {
    fn open(&mut self, flags: c_int) -> Result<(), c_int> {
        self.close_fd();
        let path = CString::new(self.path.as_str()).map_err(|_| libc::EINVAL)?;
        let fd = unsafe { libc::open(path.as_ptr(), flags | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd == -1 {
            return Err(abi::last_errno());
        }
        self.fd = fd;
        Ok(())
    }

    fn close_fd(&mut self) {
        if self.fd >= 0 {
            unsafe { libc::close(self.fd) };
            self.fd = -1;
        }
        self.running = false;
        self.frame_off = 0;
        self.read_skip = 0;
    }

    /// Negotiate channels, format and rate; returns what the device applied.
    fn negotiate(&mut self, config: &StreamConfig) -> Result<StreamConfig, c_int> {
        let wanted_afmt = afmt_for(config.format.0).unwrap_or(AFMT_S16_LE);
        // A hint only; it takes effect when the parameters below are set.
        let _ = abi::ioctl_int(self.fd, SNDCTL_DSP_SETFRAGMENT, BUFFER_HINT);
        let channels = abi::ioctl_int(self.fd, SNDCTL_DSP_CHANNELS, config.channels as c_int)?;
        let afmt = abi::ioctl_int(self.fd, SNDCTL_DSP_SETFMT, wanted_afmt)?;
        let rate = abi::ioctl_int(self.fd, SNDCTL_DSP_SPEED, config.rate as c_int)?;
        let spa_format = spa_for(afmt).ok_or(libc::ENOTSUP)?;
        let sample_bytes = crate::backend::bytes_per_sample(spa_format).ok_or(libc::ENOTSUP)?;
        if channels <= 0 || rate <= 0 {
            return Err(libc::EINVAL);
        }
        let channels = channels as u32;
        Ok(StreamConfig {
            format: libspa::param::audio::AudioFormat(spa_format),
            rate: rate as u32,
            channels,
            positions: if channels == config.channels {
                config.positions.clone()
            } else {
                Vec::new()
            },
            flags: config.flags,
            stride: sample_bytes.saturating_mul(channels),
        })
    }

    fn buffer_info(&self, request: c_int) -> AudioBufInfo {
        let mut info = AudioBufInfo::default();
        let _ = abi::ioctl_struct(self.fd, request, &mut info);
        info
    }

    fn record_geometry(&mut self, info: AudioBufInfo) {
        let fragment = info.fragsize.max(0) as u32;
        let total = info.fragstotal.max(0) as u32;
        self.fragment_bytes = fragment;
        self.capacity_bytes = fragment.saturating_mul(total);
    }

    fn outcome(&self, requested: &StreamConfig, actual: StreamConfig) -> ConfigureOutcome {
        ConfigureOutcome {
            adjusted: AdjustmentFlags::between(requested, &actual),
            actual_config: actual,
            applied_buffer: AppliedBufferGeometry {
                capacity_bytes: (self.capacity_bytes != 0).then_some(self.capacity_bytes),
                quantum_bytes: (self.fragment_bytes != 0).then_some(self.fragment_bytes),
                delivery: DeliveryQuantum::unavailable(),
            },
            buffer_constraints: BufferConstraints {
                capacity_limit_bytes: (self.capacity_bytes != 0).then_some(self.capacity_bytes),
                quantum_cap_frames: quantum_cap_frames(self.capacity_bytes, self.stride),
                quantum_cap_basis: Some("3.25 periods"),
            },
        }
    }

    fn errors(&self) -> AudioErrInfo {
        // SAFETY: the structure is plain integers; all-zero is a valid value.
        let mut info: AudioErrInfo = unsafe { std::mem::zeroed() };
        let _ = abi::ioctl_struct(self.fd, SNDCTL_DSP_GETERROR, &mut info);
        info
    }

    fn halt(&mut self) -> bool {
        if self.fd < 0 || abi::ioctl_none(self.fd, SNDCTL_DSP_HALT).is_err() {
            return false;
        }
        self.running = false;
        self.frame_off = 0;
        self.read_skip = 0;
        true
    }

    fn write_bytes(&mut self, data: &[u8]) -> WriteOutcome {
        if self.fd < 0 {
            return WriteOutcome {
                bytes: 0,
                status: IoStatus::Disconnected,
            };
        }
        if data.is_empty() {
            return WriteOutcome::consumed(0);
        }
        let written = unsafe { libc::write(self.fd, data.as_ptr().cast(), data.len()) };
        if written < 0 {
            return WriteOutcome {
                bytes: 0,
                status: io_status(abi::last_errno()),
            };
        }
        let written = written as usize;
        if self.stride != 0 {
            self.frame_off = ((self.frame_off as usize + written) % self.stride as usize) as u32;
        }
        WriteOutcome::consumed(written)
    }

    /// Write zeroes; returns how many bytes the device accepted.
    fn write_zeroes(&mut self, mut bytes: usize) -> usize {
        let mut total = 0;
        while bytes != 0 {
            let chunk = bytes.min(SILENCE.len());
            let outcome = self.write_bytes(&SILENCE[..chunk]);
            total += outcome.bytes;
            if outcome.bytes != chunk {
                break;
            }
            bytes -= chunk;
        }
        total
    }
}

impl Drop for OssStream {
    fn drop(&mut self) {
        self.close_fd();
    }
}

impl StreamLifecycle for OssStream {
    type WakeDriver = NoWakeDriver;

    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            fd: -1,
            running: false,
            stride: 0,
            frame_off: 0,
            read_skip: 0,
            capacity_bytes: 0,
            fragment_bytes: 0,
            period_bytes: 0,
        }
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn is_closed(&self) -> bool {
        self.fd < 0
    }

    fn is_running(&self) -> bool {
        self.running
    }

    fn register_wake(
        &self,
        _driver: &mut Self::WakeDriver,
        _stream: StreamIdentity,
        _buffer: WakeBufferState,
    ) -> Result<(), WakeError> {
        Err(WakeError::new("no native wake source on illumos"))
    }

    fn wake_available() -> bool {
        false
    }

    fn new_wake_driver() -> Result<Self::WakeDriver, WakeError> {
        Err(WakeError::new("no native wake source on illumos"))
    }

    fn wake_diagnostic(kind: WakeDiagnostic) -> &'static str {
        match kind {
            WakeDiagnostic::Selected => "using native wake events",
            WakeDiagnostic::Create => "can't create a native wake source",
            WakeDiagnostic::Read => "reading the native wake event",
            WakeDiagnostic::Remove => "removing the native wake stream",
            WakeDiagnostic::Register => "can't register native wake events",
            WakeDiagnostic::Threshold => "can't set the native wake threshold",
            WakeDiagnostic::Arm => "arming the native wake timer",
        }
    }

    fn close(&mut self) {
        self.close_fd();
    }

    fn suspend(&mut self) -> bool {
        self.halt()
    }
}

impl PlaybackOperations for OssStream {
    type Properties = OssNodeProperties;

    fn configure(
        &mut self,
        config: &StreamConfig,
        _properties: &Self::Properties,
        log: &Log,
    ) -> Result<ConfigureOutcome, c_int> {
        self.open(libc::O_WRONLY).map_err(|errno| -errno)?;
        let actual = match self.negotiate(config) {
            Ok(actual) => actual,
            Err(errno) => {
                self.close_fd();
                return Err(-errno);
            }
        };
        self.stride = actual.stride.max(1);
        let info = self.buffer_info(SNDCTL_DSP_GETOSPACE);
        self.record_geometry(info);
        crate::info!(
            log,
            "{}: playback {} Hz, {} ch, stride {}; device buffer {} bytes in {}-byte fragments",
            self.path,
            actual.rate,
            actual.channels,
            self.stride,
            self.capacity_bytes,
            self.fragment_bytes
        );
        Ok(self.outcome(config, actual))
    }

    fn prime_buffer(
        &mut self,
        request: PlaybackBufferRequest,
        _properties: &Self::Properties,
        log: &Log,
    ) -> PlaybackBufferGeometry {
        self.running = true;
        self.period_bytes = request.period_bytes;
        let queued = <Self as PlaybackOperations>::queued_bytes(self);
        let geometry = playback_geometry(request, self.capacity_bytes, self.fragment_bytes, queued);
        crate::debug!(
            log,
            "{}: prime period {} target {} minimum {} capacity {} (needs {})",
            self.path,
            request.period_bytes,
            geometry.target_goal_bytes,
            geometry.minimum_fill_bytes,
            geometry.capacity_bytes,
            geometry.required_capacity_bytes
        );
        geometry
    }

    fn retune_buffer(
        &mut self,
        request: PlaybackBufferRequest,
        current_fill_bytes: u32,
        _now_ns: u64,
        _log: &Log,
    ) -> PlaybackRetune {
        if !self.running
            || self.period_bytes == 0
            || request.period_bytes == 0
            || request.period_bytes == self.period_bytes
        {
            return PlaybackRetune::Unchanged;
        }
        let geometry = playback_geometry(
            request,
            self.capacity_bytes,
            self.fragment_bytes,
            current_fill_bytes,
        );
        if geometry.capacity_bytes >= geometry.required_capacity_bytes {
            self.period_bytes = request.period_bytes;
            PlaybackRetune::Applied(geometry)
        } else if self.halt() {
            PlaybackRetune::Reprime
        } else {
            PlaybackRetune::Rebuild
        }
    }

    fn write(&mut self, data: &[u8]) -> WriteOutcome {
        self.write_bytes(data)
    }

    fn write_silence(&mut self, bytes: u32) {
        if self.stride == 0 {
            return;
        }
        if self.frame_off != 0 {
            let remaining = (self.stride - self.frame_off) as usize;
            if self.write_zeroes(remaining) != remaining {
                return;
            }
        }
        let bytes = bytes / self.stride * self.stride;
        self.write_zeroes(bytes as usize);
    }

    fn end_buffer_sequence(&mut self) -> bool {
        if self.frame_off == 0 || self.stride == 0 {
            return false;
        }
        let remaining = (self.stride - self.frame_off) as usize;
        if self.write_zeroes(remaining) == remaining {
            false
        } else {
            self.halt()
        }
    }

    fn queued_bytes(&self) -> u32 {
        // The client stream's own buffer: capacity minus free space.
        //
        // Not SNDCTL_DSP_GETODELAY: on illumos that adds the frames already
        // mixed into the device's ring (auclnt_get_output_qlen's second
        // count), which stays at tens of milliseconds whether or not this
        // stream has supplied anything. Measured on the first runs: the
        // device ring held 1024-4095 frames throughout while the stream
        // buffer was empty more than half the time, so the fill the node
        // steered by never showed the starvation.
        let free = self.buffer_info(SNDCTL_DSP_GETOSPACE).bytes.max(0) as u32;
        self.capacity_bytes.saturating_sub(free)
    }

    fn underruns(&self) -> XrunObservation {
        // SNDCTL_DSP_GETERROR reports the count since the last call and
        // clears it.
        XrunObservation {
            counter: XrunCounter::PRIMARY,
            value: self.errors().play_underruns.max(0) as u64,
            unit: XrunUnit::Events,
            update: CounterUpdate::ResettingTotal,
            quality: ObservationQuality::Estimated,
        }
    }

    fn log_underrun_recovery(&self, count: u32, _now_ns: u64, suppressed: u32, log: &Log) {
        crate::debug!(
            log,
            "{}: recovering from {} playback underrun(s) ({} not logged)",
            self.path,
            count,
            suppressed
        );
    }

    fn log_ignored_underruns(
        &self,
        count: u32,
        fill_bytes: u32,
        recovery_threshold_bytes: u32,
        log: &Log,
    ) {
        crate::debug!(
            log,
            "{}: {} playback underrun(s) at fill {} (threshold {})",
            self.path,
            count,
            fill_bytes,
            recovery_threshold_bytes
        );
    }

    fn pause(&mut self) -> Result<PauseOutcome, StreamError> {
        // The queue is not preserved across a pause; the node resets and
        // primes again on Start.
        Ok(PauseOutcome::Reprime)
    }

    fn resume(&mut self) -> Result<(), StreamError> {
        if self.fd < 0 {
            return Err(StreamError::from_native_code(libc::ENODEV));
        }
        self.running = true;
        Ok(())
    }

    fn underrun_low(
        _target_fill: u32,
        _delivery_quantum: u32,
        period_bytes: u32,
        _drained_bytes: u32,
    ) -> u32 {
        period_bytes.max(1)
    }

    fn debug_log_priorities(_log: &Log) {}
}

impl CaptureOperations for OssStream {
    type Properties = OssNodeProperties;

    fn configure(
        &mut self,
        config: &StreamConfig,
        _properties: &Self::Properties,
        log: &Log,
    ) -> Result<ConfigureOutcome, c_int> {
        self.open(libc::O_RDONLY).map_err(|errno| -errno)?;
        let actual = match self.negotiate(config) {
            Ok(actual) => actual,
            Err(errno) => {
                self.close_fd();
                return Err(-errno);
            }
        };
        self.stride = actual.stride.max(1);
        let info = self.buffer_info(SNDCTL_DSP_GETISPACE);
        self.record_geometry(info);
        crate::info!(
            log,
            "{}: capture {} Hz, {} ch, stride {}; device buffer {} bytes in {}-byte fragments",
            self.path,
            actual.rate,
            actual.channels,
            self.stride,
            self.capacity_bytes,
            self.fragment_bytes
        );
        Ok(self.outcome(config, actual))
    }

    fn prime_buffer(
        &mut self,
        request: CaptureBufferRequest,
        _properties: &Self::Properties,
        _scratch: &mut [u8],
        _log: &Log,
    ) -> CaptureBufferGeometry {
        self.running = true;
        self.period_bytes = request.period_bytes;
        let mut geometry = capture_geometry(request, self.capacity_bytes, self.fragment_bytes);
        geometry.device_lost = self.fd < 0;
        geometry
    }

    fn retune_buffer(
        &mut self,
        request: CaptureBufferRequest,
        primed: bool,
        _log: &Log,
    ) -> CaptureRetune {
        if !primed
            || self.period_bytes == 0
            || request.period_bytes == 0
            || request.period_bytes == self.period_bytes
        {
            return CaptureRetune::Unchanged;
        }
        let geometry = capture_geometry(request, self.capacity_bytes, self.fragment_bytes);
        if geometry.capacity_bytes >= geometry.required_capacity_bytes {
            self.period_bytes = request.period_bytes;
            CaptureRetune::Applied(geometry)
        } else if self.halt() {
            CaptureRetune::Reprime
        } else {
            CaptureRetune::Rebuild
        }
    }

    fn read(&mut self, data: &mut [u8]) -> ReadOutcome {
        if self.fd < 0 {
            return ReadOutcome {
                bytes: 0,
                status: IoStatus::Disconnected,
            };
        }
        let stride = self.stride.max(1) as usize;
        // Drop the tail of a frame whose head an earlier short read consumed.
        while self.read_skip != 0 {
            let mut scratch = [0u8; 64];
            let want = (self.read_skip as usize).min(scratch.len());
            let got = unsafe { libc::read(self.fd, scratch.as_mut_ptr().cast(), want) };
            if got <= 0 {
                let status = if got == 0 {
                    IoStatus::WouldBlock
                } else {
                    io_status(abi::last_errno())
                };
                return ReadOutcome { bytes: 0, status };
            }
            self.read_skip -= got as u32;
        }
        let want = data.len() / stride * stride;
        if want == 0 {
            return ReadOutcome {
                bytes: 0,
                status: IoStatus::Progress,
            };
        }
        let got = unsafe { libc::read(self.fd, data.as_mut_ptr().cast(), want) };
        if got < 0 {
            return ReadOutcome {
                bytes: 0,
                status: io_status(abi::last_errno()),
            };
        }
        let got = got as usize;
        let torn = got % stride;
        if torn != 0 {
            self.read_skip = (stride - torn) as u32;
        }
        ReadOutcome {
            bytes: got - torn,
            status: if got == 0 {
                IoStatus::WouldBlock
            } else {
                IoStatus::Progress
            },
        }
    }

    fn queued_bytes(&self) -> u32 {
        self.buffer_info(SNDCTL_DSP_GETISPACE).bytes.max(0) as u32
    }

    fn overruns(&self) -> XrunObservation {
        XrunObservation {
            counter: XrunCounter::PRIMARY,
            value: self.errors().rec_overruns.max(0) as u64,
            unit: XrunUnit::Events,
            update: CounterUpdate::ResettingTotal,
            quality: ObservationQuality::Estimated,
        }
    }

    fn recover_overrun(
        &mut self,
        _overrun_count: u32,
        _pre_read_fill: Option<u32>,
        _log: &Log,
    ) -> Option<bool> {
        // The framework drops the oldest data and keeps recording; nothing
        // needs resetting.
        None
    }

    fn log_overrun_recovery(&self, count: u32, _now_ns: u64, suppressed: u32, log: &Log) {
        crate::debug!(
            log,
            "{}: {} capture overrun(s) ({} not logged)",
            self.path,
            count,
            suppressed
        );
    }

    fn clear_overrun_observation(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(period: u32) -> PlaybackBufferRequest {
        PlaybackBufferRequest {
            period_bytes: period,
            graph_rate: 48_000,
            stride: 4,
            device_rate: 48_000,
            write_bytes: period,
            maximum_write_bytes: period,
        }
    }

    #[test]
    fn the_buffer_hint_stays_under_the_kernel_limit() {
        let fragments = (BUFFER_HINT >> 16) as u32;
        let bytes = 1u32 << (BUFFER_HINT & 0xffff);
        assert!((2..0x7fff).contains(&fragments));
        assert!(fragments * bytes < 65_536);
    }

    #[test]
    fn the_quantum_cap_follows_the_device_buffer() {
        // 61440 bytes of 8-byte frames: room for a 2048-frame quantum.
        assert_eq!(quantum_cap_frames(61_440, 8), Some(2_363));
        // The 50 ms default (19200 bytes) is not: the cap drops below 1024.
        assert_eq!(quantum_cap_frames(19_200, 8), Some(738));
        assert_eq!(quantum_cap_frames(0, 8), None);
        // The cap is exactly what the geometry formula asks for.
        let period = 2_363 * 8;
        let geometry = playback_geometry(request(period), 61_440, 3_840, 0);
        assert!(geometry.capacity_bytes >= geometry.required_capacity_bytes);
    }

    #[test]
    fn formats_round_trip() {
        for (afmt, spa) in FORMATS {
            assert_eq!(afmt_for(*spa), Some(*afmt));
            assert_eq!(spa_for(*afmt), Some(*spa));
        }
        assert_eq!(afmt_for(libspa::sys::SPA_AUDIO_FORMAT_F32_LE), None);
    }

    #[test]
    fn errno_values_map_to_stream_states() {
        assert_eq!(io_status(libc::EAGAIN), IoStatus::WouldBlock);
        assert_eq!(io_status(libc::ENXIO), IoStatus::Disconnected);
        assert!(matches!(io_status(libc::EIO), IoStatus::Fatal(_)));
    }

    #[test]
    fn playback_targets_stay_inside_the_device_buffer() {
        let geometry = playback_geometry(request(4096), 65_536, 1024, 0);
        assert!(geometry.target_goal_bytes >= 4096);
        assert!(geometry.target_goal_bytes <= 65_536);
        assert!(geometry.minimum_fill_bytes > 4096);
        assert!(geometry.capacity_bytes >= geometry.required_capacity_bytes);
        assert!(!geometry.delay_capped);
        // Nothing queued yet: the first target is whatever one write leaves.
        assert_eq!(geometry.target_fill_bytes, 0);
    }

    #[test]
    fn a_small_device_buffer_is_reported_as_too_small() {
        let geometry = playback_geometry(request(8192), 8192, 1024, 0);
        assert!(geometry.capacity_bytes < geometry.required_capacity_bytes);
    }

    #[test]
    fn a_closed_stream_refuses_io_without_touching_a_descriptor() {
        let mut stream = <OssStream as StreamLifecycle>::new("/nonexistent/dsp");
        assert!(stream.is_closed());
        assert!(!stream.is_running());
        assert_eq!(stream.write_bytes(&[0; 8]).status, IoStatus::Disconnected);
        assert!(!<OssStream as StreamLifecycle>::suspend(&mut stream));
        assert!(!<OssStream as StreamLifecycle>::wake_available());
    }

    #[test]
    fn writes_track_the_position_inside_a_frame() {
        // A pipe stands in for the device node: plain non-blocking writes.
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let mut stream = <OssStream as StreamLifecycle>::new("pipe");
        stream.fd = fds[1];
        stream.stride = 4;
        assert_eq!(stream.write_bytes(&[1, 2, 3, 4, 5, 6]).bytes, 6);
        assert_eq!(stream.frame_off, 2);
        // write_silence first closes the open frame, then writes whole frames.
        <OssStream as PlaybackOperations>::write_silence(&mut stream, 9);
        assert_eq!(stream.frame_off, 0);
        let mut seen = [0xffu8; 32];
        let got = unsafe { libc::read(fds[0], seen.as_mut_ptr().cast(), seen.len()) };
        assert_eq!(got, 6 + 2 + 8);
        assert_eq!(&seen[..6], &[1, 2, 3, 4, 5, 6]);
        assert!(seen[6..16].iter().all(|&byte| byte == 0));
        unsafe { libc::close(fds[0]) };
    }
}
