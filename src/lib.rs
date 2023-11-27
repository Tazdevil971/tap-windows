//! # tap-windows
//! Library to interface with the tap-windows driver
//! created by OpenVPN to manage tap interfaces.
//! Look at the documentation for `Device` for a
//! pretty simple example on how to use this library.
#![cfg(windows)]

mod ffi;
mod iface;
mod netsh;

use std::{io, net, time};
use windows::Win32::{
    Foundation::HANDLE,
    NetworkManagement::Ndis::NET_LUID_LH,
    System::Ioctl::{FILE_ANY_ACCESS, FILE_DEVICE_UNKNOWN, METHOD_BUFFERED},
};

/// tap-windows hardware ID
pub const HARDWARE_ID: &str = "tap0901";

/// A tap-windows device handle, it offers facilities to:
/// - create, open and delete interfaces
/// - write and read the current configuration
/// - write and read packets from the device
///
/// Example
/// ```no_run
/// use tap_windows::{Device, HARDWARE_ID};
/// use std::io::Read;
///
/// const MY_INTERFACE: &str = "My Interface";
///
/// // Try to open the device
/// let mut dev = Device::open(HARDWARE_ID, MY_INTERFACE)
///     .or_else(|_| -> std::io::Result<_> {
///         // The device does not exists...
///         // try creating a new one
///         
///         let dev = Device::create(HARDWARE_ID)?;
///         dev.set_name(MY_INTERFACE)?;
///     
///         Ok(dev)
///     })
///     // Everything failed, just panic
///     .expect("Failed to open device");
///
/// dev.up().unwrap();
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
    luid: NET_LUID_LH,
    handle: HANDLE,
    component_id: String,
}

impl Device {
    /// Creates a new tap-windows device
    ///
    /// Example
    /// ```no_run
    /// use tap_windows::{Device, HARDWARE_ID};
    ///
    /// let dev = Device::create(HARDWARE_ID)
    ///     .expect("Failed to create device");
    ///
    /// println!("{:?}", dev.get_name());
    /// ```
    pub fn create(component_id: &str) -> io::Result<Self> {
        let luid = iface::create_interface(component_id)?;

        // Even after retrieving the luid, we might need to wait
        let start = time::Instant::now();
        let handle = loop {
            // If we surpassed 2 seconds just return
            let now = time::Instant::now();
            if now - start > time::Duration::from_secs(2) {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Interface timed out"));
            }

            match iface::open_interface(&luid) {
                Err(_) => {
                    std::thread::yield_now();
                    continue;
                }
                Ok(handle) => break handle,
            };
        };

        Ok(Self {
            luid,
            handle,
            component_id: component_id.to_owned(),
        })
    }

    /// Opens an existing tap-windows device by name
    ///
    /// Example
    /// ```no_run
    /// use tap_windows::{Device, HARDWARE_ID};
    ///
    /// let dev = Device::open(HARDWARE_ID, "My Own Device")
    ///     .expect("Failed to open device");
    ///
    /// println!("{:?}", dev.get_name());
    /// ```
    pub fn open(component_id: &str, name: &str) -> io::Result<Self> {
        let luid = ffi::alias_to_luid(name)?;
        iface::check_interface(component_id, &luid)?;

        let handle = iface::open_interface(&luid)?;

        Ok(Self {
            luid,
            handle,
            component_id: component_id.to_owned(),
        })
    }

    /// Deletes the interface before closing it.
    /// By default interfaces are never deleted on Drop,
    /// with this you can choose if you want deletion or not
    ///
    /// Example
    /// ```no_run
    /// use tap_windows::{Device, HARDWARE_ID};
    ///
    /// let dev = Device::create(HARDWARE_ID)
    ///     .expect("Failed to create device");
    ///
    /// println!("{:?}", dev.get_name());
    ///
    /// // Perform a quick cleanup before exiting
    /// dev.delete().expect("Failed to delete device");
    /// ```
    pub fn delete(self) -> io::Result<()> {
        iface::delete_interface(&self.component_id, &self.luid)?;

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
        ffi::device_io_control(self.handle, TAP_IOCTL_GET_MAC, &(), &mut mac).map(|_| mac)
    }

    /// Retrieve the version of the driver
    pub fn get_version(&self) -> io::Result<[u64; 3]> {
        let in_version: [u64; 3] = [0; 3];
        let mut out_version: [u64; 3] = [0; 3];
        ffi::device_io_control(self.handle, TAP_IOCTL_GET_VERSION, &in_version, &mut out_version).map(|_| out_version)
    }

    /// Retieve the mtu of the interface
    pub fn get_mtu(&self) -> io::Result<u32> {
        let in_mtu: u32 = 0;
        let mut out_mtu = 0;
        ffi::device_io_control(self.handle, TAP_IOCTL_GET_MTU, &in_mtu, &mut out_mtu).map(|_| out_mtu)
    }

    /// Retrieve the name of the interface
    pub fn get_name(&self) -> io::Result<String> {
        ffi::luid_to_alias(&self.luid)
    }

    /// Set the name of the interface
    pub fn set_name(&self, newname: &str) -> io::Result<()> {
        let name = self.get_name()?;
        netsh::set_interface_name(&name, newname)
    }

    /// Set the ip of the interface
    /// ```no_run
    /// use tap_windows::{Device, HARDWARE_ID};
    ///
    /// let dev = Device::create(HARDWARE_ID)
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
        let mut out_status: u32 = 0;
        ffi::device_io_control(self.handle, TAP_IOCTL_SET_MEDIA_STATUS, &status, &mut out_status)
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

#[allow(non_snake_case)]
#[inline]
const fn CTL_CODE(DeviceType: u32, Function: u32, Method: u32, Access: u32) -> u32 {
    (DeviceType << 16) | (Access << 14) | (Function << 2) | Method
}

const TAP_IOCTL_GET_MAC: u32 = CTL_CODE(FILE_DEVICE_UNKNOWN, 1, METHOD_BUFFERED, FILE_ANY_ACCESS);
const TAP_IOCTL_GET_VERSION: u32 = CTL_CODE(FILE_DEVICE_UNKNOWN, 2, METHOD_BUFFERED, FILE_ANY_ACCESS);
const TAP_IOCTL_GET_MTU: u32 = CTL_CODE(FILE_DEVICE_UNKNOWN, 3, METHOD_BUFFERED, FILE_ANY_ACCESS);
const TAP_IOCTL_SET_MEDIA_STATUS: u32 = CTL_CODE(FILE_DEVICE_UNKNOWN, 6, METHOD_BUFFERED, FILE_ANY_ACCESS);
// const TAP_IOCTL_CONFIG_TUN: u32 = CTL_CODE(FILE_DEVICE_UNKNOWN, 10, METHOD_BUFFERED, FILE_ANY_ACCESS);
