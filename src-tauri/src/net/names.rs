//! Names of devices on the network, from reverse DNS, mDNS and NetBIOS.

use std::collections::HashMap;
use std::net::Ipv4Addr;

/// The best name found for each address, already tidied for showing
/// ("deco-main.lan" becomes "deco-main"). Addresses with no name are left
/// out. Takes at most about 2.5 seconds. Must run inside the tokio runtime.
pub async fn discover(ips: &[Ipv4Addr]) -> HashMap<Ipv4Addr, String> {
    let _ = ips;
    todo!()
}
