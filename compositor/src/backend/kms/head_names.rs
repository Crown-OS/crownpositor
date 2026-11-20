//! The name each head goes by (see [`crate::utils::head_name`]). A head is
//! renamed only while it is off, so a lit monitor never changes its name.

use smithay::{
    backend::drm::{DrmDevice, DrmNode},
    reexports::{
        drm::control::{Device as _, connector},
        udev,
    },
};

use crate::{backend::kms::KmsState, utils::head_name};

const UDEV_PATH_PROPERTY: &str = "ID_PATH";
const PCI_PREFIX: &str = "pci-";

/// What tells a GPU apart from boot to boot: its PCI address, else its whole
/// udev `ID_PATH`, else its device name.
pub fn device_address(node: DrmNode) -> String {
    let device = match udev::Device::from_devnum(udev::DeviceType::Character, node.dev_id()) {
        Ok(device) => device,
        Err(err) => {
            tracing::warn!(%err, %node, "no udev device for this GPU");
            return node.to_string();
        }
    };
    match device.property_value(UDEV_PATH_PROPERTY) {
        Some(path) => {
            let path = path.to_string_lossy();
            path.strip_prefix(PCI_PREFIX).unwrap_or(&path).to_owned()
        }
        None => device.sysname().to_string_lossy().into_owned(),
    }
}

/// The kernel names of every connector on a GPU, plugged in or not.
fn connector_names(drm: &DrmDevice) -> Vec<String> {
    let Ok(handles) = drm.resource_handles() else {
        return Vec::new();
    };
    handles
        .connectors()
        .iter()
        .filter_map(|handle| drm.get_connector(*handle, false).ok())
        .map(|info| head_name::qualified(info.interface().as_str(), info.interface_id(), None))
        .collect()
}

/// Whether a head carries its GPU's address: only off the primary GPU, and
/// only when another GPU has a connector with the same name.
pub fn needs_address(on_primary: bool, kernel_name: &str, elsewhere: &[String]) -> bool {
    !on_primary && elsewhere.iter().any(|other| other == kernel_name)
}

impl KmsState {
    /// The name connector `interface`-`index` on `node` goes by with the
    /// GPUs open now.
    pub fn head_name(&self, node: DrmNode, interface: connector::Interface, index: u32) -> String {
        let kernel_name = head_name::qualified(interface.as_str(), index, None);
        let elsewhere: Vec<String> = self
            .devices
            .iter()
            .filter(|(other, _)| **other != node)
            .flat_map(|(_, device)| connector_names(&device.drm))
            .collect();
        let address = self
            .devices
            .get(&node)
            .map(|device| device.address.as_str())
            .filter(|_| needs_address(node == self.primary_node, &kernel_name, &elsewhere));
        head_name::qualified(interface.as_str(), index, address)
    }

    /// Gives every head that is off the name it goes by now.
    pub fn rename_heads_that_are_off(&mut self) {
        let renamed: Vec<(DrmNode, connector::Handle, String)> = self
            .devices
            .iter()
            .flat_map(|(node, device)| {
                device
                    .heads
                    .values()
                    .filter(|head| !head.state.is_enabled())
                    .filter_map(|head| {
                        let name = self.head_name(*node, head.interface, head.interface_id);
                        (name != head.name).then_some((*node, head.connector, name))
                    })
            })
            .collect();
        for (node, connector, name) in renamed {
            if let Some(head) = self
                .devices
                .get_mut(&node)
                .and_then(|device| device.heads.get_mut(&connector))
            {
                tracing::info!(from = head.name, to = name, "head renamed");
                head.name = name;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_head_on_the_primary_gpu_keeps_the_kernel_name() {
        assert!(!needs_address(true, "HDMI-A-1", &names(&["HDMI-A-1"])));
    }

    #[test]
    fn a_head_elsewhere_carries_the_address_only_on_a_clash() {
        let primary = names(&["eDP-1", "DP-1", "HDMI-A-1"]);
        assert!(needs_address(false, "HDMI-A-1", &primary));
        assert!(!needs_address(false, "DP-2", &primary));
    }
}
