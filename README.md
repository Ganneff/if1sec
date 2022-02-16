# if1sec

A munin plugin to collect interface traffic data once every second.

## How
The plugin uses the /sys filesystem and reads TX/RX statistics from
there. Statistics are read once per second and written to a cachefile,
whenever munin asks for the data, the content of the cachefile is
send.

## Usage
Compile (or load a released binary, if one is there) and put the
binary somewhere. Then link it into the munin plugins dir, with a name
that ends in `_INTERFACE`, like `if1sec_eth0` to create graphs of the
_eth0_ interface.

When first called without arguments, if1sec will spawn itself into the
background to gather data. This can also be triggered by calling it
with the `acquire` parameter.

## Local build
Use cargo build as usual. Note that the release build contains much
less logging code than the debug build, so if you want to find out,
why something does not work as planned, ensure to use a debug build
(`cargo build` instead of `cargo build --release`).

### Debian package
A minimal Debian package can be build using `cargo deb`, provided that
you installed this feature (`cargo install cargo-deb`).