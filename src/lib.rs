//! # tap-windows
//! Library to interface with the tap-windows driver
//! created by OpenVPN to manage tap interfaces.
//! Look at the documentation for `Device` for a
//! pretty simple example on how to use this library.
#![cfg(windows)]

/// Encode a string as a utf16 buffer
fn encode_utf16(string: &str) -> Vec<u16> {
    use std::iter::once;
    string.encode_utf16().chain(once(0)).collect()
}

/// Decode a string from a utf16 buffer
fn decode_utf16(string: &[u16]) -> String {
    let end = string.iter().position(|b| *b == 0).unwrap_or(string.len());
    String::from_utf16_lossy(&string[..end])
}

mod ffi;
mod iface;
mod netsh;

use std::{io, net, time};
use winapi::shared::ifdef::NET_LUID;
use winapi::um::winioctl::*;
use winapi::um::winnt::HANDLE;

/// A tap-windows device handle, it offers facilities to:
/// - create, open and delete interfaces
/// - write and read the current configuration
/// - write and read packets from the device
/// Example
/// ```no_run
/// use tap_windows::Device;
/// use std::io::Read;
///
/// const MY_INTERFACE: &str = "My Interface";
///
/// // Try to open the device
/// let mut dev = Device::open(MY_INTERFACE)
///     .or_else(|_| -> std::io::Result<_> {
///         // The device does not exists...
///         // try creating a new one
///         
///         let dev = Device::create()?;
///         dev.set_name(MY_INTERFACE)?;
///     
///         Ok(dev)
///     })
///     // Everything failed, just panic
///     .expect("Failed to open device");
///
/// // Set the device ip
/// dev.set_ip([192, 168, 60, 1], [255, 255, 255, 0])
///     .expect("Failed to set device ip");
///
/// // Setup read buffer
/// let mtu = dev.get_mtu().unwrap_or(1500);
/// let mut buf = vec![0; mtu as usize];
///
/// // Read a single packet from the device
/// let amt = dev.read(&mut buf)
///     .expect("Failed to read packet");
///
/// // Print it
/// println!("{:#?}", &buf[..amt]);
/// ```
pub struct Device {
    luid: NET_LUID,
    handle: HANDLE,
}

impl Device {
    /// Creates a new tap-windows device
    /// Example
    /// ```no_run
    /// use tap_windows::Device;
    ///
    /// let dev = Device::create()
    ///     .expect("Failed to create device");
    ///
    /// println!("{:?}", dev.get_name());
    /// ```
    pub fn create() -> io::Result<Self> {
        let luid = iface::create_interface()?;

        // Even after retrieving the luid, we might need to wait
        let start = time::Instant::now();
        let handle = loop {
            // If we surpassed 2 seconds just return
            let now = time::Instant::now();
            if now - start > time::Duration::from_secs(2) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Interface timed out",
                ));
            }

            match iface::open_interface(&luid) {
                Err(_) => {
                    std::thread::yield_now();
                    continue;
                }
                Ok(handle) => break handle,
            };
        };

        Ok(Self { luid, handle })
    }

    /// Opens an existing tap-windows device by name
    /// Example
    /// ```no_run
    /// use tap_windows::Device;
    ///
    /// let dev = Device::open("My Own Device")
    ///     .expect("Failed to open device");
    ///
    /// println!("{:?}", dev.get_name());
    /// ```
    pub fn open(name: &str) -> io::Result<Self> {
        let name = encode_utf16(name);

        let luid = ffi::alias_to_luid(&name)?;
        iface::check_interface(&luid)?;

        let handle = iface::open_interface(&luid)?;

        Ok(Self { luid, handle })
    }

    /// Deletes the interface before closing it.
    /// By default interfaces are never deleted on Drop,
    /// with this you can choose if you want deletion or not
    /// Example
    /// ```no_run
    /// use tap_windows::Device;
    ///
    /// let dev = Device::create()
    ///     .expect("Failed to create device");
    ///
    /// println!("{:?}", dev.get_name());
    ///
    /// // Perform a quick cleanup before exiting
    /// dev.delete().expect("Failed to delete device");
    /// ```
    pub fn delete(self) -> io::Result<()> {
        iface::delete_interface(&self.luid)?;

        Ok(())
    }

    /// Sets the status of the interface to connected.
    /// Equivalent to `.set_status(true)`
    pub fn up(&self) -> io::Result<()> {
        self.set_status(true)
    }

    /// Sets the status of the interface to disconnected.
    /// Equivalent to `.set_status(false)`
    pub fn down(&self) -> io::Result<()> {
        self.set_status(false)
    }

    /// Retieve the mac of the interface
    pub fn get_mac(&self) -> io::Result<[u8; 6]> {
        let mut mac = [0; 6];

        ffi::device_io_control(
            self.handle,
            CTL_CODE(FILE_DEVICE_UNKNOWN, 1, METHOD_BUFFERED, FILE_ANY_ACCESS),
            &(),
            &mut mac,
        )
        .map(|_| mac)
    }

    /// Retrieve the version of the driver
    pub fn get_version(&self) -> io::Result<[u32; 3]> {
        let mut version = [0; 3];

        ffi::device_io_control(
            self.handle,
            CTL_CODE(FILE_DEVICE_UNKNOWN, 2, METHOD_BUFFERED, FILE_ANY_ACCESS),
            &(),
            &mut version,
        )
        .map(|_| version)
    }

    /// Retieve the mtu of the interface
    pub fn get_mtu(&self) -> io::Result<u32> {
        let mut mtu = 0;

        ffi::device_io_control(
            self.handle,
            CTL_CODE(FILE_DEVICE_UNKNOWN, 3, METHOD_BUFFERED, FILE_ANY_ACCESS),
            &(),
            &mut mtu,
        )
        .map(|_| mtu)
    }

    /// Retrieve the name of the interface
    pub fn get_name(&self) -> io::Result<String> {
        ffi::luid_to_alias(&self.luid).map(|name| decode_utf16(&name))
    }

    /// Set the name of the interface
    pub fn set_name(&self, newname: &str) -> io::Result<()> {
        let name = self.get_name()?;
        netsh::set_interface_name(&name, newname)
    }

    /// Set the ip of the interface
    /// ```no_run
    /// use tap_windows::Device;
    ///
    /// let dev = Device::create()
    ///     .expect("Failed to create device");
    ///
    /// dev.set_ip([192, 168, 60, 1], [255, 255, 255, 0])
    ///     .expect("Failed to set interface ip");
    ///
    /// println!("{:?}", dev.get_name());
    /// ```
    pub fn set_ip<A, B>(&self, address: A, mask: B) -> io::Result<()>
    where
        A: Into<net::Ipv4Addr>,
        B: Into<net::Ipv4Addr>,
    {
        let name = self.get_name()?;
        let address = address.into().to_string();
        let mask = mask.into().to_string();

        netsh::set_interface_ip(&name, &address, &mask)
    }

    /// Set the status of the interface, true for connected,
    /// false for disconnected.
    pub fn set_status(&self, status: bool) -> io::Result<()> {
        let status: u32 = if status { 1 } else { 0 };

        ffi::device_io_control(
            self.handle,
            CTL_CODE(FILE_DEVICE_UNKNOWN, 6, METHOD_BUFFERED, FILE_ANY_ACCESS),
            &status,
            &mut (),
        )
    }
}

impl io::Read for Device {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        ffi::read_file(self.handle, buf).map(|res| res as _)
    }
}

impl io::Write for Device {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        ffi::write_file(self.handle, buf).map(|res| res as _)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = ffi::close_handle(self.handle);
    }
}
