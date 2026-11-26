# illumos backend

A PipeWire SPA plugin for illumos audio devices, built on this repository's
backend-neutral core. It talks to the illumos audio framework through its OSS
interface (`/dev/dsp`); the framework converts rate, format and channel count
in the kernel.

State: first cut. Playback and capture streams, a static device list
(`/dev/dsp`), no hot-plug, no hardware volume (the graph applies volume in
software), wake-ups from the shared timer path.

    cargo build --release -p spa-kitchen-sink-illumos-oss

produces `target/release/libspa_illumos_oss.so`, to be installed in PipeWire's
SPA plugin directory.

With WirePlumber (the normal case) install three files:

    conf/pipewire.conf.d/illumos-oss.conf       -> pipewire.conf.d/
    conf/wireplumber.conf.d/illumos-oss.conf    -> wireplumber.conf.d/
    share/wireplumber/scripts/monitors/illumos-oss.lua
                                                -> wireplumber/scripts/monitors/

The WirePlumber fragment loads the device monitor as the provider of
`hardware.audio` and disables the monitors that only exist on Linux (ALSA,
Bluetooth, cameras, logind). Both daemons must find the plugin (`SPA_PLUGIN_DIR`
if it is not in the default directory).

`conf/illumos-oss-static.conf` instead creates one fixed sink without a session
manager; do not install it together with the files above. `abi/compare.c` prints the OSS request numbers and structure
sizes from the system header for comparison with `src/illumos_oss/abi.rs`.

The other crates in this workspace are FreeBSD-only; build this one by name.
