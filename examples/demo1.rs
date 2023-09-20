use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};
use tap_windows::{Device, HARDWARE_ID};
use windows::{
    core::HRESULT,
    Win32::Foundation::{ERROR_GEN_FAILURE, ERROR_INVALID_PARAMETER},
};

const MY_INTERFACE: &str = "MyInterface";

fn main() -> std::io::Result<()> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut dev = Device::open(HARDWARE_ID, MY_INTERFACE);
    if let Err(e) = dev {
        if e.raw_os_error() == Some(HRESULT::from(ERROR_INVALID_PARAMETER).0) {
            log::trace!("Device is not exist, try creating a new one");
            let new_dev = Device::create(HARDWARE_ID)?;
            new_dev.set_name(MY_INTERFACE)?;
            dev = Ok(new_dev);
        } else {
            if e.raw_os_error() == Some(HRESULT::from(ERROR_GEN_FAILURE).0) {
                log::error!("Device is already in use, exiting...");
            }
            return Err(e);
        }
    }
    let mut dev = dev?;

    dev.up()?;

    // Set the device ip
    dev.set_ip([10, 20, 60, 1], [255, 255, 255, 0])?;

    // Setup read buffer
    let mtu = dev.get_mtu().unwrap_or(1500);

    static RUNNING: AtomicBool = AtomicBool::new(true);

    let _main_loop = std::thread::spawn(move || {
        let mut buf = vec![0; mtu as usize];
        while RUNNING.load(Ordering::Relaxed) {
            let amt = dev.read(&mut buf)?;

            let data = &buf[..amt];
            let len = data.len();
            let header = &data[0..(20.min(len))];
            println!("Read packet size {} bytes. Header data {:?}", len, header);
        }
        Ok::<(), std::io::Error>(())
    });

    {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = ctrlc2::set_handler(move || {
            tx.send(()).expect("Could not send signal.");
            true
        })
        .expect("Error setting Ctrl-C handler");
        println!("Press Ctrl-C to stop session");
        rx.recv().expect("Could not receive from channel.");
        handle.join().expect("Could not join Ctrl-C handler thread");
    }

    RUNNING.store(false, Ordering::Relaxed);

    println!("Shutdown complete");

    // _main_loop.join().unwrap()?;

    Ok(())
}
