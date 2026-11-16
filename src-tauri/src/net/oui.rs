//! The maker of a device, from its hardware (MAC) address.

/// Maker name for a hardware address in any common format
/// ("aa:bb:cc:dd:ee:ff", "AA-BB-..."). None when it is unknown, randomised
/// (phones and laptops often use a made-up address per network) or not a
/// real device address.
pub fn vendor(mac: &str) -> Option<String> {
    let _ = mac;
    todo!()
}

/// True for an address a device made up for itself rather than one its maker
/// assigned.
pub fn is_randomised(mac: &str) -> bool {
    let _ = mac;
    todo!()
}

/// True when the maker name is TP-Link in any of its spellings: the owner's
/// Deco mesh nodes are found this way.
pub fn looks_like_mesh(vendor: &str) -> bool {
    let _ = vendor;
    todo!()
}
