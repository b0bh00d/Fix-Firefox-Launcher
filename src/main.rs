// The default launch options for Firefox appear to be immutable
// (https://bugzilla.mozilla.org/show_bug.cgi?id=1758732).
//
// Since I'm the only one who seems interested in using custom options, I
// decided to just re-purpose my existing Chrome solution
// (https://github.com/b0bh00d/Fix-Chrome-Launcher) instead of bruising
// my forehead.
//
// For giggles, this time I did it in Rust instead of Go.

#[macro_use]
extern crate windows_service;
extern crate winreg;
#[macro_use]
extern crate log;
extern crate argmap;
extern crate eventlog;
extern crate regex;

static APP_NAME: &str = "FixFirefoxLauncher";
static DEFAULT_OPTIONS: &str = "-private-window \"%1\"";
static DEFAULT_INTERVAL: u32 = 60;

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{thread, time::Duration, time::Instant};

use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

use winreg::{enums::*, RegKey};

define_windows_service!(ffi_service_main, service_main);

struct ServiceData {
    install_id: String,
    options: String,
    key_types: Vec<String>,
    interval: u32,
}

fn extract_executable(s: &str) -> String {
    let re = regex::Regex::new("\"(.+?)\"").unwrap();
    match re.find(s) {
        Some(m) => m.as_str().to_string(),
        None => String::new(),
    }
}

fn set_service_state(
    status_handle: &service_control_handler::ServiceStatusHandle,
    state: windows_service::service::ServiceState,
) -> windows_service::Result<()> {
    let next_status = ServiceStatus {
        // Should match the one from system service registry
        service_type: ServiceType::OWN_PROCESS,
        // The new state
        current_state: state,
        // Accept stop events when running
        controls_accepted: ServiceControlAccept::STOP,
        // Used to report an error when starting or stopping only, otherwise must be zero
        exit_code: ServiceExitCode::Win32(0),
        // Only used for pending states, otherwise must be zero
        checkpoint: 0,
        // Only used for pending states, otherwise must be zero
        wait_hint: Duration::default(),
        // Unused for setting status
        process_id: None,
    };

    // Tell the system that the service is now running
    status_handle.set_service_status(next_status)
}

fn service_main(arguments: Vec<OsString>) {
    eventlog::register(&APP_NAME).unwrap();
    eventlog::init(&APP_NAME, log::Level::Info).unwrap();

    // create our global data instance
    let mut data = Box::new(ServiceData {
        install_id: String::new(),
        options: String::from(DEFAULT_OPTIONS),
        key_types: vec![],
        interval: DEFAULT_INTERVAL,
    });

    let ff_re = regex::Regex::new("^Firefox(.+?)-(.+?)$").unwrap();
    data.key_types = vec![];

    // locate all the Firefox* entries in the HKCR tree
    for i in RegKey::predef(HKEY_CLASSES_ROOT)
        .enum_keys()
        .map(|x| x.unwrap())
        .filter(|x| x.starts_with("Firefox"))
    {
        // grab the type and suffix values from the key name
        let captures = ff_re.captures(&i).unwrap();

        match captures.get(1) {
            Some(val) => data.key_types.push(String::from(val.as_str())),
            None => warn!("Failed to match type in \"{}\"", i),
        }

        match captures.get(2) {
            Some(val) => data.install_id = String::from(val.as_str()),
            None => warn!("Failed to match key in \"{}\"", i),
        }
    }

    if !data.install_id.is_empty() {
        // if present, get the user-defined settings to override
        // the default interval and options...

        // process argument locations in order of ascending priority...

        // lowest: see if there are any runtime options in proxity to
        // the HKCR:FirefoxHTML command key

        let ff_html_cmd_key = format!(r"FirefoxHTML-{}\shell\open\command", data.install_id);

        match RegKey::predef(HKEY_CLASSES_ROOT).open_subkey(&ff_html_cmd_key) {
            Ok(handle) => {
                // user-defined values may not exist; defaults set above will
                // be used instead in each case

                match handle.get_value("ffl_options") as Result<String, std::io::Error> {
                    Ok(value) => {
                        data.options = value;
                    }
                    // if it's not there, we just use the default
                    Err(_e) => {}
                }

                match handle.get_value("ffl_interval") as Result<u32, std::io::Error> {
                    Ok(value) => {
                        data.interval = value;
                    }
                    // if it's not there, we just use the default
                    Err(_e) => {}
                }
            }
            // we can't open the Firefox key?  that's not good
            Err(value) => {
                error!("{:?}", value);
            }
        }

        // highest: are we receiving args from the Windows Service panel?

        if !arguments.is_empty() && arguments.len() > 1 {
            // convert 'arguments' into a form that argmap will digest

            let v: Vec<_> = arguments
                .into_iter()
                .filter_map(|a| a.into_string().ok())
                .collect();
            let (_args, argv) = argmap::parse(v.into_iter());

            // look for the following arguments:
            //   --ffl_interval=<int>
            //   --ffl_options="<opts>"

            match argv.get("ffl_interval") {
                Some(vec) => match vec[0].parse::<u32>() {
                    Ok(interval) => data.interval = interval,
                    Err(_e) => warn!("Failed to convert {:?} into u32.", vec[0]),
                },
                None => {}
            }
            match argv.get("ffl_options") {
                Some(vec) => data.options = vec[0].clone(),
                None => {}
            }
        }

        info!(
            "Using runtime values: \"{}\", {}",
            data.options, data.interval
        );

        if let Err(value) = run_service(data) {
            error!("{:?}", value);
        }
    } else {
        // tell them we can't find Firefox launch commands in the registry
        warn!("Firefox launch commands were not detected in the registry (Did you actually install it?); exiting.");
    }

    eventlog::deregister(&APP_NAME).unwrap();
}

fn run_service(data: Box<ServiceData>) -> windows_service::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));

    let event_stop = stop.clone();
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                event_stop.store(true, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    // closure to check a generic launch string, and correct it as required
    let check_and_correct = |hkcr: &winreg::RegKey, data: &Box<ServiceData>, key_type: &str| {
        let ff_cmd_key = format!(r"Firefox{}-{}\shell\open\command", key_type, data.install_id);
        match hkcr.open_subkey_with_flags(&ff_cmd_key, KEY_QUERY_VALUE | KEY_SET_VALUE) {
            Ok(reg_key) => {
                let launch_str: String = reg_key.get_value("").unwrap();

                // if the registry key does not contain the required
                // options, then Firefox likely did an update and we
                // need to check that the launcher string is using
                // arguments that we prefer...

                if !launch_str.contains(&data.options) { // <-- this could be more discrete
                    let exec_str = extract_executable(&launch_str);
                    let new_launch_str = format!("{} {}", exec_str, data.options);
                    info!(
                        "Correcting launch string for Firefox{}-{}: \"{}\" -> \"{}\"",
                        key_type,
                        data.install_id,
                        &launch_str[exec_str.len()..],
                        data.options
                    );
                    match reg_key.set_value("", &new_launch_str) {
                        Ok(_) => (),
                        Err(value) => {
                            error!("{:?}", value);
                        }
                    }
                }
            }
            Err(e) => {
                // the key may not have a sub-key that is a launch string...
                warn!("Could not access \"{}\": {:?}", ff_cmd_key, e);
            }
        }
    };

    // Register system service event handler
    let status_handle = service_control_handler::register(&APP_NAME, event_handler)?;

    set_service_state(&status_handle, ServiceState::Running)?;

    let mut now = Instant::now();

    while !stop.load(Ordering::SeqCst) {
        // give up our time slice
        thread::sleep(Duration::from_secs(1));

        // see if our working interval has elapsed
        let elapsed = now.elapsed();
        if elapsed.as_secs() as u32 >= data.interval {
            let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);

            for t in &data.key_types {
                check_and_correct(&hkcr, &data, &t)
            }

            // reset our interval timer
            now = Instant::now();
        }
    }

    // gracefully shut down

    set_service_state(&status_handle, ServiceState::StopPending)?;

    // perform any required clean-up here
    thread::sleep(Duration::from_secs(1));

    set_service_state(&status_handle, ServiceState::Stopped)?;

    Ok(())
}

fn main() -> windows_service::Result<()> {
    // Register generated `ffi_service_main` with the system and start the
    // service, blocking this thread until the service is stopped.
    service_dispatcher::start(&APP_NAME, ffi_service_main)?;
    Ok(())
}
