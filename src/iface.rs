use crate::ffi;
use scopeguard::{guard, ScopeGuard};
use std::io;
use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            DICD_GENERATE_ID, DICS_FLAG_GLOBAL, DIF_INSTALLDEVICE, DIF_INSTALLINTERFACES, DIF_REGISTERDEVICE,
            DIF_REGISTER_COINSTALLERS, DIF_REMOVE, DIGCF_PRESENT, DIREG_DRV, SPDIT_COMPATDRIVER, SPDRP_HARDWAREID,
        },
        Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, TRUE},
        NetworkManagement::Ndis::NET_LUID_LH,
        Storage::FileSystem::{FILE_ATTRIBUTE_SYSTEM, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
        System::Registry::{HKEY, KEY_NOTIFY, KEY_QUERY_VALUE, REG_NOTIFY_CHANGE_NAME},
    },
};

const GUID_NETWORK_ADAPTER: GUID = GUID::from_values(
    0x4d36e972,
    0xe325,
    0x11ce,
    [0xbf, 0xc1, 0x08, 0x00, 0x2b, 0xe1, 0x03, 0x18],
);

#[repr(C, align(1))]
#[derive(c2rust_bitfields::BitfieldStruct)]
#[allow(non_snake_case)]
#[allow(non_camel_case_types)]
struct _NET_LUID_LH {
    #[bitfield(name = "Reserved", ty = "u64", bits = "0..=23")]
    #[bitfield(name = "NetLuidIndex", ty = "u64", bits = "24..=47")]
    #[bitfield(name = "IfType", ty = "u64", bits = "48..=63")]
    _Value: [u8; 8],
}

/// Create a new interface and returns its NET_LUID
pub fn create_interface(component_id: &str) -> io::Result<NET_LUID_LH> {
    let devinfo = ffi::create_device_info_list(&GUID_NETWORK_ADAPTER)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_device_info_list(devinfo);
    });

    let class_name = ffi::class_name_from_guid(&GUID_NETWORK_ADAPTER)?;

    let mut devinfo_data = ffi::create_device_info(devinfo, &class_name, &GUID_NETWORK_ADAPTER, "", DICD_GENERATE_ID)?;

    ffi::set_selected_device(devinfo, &devinfo_data)?;
    ffi::set_device_registry_property(devinfo, &devinfo_data, SPDRP_HARDWAREID, Some(component_id))?;

    ffi::build_driver_info_list(devinfo, &mut devinfo_data, SPDIT_COMPATDRIVER.0)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_driver_info_list(devinfo, &devinfo_data, SPDIT_COMPATDRIVER.0);
    });

    let mut driver_version = 0;
    let mut member_index = 0;

    while let Some(drvinfo_data) = ffi::enum_driver_info(devinfo, &devinfo_data, SPDIT_COMPATDRIVER.0, member_index) {
        member_index += 1;

        if drvinfo_data.is_err() {
            continue;
        }
        let drvinfo_data = drvinfo_data?;
        if drvinfo_data.DriverVersion <= driver_version {
            continue;
        }

        let drvinfo_detail = match ffi::get_driver_info_detail(devinfo, &devinfo_data, &drvinfo_data) {
            Ok(drvinfo_detail) => drvinfo_detail,
            _ => continue,
        };

        let hardware_id = unsafe {
            PCWSTR(drvinfo_detail.HardwareID.as_ptr())
                .to_string()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
        };
        if !hardware_id.eq_ignore_ascii_case(component_id) {
            continue;
        }

        if ffi::set_selected_driver(devinfo, &devinfo_data, &drvinfo_data).is_err() {
            continue;
        }

        driver_version = drvinfo_data.DriverVersion;
    }

    if driver_version == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "No driver found"));
    }

    let uninstaller = guard((), |_| {
        let _ = ffi::call_class_installer(devinfo, &devinfo_data, DIF_REMOVE);
    });

    ffi::call_class_installer(devinfo, &devinfo_data, DIF_REGISTERDEVICE)?;

    let _ = ffi::call_class_installer(devinfo, &devinfo_data, DIF_REGISTER_COINSTALLERS);
    let _ = ffi::call_class_installer(devinfo, &devinfo_data, DIF_INSTALLINTERFACES);

    ffi::call_class_installer(devinfo, &devinfo_data, DIF_INSTALLDEVICE)?;

    let key = ffi::open_dev_reg_key(
        devinfo,
        &devinfo_data,
        DICS_FLAG_GLOBAL,
        0,
        DIREG_DRV,
        KEY_QUERY_VALUE.0 | KEY_NOTIFY.0,
    )?;

    let key = winreg::RegKey::predef(key.0);

    while key.get_value::<u32, &str>("*IfType").is_err() {
        ffi::notify_change_key_value(HKEY(key.raw_handle()), TRUE, REG_NOTIFY_CHANGE_NAME.0, 2000)?;
    }

    while key.get_value::<u32, &str>("NetLuidIndex").is_err() {
        ffi::notify_change_key_value(HKEY(key.raw_handle()), TRUE, REG_NOTIFY_CHANGE_NAME.0, 2000)?;
    }

    let if_type: u32 = key.get_value("*IfType")?;
    let luid_index: u32 = key.get_value("NetLuidIndex")?;

    // Defuse the uninstaller
    ScopeGuard::into_inner(uninstaller);

    let mut luid = NET_LUID_LH { Value: 0 };

    unsafe {
        let luid = &mut luid as *mut NET_LUID_LH as *mut _NET_LUID_LH;
        (*luid).set_IfType(if_type as _);
        (*luid).set_NetLuidIndex(luid_index as _);
    }

    Ok(luid)
}

/// Check if the given interface exists and is a valid network device
pub fn check_interface(component_id: &str, luid: &NET_LUID_LH) -> io::Result<()> {
    let devinfo = ffi::get_class_devs(&GUID_NETWORK_ADAPTER, DIGCF_PRESENT)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_device_info_list(devinfo);
    });

    let mut member_index = 0;

    while let Some(devinfo_data) = ffi::enum_device_info(devinfo, member_index) {
        member_index += 1;

        if devinfo_data.is_err() {
            continue;
        }
        let devinfo_data = devinfo_data?;

        let hardware_id = ffi::get_device_registry_property(devinfo, &devinfo_data, SPDRP_HARDWAREID);
        if hardware_id.is_err() {
            continue;
        }
        if !hardware_id?.eq_ignore_ascii_case(component_id) {
            continue;
        }

        let key = match ffi::open_dev_reg_key(
            devinfo,
            &devinfo_data,
            DICS_FLAG_GLOBAL,
            0,
            DIREG_DRV,
            KEY_QUERY_VALUE.0 | KEY_NOTIFY.0,
        ) {
            Ok(key) => winreg::RegKey::predef(key.0),
            Err(_) => continue,
        };

        let if_type: u32 = match key.get_value("*IfType") {
            Ok(if_type) => if_type,
            Err(_) => continue,
        };

        let luid_index: u32 = match key.get_value("NetLuidIndex") {
            Ok(luid_index) => luid_index,
            Err(_) => continue,
        };

        let mut luid2 = NET_LUID_LH { Value: 0 };

        unsafe {
            let luid2 = &mut luid2 as *mut NET_LUID_LH as *mut _NET_LUID_LH;
            (*luid2).set_IfType(if_type as _);
            (*luid2).set_NetLuidIndex(luid_index as _);
        }

        if unsafe { luid.Value != luid2.Value } {
            continue;
        }

        // Found it!
        return Ok(());
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "Device not found"))
}

/// Deletes an existing interface
pub fn delete_interface(component_id: &str, luid: &NET_LUID_LH) -> io::Result<()> {
    let devinfo = ffi::get_class_devs(&GUID_NETWORK_ADAPTER, DIGCF_PRESENT)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_device_info_list(devinfo);
    });

    let mut member_index = 0;

    while let Some(devinfo_data) = ffi::enum_device_info(devinfo, member_index) {
        member_index += 1;

        if devinfo_data.is_err() {
            continue;
        }
        let devinfo_data = devinfo_data?;

        let hardware_id = ffi::get_device_registry_property(devinfo, &devinfo_data, SPDRP_HARDWAREID);
        if hardware_id.is_err() {
            continue;
        }
        if !hardware_id?.eq_ignore_ascii_case(component_id) {
            continue;
        }

        let key = ffi::open_dev_reg_key(
            devinfo,
            &devinfo_data,
            DICS_FLAG_GLOBAL,
            0,
            DIREG_DRV,
            KEY_QUERY_VALUE.0 | KEY_NOTIFY.0,
        );
        if key.is_err() {
            continue;
        }
        let key = winreg::RegKey::predef(key?.0);

        let if_type: u32 = match key.get_value("*IfType") {
            Ok(if_type) => if_type,
            Err(_) => continue,
        };

        let luid_index: u32 = match key.get_value("NetLuidIndex") {
            Ok(luid_index) => luid_index,
            Err(_) => continue,
        };

        let mut luid2 = NET_LUID_LH { Value: 0 };

        unsafe {
            let luid2 = &mut luid2 as *mut NET_LUID_LH as *mut _NET_LUID_LH;
            (*luid2).set_IfType(if_type as _);
            (*luid2).set_NetLuidIndex(luid_index as _);
        }

        if unsafe { luid.Value != luid2.Value } {
            continue;
        }

        // Found it!
        return ffi::call_class_installer(devinfo, &devinfo_data, DIF_REMOVE);
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "Device not found"))
}

/// Open an handle to an interface
pub fn open_interface(luid: &NET_LUID_LH) -> io::Result<HANDLE> {
    let guid = ffi::luid_to_guid(luid).and_then(|guid| ffi::string_from_guid(&guid))?;

    let path = format!(r"\\.\Global\{}.tap", guid);

    ffi::create_file(
        &path,
        GENERIC_READ.0 | GENERIC_WRITE.0,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_SYSTEM,
    )
}
