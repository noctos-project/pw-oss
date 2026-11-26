//! The illumos OSS ioctl interface.
//!
//! Every number and layout here is taken from illumos-gate
//! `usr/src/uts/common/sys/audio/audio_oss.h`. illumos does not use the BSD
//! `_IOR`/`_IOW` encoding for these calls: it has its own (`__OSSIOR` etc.),
//! and several `AFMT_*` values differ from other OSS implementations. The
//! companion program `abi/compare.c` prints the same table from the system
//! header so the two can be compared on a real machine.

use std::ffi::{c_char, c_int, c_uint};
use std::os::fd::RawFd;

const OSSIOCPARM_MASK: u32 = 0x1fff;
const OSSIOC_VOID: u32 = 0x0000_0000;
const OSSIOC_OUT: u32 = 0x2000_0000;
const OSSIOC_IN: u32 = 0x4000_0000;
const OSSIOC_INOUT: u32 = OSSIOC_IN | OSSIOC_OUT;

const fn oss_ioc(dir: u32, group: u8, number: u32, size: usize) -> c_int {
    (dir | (((size as u32) & OSSIOCPARM_MASK) << 16) | ((group as u32) << 8) | number) as c_int
}

const fn oss_io(group: u8, number: u32) -> c_int {
    oss_ioc(OSSIOC_VOID, group, number, 0)
}

const fn oss_ior<T>(group: u8, number: u32) -> c_int {
    oss_ioc(OSSIOC_OUT, group, number, size_of::<T>())
}

const fn oss_iowr<T>(group: u8, number: u32) -> c_int {
    oss_ioc(OSSIOC_INOUT, group, number, size_of::<T>())
}

pub(crate) const AFMT_S16_LE: c_int = 0x0000_0010;
pub(crate) const AFMT_S32_LE: c_int = 0x0000_1000;
/// 24 significant bits, LSB aligned in a 32-bit word.
pub(crate) const AFMT_S24_LE: c_int = 0x0000_8000;

pub(crate) const SNDCTL_DSP_HALT: c_int = oss_io(b'P', 0);
pub(crate) const SNDCTL_DSP_SPEED: c_int = oss_iowr::<c_int>(b'P', 2);
pub(crate) const SNDCTL_DSP_SETFMT: c_int = oss_iowr::<c_int>(b'P', 5);
pub(crate) const SNDCTL_DSP_CHANNELS: c_int = oss_iowr::<c_int>(b'P', 6);
pub(crate) const SNDCTL_DSP_SETFRAGMENT: c_int = oss_iowr::<c_int>(b'P', 10);
pub(crate) const SNDCTL_DSP_GETOSPACE: c_int = oss_ior::<AudioBufInfo>(b'P', 12);
pub(crate) const SNDCTL_DSP_GETISPACE: c_int = oss_ior::<AudioBufInfo>(b'P', 13);
pub(crate) const SNDCTL_DSP_GETODELAY: c_int = oss_ior::<c_int>(b'P', 23);
pub(crate) const SNDCTL_DSP_GETERROR: c_int = oss_ior::<AudioErrInfo>(b'P', 25);
// Device enumeration (not used yet: the catalog is static for now).
#[allow(dead_code)]
pub(crate) const SNDCTL_SYSINFO: c_int = oss_ior::<OssSysInfo>(b'X', 1);
#[allow(dead_code)]
pub(crate) const SNDCTL_AUDIOINFO: c_int = oss_iowr::<OssAudioInfo>(b'X', 7);

/// `audio_buf_info`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AudioBufInfo {
    pub(crate) fragments: c_int,
    pub(crate) fragstotal: c_int,
    pub(crate) fragsize: c_int,
    pub(crate) bytes: c_int,
}

/// `audio_errinfo`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct AudioErrInfo {
    pub(crate) play_underruns: c_int,
    pub(crate) rec_overruns: c_int,
    pub(crate) play_ptradjust: c_uint,
    pub(crate) rec_ptradjust: c_uint,
    pub(crate) play_errorcount: c_int,
    pub(crate) rec_errorcount: c_int,
    pub(crate) play_lasterror: c_int,
    pub(crate) rec_lasterror: c_int,
    pub(crate) play_errorparm: c_int,
    pub(crate) rec_errorparm: c_int,
    pub(crate) filler: [c_int; 16],
}

/// `oss_sysinfo`
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct OssSysInfo {
    pub(crate) product: [c_char; 32],
    pub(crate) version: [c_char; 32],
    pub(crate) versionnum: c_int,
    pub(crate) options: [c_char; 128],
    pub(crate) numaudios: c_int,
    pub(crate) openedaudio: [c_int; 8],
    pub(crate) numsynths: c_int,
    pub(crate) nummidis: c_int,
    pub(crate) numtimers: c_int,
    pub(crate) nummixers: c_int,
    pub(crate) openedmidi: [c_int; 8],
    pub(crate) numcards: c_int,
    pub(crate) numaudioengines: c_int,
    pub(crate) license: [c_char; 16],
    pub(crate) revision_info: [c_char; 256],
    pub(crate) filler: [c_int; 172],
}

/// `oss_audioinfo`
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct OssAudioInfo {
    pub(crate) dev: c_int,
    pub(crate) name: [c_char; 64],
    pub(crate) busy: c_int,
    pub(crate) pid: c_int,
    pub(crate) caps: c_int,
    pub(crate) iformats: c_int,
    pub(crate) oformats: c_int,
    pub(crate) magic: c_int,
    pub(crate) cmd: [c_char; 64],
    pub(crate) card_number: c_int,
    pub(crate) port_number: c_int,
    pub(crate) mixer_dev: c_int,
    pub(crate) legacy_device: c_int,
    pub(crate) enabled: c_int,
    pub(crate) flags: c_int,
    pub(crate) min_rate: c_int,
    pub(crate) max_rate: c_int,
    pub(crate) min_channels: c_int,
    pub(crate) max_channels: c_int,
    pub(crate) binding: c_int,
    pub(crate) rate_source: c_int,
    pub(crate) handle: [c_char; 32],
    pub(crate) nrates: c_uint,
    pub(crate) rates: [c_uint; 20],
    pub(crate) song_name: [c_char; 64],
    pub(crate) label: [c_char; 16],
    pub(crate) latency: c_int,
    pub(crate) devnode: [c_char; 32],
    pub(crate) next_play_engine: c_int,
    pub(crate) next_rec_engine: c_int,
    pub(crate) filler: [c_int; 184],
}

/// The last OS error as a positive errno value.
pub(crate) fn last_errno() -> c_int {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

/// ioctl with no argument.
pub(crate) fn ioctl_none(fd: RawFd, request: c_int) -> Result<(), c_int> {
    // The request type differs between C libraries (int on illumos).
    if unsafe { libc::ioctl(fd, request as _) } == -1 {
        Err(last_errno())
    } else {
        Ok(())
    }
}

/// ioctl exchanging one `int`; returns the value the kernel wrote back.
pub(crate) fn ioctl_int(fd: RawFd, request: c_int, value: c_int) -> Result<c_int, c_int> {
    let mut value = value;
    if unsafe { libc::ioctl(fd, request as _, &raw mut value) } == -1 {
        Err(last_errno())
    } else {
        Ok(value)
    }
}

/// ioctl filling (or exchanging) a plain structure.
pub(crate) fn ioctl_struct<T: Copy>(fd: RawFd, request: c_int, value: &mut T) -> Result<(), c_int> {
    if unsafe { libc::ioctl(fd, request as _, std::ptr::from_mut(value)) } == -1 {
        Err(last_errno())
    } else {
        Ok(())
    }
}

/// A NUL-terminated C character array as a Rust string.
#[allow(dead_code)]
pub(crate) fn c_chars_to_string(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values worked out by hand from audio_oss.h; `abi/compare.c` prints the
    // same table from the header on a real system.
    #[test]
    fn structure_sizes_match_the_header() {
        assert_eq!(size_of::<AudioBufInfo>(), 16);
        assert_eq!(size_of::<AudioErrInfo>(), 104);
        assert_eq!(size_of::<OssSysInfo>(), 1248);
        assert_eq!(size_of::<OssAudioInfo>(), 1180);
    }

    #[test]
    fn request_numbers_match_the_header() {
        assert_eq!(SNDCTL_DSP_HALT, 0x0000_5000);
        assert_eq!(SNDCTL_DSP_SPEED, 0x6004_5002);
        assert_eq!(SNDCTL_DSP_SETFMT, 0x6004_5005);
        assert_eq!(SNDCTL_DSP_CHANNELS, 0x6004_5006);
        assert_eq!(SNDCTL_DSP_SETFRAGMENT, 0x6004_500a);
        assert_eq!(SNDCTL_DSP_GETOSPACE, 0x2010_500c);
        assert_eq!(SNDCTL_DSP_GETISPACE, 0x2010_500d);
        assert_eq!(SNDCTL_DSP_GETODELAY, 0x2004_5017);
        assert_eq!(SNDCTL_DSP_GETERROR, 0x2068_5019);
        assert_eq!(SNDCTL_SYSINFO, 0x24e0_5801);
        assert_eq!(SNDCTL_AUDIOINFO, 0x649c_5807);
    }

    #[test]
    fn c_strings_stop_at_the_terminator() {
        let mut raw = [0 as c_char; 8];
        raw[0] = b'd' as c_char;
        raw[1] = b's' as c_char;
        raw[2] = b'p' as c_char;
        assert_eq!(c_chars_to_string(&raw), "dsp");
    }
}
