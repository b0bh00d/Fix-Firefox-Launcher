// The default launch options for Firefox appear to be immutable
// (https://bugzilla.mozilla.org/show_bug.cgi?id=1758732).
//
// Since I'm the only one who seems interested in using custom options, I
// decided to just re-purpose my existing Chrome solution
// (https://github.com/b0bh00d/Fix-Chrome-Launcher) instead of bruising
// my forehead.
//
// For giggles, this time I did it in Rust instead of Go.

#[macro_use] extern crate windows_service;
extern crate winreg;
#[macro_use] extern crate log;
extern crate eventlog;
extern crate regex;
extern crate argmap;

static APP_NAME : &str = "FixFirefoxLauncher";
static DEFAULT_OPTIONS : &str = "-private-window \"%1\"";
static DEFAULT_INTERVAL : u32 = 60;

use std::ffi::OsString;
use std::{
    thread,
    time::Instant,
    time::Duration,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_service::service::{
    ServiceControl, ServiceControlAccept,
    ServiceExitCode, ServiceState,
    ServiceStatus, ServiceType,
};
use windows_service::service_dispatcher;
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

use winreg::{RegKey, enums::*};

define_windows_service!(ffi_service_main, service_main);

struct ServiceData {
    key: String,
    options: String,
    interval: u32,
}

fn extract_executable(s : &str) -> String {
    let re = regex::Regex::new("\"(.+?)\"").unwrap();
    for cap in re.captures_iter(s) {
        return cap[0].to_string();
    }

    String::new()
}

fn set_service_state(status_handle: &service_control_handler::ServiceStatusHandle, state: windows_service::service::ServiceState) -> windows_service::Result<()> {
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
    let mut data = Box::new(ServiceData{key: String::new(), options: String::from(DEFAULT_OPTIONS), interval: DEFAULT_INTERVAL});

    // locate the Firefox entries in the HKCR tree
    for i in RegKey::predef(HKEY_CLASSES_ROOT)
        .enum_keys().map(|x| x.unwrap())
        .filter(|x| x.starts_with("FirefoxHTML"))
    {
        // grab the key suffix value from the name
        let items = i.split("-").collect::<Vec<_>>();
        if items.len() > 1 {
            data.key = items[1].to_string();
        }
        else {
            warn!("\"{}\" was not in the expected format; exiting.", i);
        }
    }

    if !data.key.is_empty() {
        // if present, get the user-defined settings to override
        // the default interval and options...

        // process argument locations in order of ascending priority...

        // lowest: see if there are any runtime options in proxity to
        // the HKCR:FirefoxHTML command key

        let ff_html_cmd_key = format!(r"FirefoxHTML-{}\shell\open\command", data.key);

        match RegKey::predef(HKEY_CLASSES_ROOT).open_subkey(&ff_html_cmd_key) {
            Ok(handle) => {
                // user-defined values may not exist; defaults set above will
                // be used instead in each case

                match handle.get_value("ffl_options") as Result<String, std::io::Error> {
                    Ok(value) => {
                        data.options = value;
                    },
                    // if it's not there, we just use the default
                    Err(_e) => {
                    },
                }

                match handle.get_value("ffl_interval") as Result<u32, std::io::Error> {
                    Ok(value) => {
                        data.interval = value;
                    },
                    // if it's not there, we just use the default
                    Err(_e) => {
                    },
                }
            },
            // we can't open the Firefox key?  that's not good
            Err(value) => {
                error!("{:?}", value);
            },
        }

        // highest: are we receiving args from the Windows Service panel?

        if !arguments.is_empty() && arguments.len() > 1 {
            // convert 'arguments' into a form that argmap will digest
            let mut v = vec![];

            // note: 'arguments' is implicitly moved here
            for a in arguments {
                match a.into_string() {
                    Ok(s) => { v.push(s) }
                    Err(_e) => {}
                };
            }

            let (_args, argv) = argmap::parse(v.into_iter());

            // look for the following arguments:
            //   --ffl_interval=<int>
            //   --ffl_options="<opts>"

            match argv.get("ffl_interval") {
                Some(vec) => {
                    match vec[0].parse::<u32>() {
                        Ok(interval) => data.interval = interval,
                        Err(_e) => warn!("Failed to convert {:?} into u32.", vec[0])
                    }
                },
                None => {}
            }
            match argv.get("ffl_options") {
                Some(vec) => data.options = vec[0].clone(),
                None => {}
            }
        }

        info!("Using runtime values: \"{}\", {}", data.options, data.interval);

        if let Err(value) = run_service(data) {
            error!("{:?}", value);
        }
    }
    else {
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
            ServiceControl::Interrogate => {
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    // Register system service event handler
    let status_handle = service_control_handler::register(&APP_NAME, event_handler)?;

    set_service_state(&status_handle, ServiceState::Running)?;

    let ff_html_cmd_key = format!(r"FirefoxHTML-{}\shell\open\command", data.key);
    let ff_url_cmd_key = format!(r"FirefoxURL-{}\shell\open\command", data.key);

    let mut now = Instant::now();

    while !stop.load(Ordering::SeqCst) {
        // give up our time slice
        thread::sleep(Duration::from_secs(1));

        // see if our working interval has elapsed
        let elapsed = now.elapsed();
        if elapsed.as_secs() as u32 >= data.interval {
            let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);
            match hkcr.open_subkey_with_flags(&ff_html_cmd_key, KEY_QUERY_VALUE|KEY_SET_VALUE) {
                Ok(reg_key) => {
                    let launch_str : String = reg_key.get_value("").unwrap();
        
                    // if the registry key does not contain the required
                    // options, then Firefox likely did an update and we
                    // need to check that the launcher string is using
                    // arguments that we prefer...

                    if !launch_str.contains(&data.options) {  // <-- this could be more discrete
                        let exec_str = extract_executable(&launch_str);
                        let new_launch_str = format!("{} {}", exec_str, data.options);
                        info!("Setting launch string: \"{}\"", new_launch_str);
                        match reg_key.set_value("", &new_launch_str) {
                            Ok(_) => (),
                            Err(value) => {
                                error!("{:?}", value);
                            }
                        }
                    }
                },
                Err(e) => {
                    error!("Failed to open the '{}' registry value: {}", ff_html_cmd_key, e);
                    stop.store(true, Ordering::SeqCst);
                }
            }

            match hkcr.open_subkey_with_flags(&ff_url_cmd_key, KEY_QUERY_VALUE|KEY_SET_VALUE) {
                Ok(reg_key) => {
                    let launch_str : String = reg_key.get_value("").unwrap();
        
                    // have our user options been removed/overridden?
                    if !launch_str.contains(&data.options) {
                        let exec_str = extract_executable(&launch_str);
                        let new_launch_str = format!("{} {}", exec_str, data.options);
                        match reg_key.set_value("", &new_launch_str) {
                            Ok(_) => (),
                            Err(value) => {
                                error!("{:?}", value);
                            }
                        }
                    }
                },
                Err(e) => {
                    error!("Failed to open the '{}' registry value: {}", ff_url_cmd_key, e);
                    stop.store(true, Ordering::SeqCst);
                }
            }

            // restart our interval timer
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
