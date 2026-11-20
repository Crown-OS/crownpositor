use std::{fs, os::unix::fs::symlink};

use tempfile::TempDir;

use super::*;

const CARD: CardNode = CardNode {
    major: 226,
    minor: 1,
};

struct FakeSysfs {
    root: TempDir,
}

impl FakeSysfs {
    /// One card, `card1`, reached through its node as on the real tree.
    fn new() -> std::io::Result<Self> {
        let fake = Self {
            root: TempDir::new()?,
        };
        fake.directory("dev/char/226:1/device/drm/card1")?;
        fake.directory("dev/char/226:1/device/drm/renderD128")?;
        Ok(fake)
    }

    fn path(&self, relative: &str) -> std::path::PathBuf {
        self.root.path().join(relative)
    }

    fn directory(&self, relative: &str) -> std::io::Result<()> {
        fs::create_dir_all(self.path(relative))
    }

    fn file(&self, relative: &str, contents: &str) -> std::io::Result<()> {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }

    fn backlight(&self, name: &str, kind: &str, value: u32, max: u32) -> std::io::Result<()> {
        let base = format!("class/backlight/{name}");
        self.file(&format!("{base}/type"), &format!("{kind}\n"))?;
        self.file(&format!("{base}/brightness"), &format!("{value}\n"))?;
        self.file(&format!("{base}/max_brightness"), &format!("{max}\n"))
    }

    fn sysfs(&self) -> Sysfs {
        Sysfs::at(self.root.path())
    }
}

fn target(connector: &str, has_gamma: bool) -> ProbeTarget {
    ProbeTarget {
        output: OutputId::from_raw(1),
        card: CARD,
        connector: connector.to_owned(),
        has_gamma,
    }
}

fn no_ddc(_: u32) -> Option<(u32, u32)> {
    None
}

#[test]
fn the_card_is_found_through_its_node() -> std::io::Result<()> {
    let fake = FakeSysfs::new()?;
    assert_eq!(fake.sysfs().card_name(CARD).as_deref(), Some("card1"));
    let other = CardNode {
        major: 226,
        minor: 7,
    };
    assert_eq!(fake.sysfs().card_name(other), None);
    Ok(())
}

#[test]
fn a_panel_uses_the_backlight_under_its_connector() -> std::io::Result<()> {
    let fake = FakeSysfs::new()?;
    fake.directory("class/drm/card1-eDP-1/amdgpu")?;
    fake.file("class/drm/card1-eDP-1/amdgpu_bl1/max_brightness", "255")?;
    fake.backlight("amdgpu_bl1", "raw", 128, 255)?;
    fake.backlight("acpi_video0", "firmware", 5, 10)?;

    let found = fake.sysfs().probe(&target("eDP-1", true), no_ddc);
    assert_eq!(
        found,
        Found {
            device: Device::Backlight {
                name: "amdgpu_bl1".to_owned(),
                max: 255
            },
            raw: Some(128),
        }
    );
    Ok(())
}

#[test]
fn a_panel_without_its_own_backlight_takes_firmware_before_platform_before_raw()
-> std::io::Result<()> {
    let fake = FakeSysfs::new()?;
    fake.directory("class/drm/card1-eDP-1/amdgpu")?;
    fake.backlight("a_raw", "raw", 1, 10)?;
    fake.backlight("b_platform", "platform", 2, 10)?;
    fake.backlight("nvidia_wmi_ec_backlight", "firmware", 3, 10)?;

    let found = fake.sysfs().probe(&target("eDP-1", true), no_ddc);
    assert_eq!(
        found.device,
        Device::Backlight {
            name: "nvidia_wmi_ec_backlight".to_owned(),
            max: 10
        }
    );
    Ok(())
}

#[test]
fn a_monitor_uses_its_ddc_link_before_its_aux_bus() -> std::io::Result<()> {
    let fake = FakeSysfs::new()?;
    let connector = "class/drm/card1-DP-1";
    fake.directory(&format!("{connector}/i2c-12"))?;
    fake.directory("devices/i2c-3")?;
    symlink(
        fake.path("devices/i2c-3"),
        fake.path(&format!("{connector}/ddc")),
    )?;

    let mut asked = Vec::new();
    let found = fake.sysfs().probe(&target("DP-1", true), |bus| {
        asked.push(bus);
        (bus == 12).then_some((40, 100))
    });
    assert_eq!(asked, [3, 12]);
    assert_eq!(
        found,
        Found {
            device: Device::Ddc { bus: 12, max: 100 },
            raw: Some(40),
        }
    );
    Ok(())
}

#[test]
fn a_monitor_that_does_not_answer_is_dimmed_in_software() -> std::io::Result<()> {
    let fake = FakeSysfs::new()?;
    fake.directory("class/drm/card1-HDMI-A-1/i2c-5")?;
    let found = fake.sysfs().probe(&target("HDMI-A-1", true), no_ddc);
    assert_eq!(found.device, Device::Software);
    assert_eq!(found.raw, None);
    Ok(())
}

#[test]
fn an_output_without_gamma_or_hardware_has_no_control() -> std::io::Result<()> {
    let fake = FakeSysfs::new()?;
    let found = fake.sysfs().probe(&target("DP-2", false), no_ddc);
    assert_eq!(found.device, Device::None);
    Ok(())
}

#[test]
fn a_panel_on_an_unknown_card_never_reaches_the_machine_backlight() -> std::io::Result<()> {
    let fake = FakeSysfs::new()?;
    fake.backlight("acpi_video0", "firmware", 5, 10)?;
    let elsewhere = ProbeTarget {
        card: CardNode {
            major: 226,
            minor: 9,
        },
        ..target("eDP-1", true)
    };
    assert_eq!(
        fake.sysfs().probe(&elsewhere, no_ddc).device,
        Device::Software
    );
    Ok(())
}

#[test]
fn only_card_entries_name_a_card() {
    assert!(is_card("card0"));
    assert!(is_card("card12"));
    assert!(!is_card("card1-eDP-1"));
    assert!(!is_card("card"));
    assert!(!is_card("renderD128"));
    assert_eq!(bus_number("i2c-14"), Some(14));
    assert_eq!(bus_number("drm_dp_aux1"), None);
}
