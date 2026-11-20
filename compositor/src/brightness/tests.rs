use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::*;

const PANEL: OutputId = OutputId::from_raw(1);
const MONITOR: OutputId = OutputId::from_raw(2);
const TABLET: OutputId = OutputId::from_raw(3);

fn brightness() -> (Brightness, UnboundedReceiver<Job>) {
    let (jobs, requests) = mpsc::unbounded_channel();
    (Brightness::with_jobs(jobs, Device::None), requests)
}

fn backlight() -> Device {
    Device::Backlight {
        name: "amdgpu_bl1".to_owned(),
        max: 255,
    }
}

fn written(requests: &mut UnboundedReceiver<Job>) -> Vec<u32> {
    std::iter::from_fn(|| requests.try_recv().ok())
        .filter_map(|job| match job {
            Job::Write { raw, .. } => Some(raw),
            _ => None,
        })
        .collect()
}

#[test]
fn a_set_is_raised_to_the_floor_of_the_output_control() {
    let (mut brightness, _requests) = brightness();
    brightness.adopt(PANEL, backlight(), FULL);
    brightness.adopt(MONITOR, Device::Software, FULL);
    brightness.adopt(TABLET, Device::None, FULL);

    assert_eq!(
        brightness.set(PANEL, 0).map(|reading| reading.level),
        Some(100)
    );
    assert_eq!(
        brightness.set(MONITOR, 500).map(|reading| reading.level),
        Some(1500)
    );
    assert_eq!(brightness.set(TABLET, 500), None);
    assert_eq!(brightness.reading(TABLET), Reading::UNCONTROLLED);
}

#[test]
fn only_a_software_level_dims_the_gamma_ramp() {
    let (mut brightness, _requests) = brightness();
    brightness.adopt(PANEL, backlight(), 3000);
    brightness.adopt(MONITOR, Device::Software, 6000);
    brightness.adopt(TABLET, Device::None, FULL);

    assert_eq!(brightness.software_dim(PANEL), 1.0);
    assert_eq!(brightness.software_dim(MONITOR), 0.6);
    assert_eq!(brightness.software_dim(TABLET), 1.0);
    assert_eq!(brightness.software_dim(OutputId::from_raw(99)), 1.0);
}

#[test]
fn a_hardware_set_becomes_a_write_and_a_software_set_a_ramp() {
    let (mut brightness, mut requests) = brightness();
    brightness.adopt(PANEL, backlight(), FULL);
    brightness.adopt(MONITOR, Device::Software, FULL);
    assert!(brightness.take_software_changed());

    brightness.set(PANEL, FULL);
    assert_eq!(written(&mut requests), [255]);
    assert!(!brightness.take_software_changed());

    brightness.set(MONITOR, 4000);
    assert!(written(&mut requests).is_empty());
    assert!(brightness.take_software_changed());
}

#[test]
fn a_read_from_before_the_newest_set_is_dropped() {
    let (mut brightness, _requests) = brightness();
    brightness.adopt(PANEL, backlight(), 5000);
    brightness.refresh(PANEL);
    brightness.set(PANEL, 6000);

    assert_eq!(brightness.read_back(PANEL, 0, 10), None);
    assert_eq!(brightness.reading(PANEL).level, 6000);
}

#[test]
fn a_read_that_matches_the_level_keeps_it_and_one_that_differs_replaces_it() {
    let (mut brightness, _requests) = brightness();
    brightness.adopt(PANEL, backlight(), 100);
    let raw_of_level = curve::Curve::Perceptual.raw(100, 255);

    assert_eq!(brightness.read_back(PANEL, 0, raw_of_level), None);
    assert_eq!(brightness.reading(PANEL).level, 100);

    let changed = brightness.read_back(PANEL, 0, 255);
    assert_eq!(changed.map(|reading| reading.level), Some(FULL));
}

#[test]
fn a_probe_reports_none_until_it_answers() {
    let (mut brightness, _requests) = brightness();
    let target = ProbeTarget {
        output: MONITOR,
        card: CardNode {
            major: 226,
            minor: 0,
        },
        connector: "DP-1".to_owned(),
        has_gamma: true,
    };
    brightness.probe(target);
    assert_eq!(brightness.reading(MONITOR), Reading::UNCONTROLLED);

    let found = Found {
        device: Device::Ddc { bus: 3, max: 100 },
        raw: Some(40),
    };
    assert_eq!(
        brightness.probed(MONITOR, found, 1500),
        Some(Reading {
            control: Control::Ddc,
            level: 4000
        })
    );
}

#[test]
fn a_probe_that_finds_no_hardware_starts_at_the_saved_software_level() {
    let (mut brightness, _requests) = brightness();
    brightness.adopt(MONITOR, Device::None, FULL);
    let found = Found {
        device: Device::Software,
        raw: None,
    };
    let reading = brightness.probed(MONITOR, found, 4200);
    assert_eq!(reading.map(|reading| reading.level), Some(4200));
    assert_eq!(brightness.software_dim(MONITOR), 0.42);
}

#[test]
fn an_answer_for_a_forgotten_output_is_dropped() {
    let (mut brightness, _requests) = brightness();
    brightness.adopt(MONITOR, Device::None, FULL);
    brightness.forget(MONITOR);
    let found = Found {
        device: Device::Software,
        raw: None,
    };
    assert_eq!(brightness.probed(MONITOR, found, FULL), None);
}

#[test]
fn a_step_moves_from_the_current_level() {
    let (mut brightness, _requests) = brightness();
    brightness.adopt(MONITOR, Device::Software, 5000);
    let stepped = brightness.step(MONITOR, Step(-5));
    assert_eq!(stepped.map(|reading| reading.level), Some(4500));
    let floored = brightness.step(MONITOR, Step(-100));
    assert_eq!(floored.map(|reading| reading.level), Some(1500));
}
