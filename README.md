# tap-windows
Library to interface with the tap-windows driver created by OpenVPN to manage tap interfaces.

## Install
Add this to your `[dependencies]` in `Cargo.toml`
```toml
tap-windows = "0.1"
```
Alternatively you can install it by running `cargo add tap-windows`.

## Usage
Check the documentation for `Device` for a simple usage example.

## Features
Currently this implementation lacks many features. Here is a list of currently implemented (and unimplemented but planned) features:
- [x] Creating/opening/deleting interfaces.
- [x] Reading and writing from an interface.
- [x] Read driver configuration (mtu, version, mac).
- [x] Write interface ip configuration (set interface ip/mask).
- [ ] Read interface ip configuration (get interface ip/mask).
- [ ] Tun emulation mode.
- [ ] Async read/write.
- [ ] Drop netsh for interface configuration (maybe switch to wmi?).