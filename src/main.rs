//! if1sec - Collect network interface data for munin every second
//!
//! Use as munin plugin, it expects to be symlinked per interface. So
//! a symlink if1sec_eth0 to this plugin will collect data for the eth0
//! interface.
// SPDX-License-Identifier:  GPL-3.0-only

#![warn(missing_docs)]

use anyhow::Result;
use log::{debug, error, info, warn};
use munin_plugin::{Config, MuninPlugin};
use simple_logger::SimpleLogger;
use std::{
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The struct for our plugin, so we can easily store some values over
/// the lifetime of our plugin.
struct InterfacePlugin {
    /// For which interface we should gather data
    interface: String,

    /// Where to get TXBytes from
    if_txbytes: PathBuf,

    /// Where to get RXBytes from
    if_rxbytes: PathBuf,
}

impl InterfacePlugin {
    /// Check the name we are called with and split it on _.
    fn get_interface() -> String {
        std::env::args()
            .next()
            .expect("Couldn't get program arguments")
            .split('_')
            .last()
            .expect("Couldn't split arguments into parts")
            .to_string()
    }
}

impl Default for InterfacePlugin {
    /// Set defaults
    fn default() -> Self {
        let interface = InterfacePlugin::get_interface();
        let if_rxbytes = Path::new("/sys/class/net")
            .join(&interface)
            .join("statistics/rx_bytes");
        let if_txbytes = Path::new("/sys/class/net")
            .join(&interface)
            .join("statistics/tx_bytes");
        if !Path::exists(&if_txbytes) {
            error!("Can not find TX input file: {:?}", if_txbytes);
            std::process::exit(2);
        }
        if !Path::exists(&if_rxbytes) {
            error!("Can not find RX input file: {:?}", if_rxbytes);
            std::process::exit(2);
        }
        Self {
            interface,
            if_rxbytes,
            if_txbytes,
        }
    }
}

impl MuninPlugin for InterfacePlugin {
    fn config<W: Write>(&self, handle: &mut BufWriter<W>) -> Result<()> {
        // Check network "speed" as shown by VM
        let speedpath = Path::new("/sys/class/net/")
            .join(&self.interface)
            .join("speed");
        debug!("speed: {:#?}", speedpath);
        let speed: usize = if Path::exists(&speedpath) {
            let rspeed: usize = std::fs::read_to_string(&speedpath)
                .unwrap_or_else(|_| "0".to_owned())
                .trim()
                .parse()?;
            if rspeed <= 0 {
                1000
            } else {
                rspeed as usize
            }
        } else {
            1000
        };
        let max = speed / 8 * 1000000;

        writeln!(
            handle,
            "graph_title Interface 1sec stats for {}",
            self.interface
        )?;
        writeln!(handle, "graph_category network")?;
        writeln!(handle, "graph_args --base 1000")?;
        writeln!(
            handle,
            "graph_data_size custom 1d, 1s for 1d, 5s for 2d, 10s for 7d, 1m for 1t, 5m for 1y"
        )?;
        writeln!(handle, "graph_vlabel bits in (-) / out (+)")?;
        writeln!(handle, "graph_info This graph shows the traffic of the {} network self.interface. Please note that the traffic is shown in bits per second, not bytes.", self.interface)?;
        writeln!(handle, "update_rate 1")?;
        writeln!(handle, "{0}_rx.label {0} bits", self.interface)?;
        writeln!(handle, "{0}_rx.cdef {0}_rx,8,*", self.interface)?;
        writeln!(handle, "{}_rx.type DERIVE", self.interface)?;
        writeln!(handle, "{}_rx.min 0", self.interface)?;
        writeln!(handle, "{}_rx.graph no", self.interface)?;
        writeln!(handle, "{}_tx.label bps", self.interface)?;
        writeln!(handle, "{0}_tx.cdef {0}_tx,8,*", self.interface)?;
        writeln!(handle, "{}_tx.type DERIVE", self.interface)?;
        writeln!(handle, "{}_tx.min 0", self.interface)?;
        writeln!(handle, "{0}_tx.negative {0}_rx", self.interface)?;
        writeln!(handle, "{}_rx.max {}", self.interface, max)?;
        writeln!(handle, "{}_tx.max {}", self.interface, max)?;
        writeln!(
            handle,
            "{0}_rx.info Received traffic on the {0} self.interface. Maximum speed is {1} Mbps.",
            self.interface, speed
        )?;
        writeln!(
            handle,
            "{0}_tx.info Transmitted traffic on the {0} self.interface. Maximum speed {1} Mbps.",
            self.interface, speed
        )?;

        Ok(())
    }

    fn acquire<W: Write>(
        &mut self,
        handle: &mut BufWriter<W>,
        _config: &Config,
        epoch: u64,
    ) -> Result<()> {
        // Read in the received and transferred bytes, store as u64
        let rx: u64 = std::fs::read_to_string(&self.if_rxbytes)?.trim().parse()?;
        let tx: u64 = std::fs::read_to_string(&self.if_txbytes)?.trim().parse()?;

        // And now write out values
        writeln!(handle, "{0}_tx.value {1}:{2}", self.interface, epoch, tx)?;
        writeln!(handle, "{0}_rx.value {1}:{2}", self.interface, epoch, rx)?;

        Ok(())
    }
}

fn main() -> Result<()> {
    SimpleLogger::new().init().unwrap();
    info!("if1sec started");

    // Set out config
    let mut config = Config::new(String::from("if1sec"));
    // Yes, we want to run as a daemon, gathering data once a second
    config.daemonize = true;
    // Fetchsize 64k is arbitary, but better than default 8k.
    config.fetchsize = 65535;

    let mut iface = InterfacePlugin {
        ..Default::default()
    };

    debug!("Interface: {:#?}", iface);
    // Get running
    iface.start(config)?;
    Ok(())
}
