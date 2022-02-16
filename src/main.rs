//! if1sec - Collect network interface data for munin every second
//!
//! Use as munin plugin, it expects to be symlinked per interface. So
//! a symlink if1sec_eth0 to this plugin will collect data for the eth0
//! interface.
// SPDX-License-Identifier:  GPL-3.0-only

#![warn(missing_docs)]

use daemonize::Daemonize;
use fs2::FileExt;
use log::{debug, error, info, trace, warn};
use simple_logger::SimpleLogger;
use spin_sleep::LoopHelper;
use std::{
    env,
    error::Error,
    fs::{rename, File, OpenOptions},
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

/// Check the name we are called with and split it on _.
///
/// We ought to be symlinked to if1sec_INTERFACE and work for
/// the given INTERFACE then (say, eth0).
fn get_interface() -> String {
    std::env::args()
        .next()
        .expect("Couldn't get program arguments")
        .split('_')
        .last()
        .expect("Couldn't split arguments into parts")
        .to_string()
}

/// Print out munin config data
///
/// If we can read the interface speed, we hand out real value. If we
/// can not, we assume 1000, as many VMs simply do not show anything
/// real here. And 1000 is a nice value for them, though they don't
/// really have a physical limitation.
fn config(interface: &str) -> Result<(), Box<dyn Error>> {
    // Check network "speed" as shown by VM
    let speedpath = Path::new("/sys/class/net/").join(&interface).join("speed");
    debug!("speed: {:#?}", speedpath);
    let speed: usize = if Path::exists(&speedpath) {
        let rspeed: isize = std::fs::read_to_string(&speedpath)?.trim().parse()?;
        if rspeed <= 0 {
            1000
        } else {
            rspeed as usize
        }
    } else {
        1000
    };
    let max = speed / 8 * 1000000;

    println!("graph_title Interface 1sec stats for {}", interface);
    println!("graph_category network");
    println!("graph_args --base 1000");
    println!("graph_data_size custom 1d, 1s for 1d, 5s for 2d, 10s for 7d, 1m for 1t, 5m for 1y");
    println!("graph_vlabel bits in (-) / out (+)");
    println!("graph_info This graph shows the traffic of the {} network interface. Please note that the traffic is shown in bits per second, not bytes.", interface);
    println!("update_rate 1");
    println!("{0}_rx.label {0} bits", interface);
    println!("{0}_rx.cdef {0}_rx,8,*", interface);
    println!("{}_rx.type DERIVE", interface);
    println!("{}_rx.min 0", interface);
    println!("{}_rx.graph no", interface);
    println!("{}_tx.label bps", interface);
    println!("{0}_tx.cdef {0}_tx,8,*", interface);
    println!("{}_tx.type DERIVE", interface);
    println!("{}_tx.min 0", interface);
    println!("{0}_tx.negative {0}_rx", interface);
    println!("{}_rx.max {}", interface, max);
    println!("{}_tx.max {}", interface, max);
    println!(
        "{0}_rx.info Received traffic on the {0} interface. Maximum speed is {1} Mbps.",
        interface, speed
    );
    println!(
        "{0}_tx.info Transmitted traffic on the {0} interface. Maximum speed {1} Mbps.",
        interface, speed
    );

    Ok(())
}

/// Gather the data from the system.
///
/// Daemonize into background and then run a loop forever, that
/// fetches data once a second and appends it to the given cachefile.
/// All file pathes (statistic files for TX and RX data, cache and
/// pidfile) have to be calculated before and just handed over to this
/// function.
///
/// We read the values from the statistic files and parse them to a
/// u128, that ought to be big enough to not overflow.
fn acquire(
    interface: &str,
    txfile: &Path,
    rxfile: &Path,
    cachefile: &Path,
    pidfile: &Path,
) -> Result<(), Box<dyn Error>> {
    trace!("Going to daemonize");

    let daemonize = Daemonize::new()
        .pid_file(pidfile)
        .chown_pid_file(true)
        .working_directory("/tmp");

    match daemonize.start() {
        Ok(_) => {
            // The loop helper makes it easy to repeat a loop once a second
            let mut loop_helper = LoopHelper::builder().build_with_target_rate(1); // Only once a second

            // We run forever
            loop {
                // Let loop helper prepare
                loop_helper.loop_start();

                // We need the current epoch
                let epoch = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time gone broken, what?")
                    .as_secs(); // without the nanosecond part

                // Read in the received and transferred bytes, store as u128
                let rx: u128 = std::fs::read_to_string(&rxfile)?.trim().parse()?;
                let tx: u128 = std::fs::read_to_string(&txfile)?.trim().parse()?;
                // This block only to ensure we close the cachefd before we go sleep
                {
                    // Open the munin cachefile to store our values
                    let mut cachefd = OpenOptions::new()
                        .create(true) // If not there, create
                        .write(true) // We want to write
                        .append(true) // We want to append
                        .open(cachefile)
                        .expect("Couldn't open file");
                    // And now write out values
                    writeln!(cachefd, "{0}_tx.value {1}:{2}", interface, epoch, tx)?;
                    writeln!(cachefd, "{0}_rx.value {1}:{2}", interface, epoch, rx)?;
                } // cachefile is closed here

                // Sleep for the rest of the second
                loop_helper.loop_sleep();
            }
        }
        Err(e) => {
            error!("Something gone wrong: {}", e);
            Err(Box::new(e))
        }
    }
}

/// Hand out the collected interface data
///
/// Basically a "mv file tmpfile && cat tmpfile && rm tmpfile",
/// as the file is already in proper format
fn fetch(cache: &Path) -> Result<(), Box<dyn Error>> {
    // We need a temporary file
    let fetchpath =
        NamedTempFile::new_in(cache.parent().expect("Could not find useful temp path"))?;
    debug!("Fetchcache: {:?}, Cache: {:?}", fetchpath, cache);
    // Rename the cache file, to ensure that acquire doesn't add data
    // between us outputting data and deleting the file
    rename(&cache, &fetchpath)?;
    // We want to write possibly large amount to stdout, take and lock it
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    // Want to read the tempfile now
    let mut fetchfile = std::fs::File::open(&fetchpath)?;
    // And ask io::copy to just take it all and show it into stdout
    io::copy(&mut fetchfile, &mut handle)?;
    Ok(())
}

/// Manage it all.
///
/// Note that, while we do have extensive logging statements all over
/// the code, we use the crates feature to **not** compile in levels
/// we do not want. So in devel/debug builds, we have all levels
/// including trace! available, release build will only show warn! and
/// error! logs (tiny amount).
fn main() {
    SimpleLogger::new().init().unwrap();
    info!("if1sec started");
    // Store arguments
    let args: Vec<String> = env::args().collect();
    // And see what interface we ought to deal with
    let interface = get_interface();
    if interface.eq(&args[0]) {
        error!("Could not determine interface to work for!");
        std::process::exit(1);
    }
    debug!("I am called for interface {}", interface);
    // Check the data files we are interested in
    let if_txbytes = Path::new("/sys/class/net")
        .join(&interface)
        .join("statistics/tx_bytes");
    let if_rxbytes = Path::new("/sys/class/net")
        .join(&interface)
        .join("statistics/rx_bytes");
    debug!("TX: {:?}", if_txbytes);
    debug!("RX: {:?}", if_rxbytes);

    if !Path::exists(&if_txbytes) {
        error!("Can not find TX input file: {:?}", if_txbytes);
        std::process::exit(2);
    }
    if !Path::exists(&if_rxbytes) {
        error!("Can not find RX input file: {:?}", if_rxbytes);
        std::process::exit(2);
    }

    // Where is our plugin state directory?
    let plugstate = env::var("MUNIN_PLUGSTATE").expect("Could not read MUNIN_PLUGSTATE variable");
    debug!("Plugin State: {:#?}", plugstate);
    // Put our cache file there
    let cache = Path::new(&plugstate).join(format!("munin.if1sec_{}.value", interface));
    debug!("Cache: {:?}", cache);
    let pidfile = Path::new(&plugstate).join(format!("munin.if1sec_{}.pid", interface));
    debug!("PIDfile: {:?}", pidfile);

    // Does the master support dirtyconfig?
    let dirtyconfig = match env::var("MUNIN_CAP_DIRTYCONFIG") {
        Ok(val) => val.eq(&"1"),
        Err(_) => false,
    };
    debug!("Dirtyconfig is: {:?}", dirtyconfig);

    // Now go over our other args and see what we are supposed to do
    match args.len() {
        // no arguments passed, print data
        1 => {
            trace!("No argument, assuming fetch");
            // Before we fetch we should ensure that we have a data
            // gatherer running. It locks the pidfile, so lets see if
            // it's locked or we can have it.
            let lockfile = !Path::exists(&pidfile) || {
                let lockedfile = File::open(&pidfile).expect("Could not open pidfile");
                lockedfile.try_lock_exclusive().is_ok()
            };

            if lockfile {
                // if Path::exists(&pidfile) {
                //     let lockfile = File::open(&pidfile).expect("Could not open pidfile");
                //     if lockfile.try_lock_exclusive().is_ok() {
                //         // Appears we can lock exclusive -> acquire doesn't seem to be running
                debug!("Could lock the pidfile, will spawn acquire now");
                Command::new(&args[0])
                    .arg("acquire".to_owned())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("failed to execute acquire");
                debug!("Spawned, sleep for 1s, then continue");
                // Now we wait one second before going on, so the
                // newly spawned process had a chance to generate us
                // some data
                thread::sleep(Duration::from_secs(1));
                // }
            }
            if let Err(e) = fetch(&cache) {
                error!("Could not fetch data: {}", e);
                std::process::exit(6);
            }
        }

        // one argument passed, check it and do something
        2 => match args[1].as_str() {
            "config" => {
                trace!("Called to hand out config");
                config(&interface).expect("Could not write out config");
                // If munin supports the dirtyconfig feature, we can hand out the data
                if dirtyconfig {
                    if let Err(e) = fetch(&cache) {
                        error!("Could not fetch data: {}", e);
                        std::process::exit(6);
                    }
                };
            }
            "acquire" => {
                trace!("Called to gather data");
                // Only will ever process anything after this line, if
                // one process has our pidfile already locked, ie. if
                // another acquire is running. (Or if we can not
                // daemonize for another reason).
                if let Err(e) = acquire(&interface, &if_txbytes, &if_rxbytes, &cache, &pidfile) {
                    error!("Error: {}", e);
                    std::process::exit(5);
                };
            }
            _ => {
                error!("Unknown command {}", args[1]);
                std::process::exit(3);
            }
        },
        // all the other cases
        _ => {
            error!("Unknown number of arguments");
            std::process::exit(4);
        }
    }
    info!("All done");
}
