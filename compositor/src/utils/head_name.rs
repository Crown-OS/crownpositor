//! The names heads go by. A head on the primary GPU is `IFACE-N`, as the
//! kernel names its connector. A head on another GPU whose connector name
//! another GPU also has is `IFACE-N@ADDRESS`, where the address is the GPU's
//! udev `ID_PATH` (its PCI address for a PCI device), which stays the same
//! from boot to boot.

const ADDRESS_SEPARATOR: char = '@';

/// The name of connector `interface`-`index`, qualified by `address` when
/// one is given.
pub fn qualified(interface: &str, index: u32, address: Option<&str>) -> String {
    match address {
        Some(address) => format!("{interface}-{index}{ADDRESS_SEPARATOR}{address}"),
        None => format!("{interface}-{index}"),
    }
}

/// The connector name the kernel gives the head, as sysfs and older config
/// entries know it.
pub fn unqualified(name: &str) -> &str {
    name.split_once(ADDRESS_SEPARATOR)
        .map_or(name, |(connector, _)| connector)
}

/// The GPU address of a qualified name.
pub fn pci_suffix(name: &str) -> Option<&str> {
    name.split_once(ADDRESS_SEPARATOR)
        .map(|(_, address)| address)
}

/// Whether `head` went by `given` before it gained a GPU address, as a
/// name written by hand before then still says.
pub fn was_named(head: &str, given: &str) -> bool {
    pci_suffix(head).is_some() && unqualified(head) == given
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: &str = "0000:01:00.0";

    #[test]
    fn a_head_without_an_address_keeps_the_kernel_name() {
        assert_eq!(qualified("HDMI-A", 1, None), "HDMI-A-1");
        assert_eq!(unqualified("HDMI-A-1"), "HDMI-A-1");
        assert_eq!(pci_suffix("HDMI-A-1"), None);
    }

    #[test]
    fn a_qualified_name_splits_back_into_connector_and_address() {
        let name = qualified("HDMI-A", 1, Some(ADDRESS));
        assert_eq!(name, "HDMI-A-1@0000:01:00.0");
        assert_eq!(unqualified(&name), "HDMI-A-1");
        assert_eq!(pci_suffix(&name), Some(ADDRESS));
    }

    #[test]
    fn only_a_qualified_head_answers_to_its_old_name() {
        assert!(was_named("HDMI-A-1@0000:01:00.0", "HDMI-A-1"));
        assert!(!was_named("HDMI-A-1", "HDMI-A-1"));
        assert!(!was_named("HDMI-A-1@0000:01:00.0", "HDMI-A-2"));
    }

    #[test]
    fn an_address_that_is_not_pci_is_kept_whole() {
        let name = qualified("DP", 2, Some("platform-fd5a0000.gpu"));
        assert_eq!(unqualified(&name), "DP-2");
        assert_eq!(pci_suffix(&name), Some("platform-fd5a0000.gpu"));
    }
}
