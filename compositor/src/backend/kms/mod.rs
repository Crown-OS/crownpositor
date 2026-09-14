//! The DRM/KMS backend: real GPUs, real outputs, libinput input.
//!
//! Ownership layout mirrors the hardware. [`KmsState`] owns the seat session
//! and one [`Device`] per GPU; each device will own one rendering surface per
//! connected monitor (Step 3). GLES contexts live inside the [`GpuManager`],
//! which hands out a [`MultiRenderer`] that can composite a client buffer
//! living on any GPU onto any output — single-GPU machines simply never hit
//! the copy path.
//!
//! The graphics-API switch ([`GraphicsApi`]) decides who *allocates* the
//! scanout buffers (GBM vs Vulkan); see [`crate::backend::render`] for the
//! shape of that seam.

pub mod crtc_pool;
pub mod device;
pub mod head;
pub mod props;
pub mod reconfigure;
pub mod surface;
pub mod vulkan;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::Context as _;
use smithay::{
    wayland::drm_lease::DrmLeaseState,
    backend::{
        allocator::gbm::GbmDevice,
        drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType},
        egl::context::ContextPriority,
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{ImportDma, multigpu::gbm::GbmGlesBackend},
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{self, UdevBackend, UdevEvent},
    },
    output::Output,
    reexports::{input::Libinput, rustix::fs::OFlags},
    utils::DeviceFd,
    wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal},
};

pub use crate::backend::kms::surface::redraw_queued_outputs;
use smithay::reexports::drm::control::{connector, crtc};
pub use crate::backend::kms::{device::Device, vulkan::VulkanContext};
use crate::{
    backend::render::{CrownRenderer, GraphicsApi, KmsGpuManager, KmsRenderer},
    state::{BackendState, State},
};

/// KMS-specific failures, typed so callers can tell "this GPU is unusable"
/// (skip it) from "the seat is unusable" (abort the backend).
#[derive(Debug, thiserror::Error)]
pub enum KmsError {
    #[error("failed to create the libseat session: {0}")]
    Session(#[from] smithay::backend::session::libseat::Error),
    #[error("libinput could not take seat {seat}")]
    SeatAssignment { seat: String },
    #[error("failed to enumerate GPUs: {0}")]
    Udev(#[from] std::io::Error),
    #[error("seat {seat} has no GPU")]
    NoGpu { seat: String },
    #[error("failed to open DRM device {path}: {source}")]
    OpenDevice {
        path: String,
        source: smithay::backend::session::libseat::Error,
    },
    #[error("failed to initialize DRM on {path}: {source}")]
    Drm {
        path: String,
        source: smithay::backend::drm::DrmError,
    },
    #[error("failed to initialize GBM on {path}: {source}")]
    Gbm {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to add GPU {node} to the renderer: {0}", node = .1)]
    AddNode(smithay::backend::egl::Error, DrmNode),
    #[error("the GPU manager failed: {0}")]
    GpuManager(String),
}

pub struct KmsState {
    pub session: LibSeatSession,
    pub libinput: Libinput,
    pub gpu_manager: KmsGpuManager,
    /// Which API allocates scanout buffers. Fixed for the session.
    pub api: GraphicsApi,
    /// Present only when `api` is Vulkan *and* the instance came up.
    pub vulkan: Option<VulkanContext>,
    /// The boot GPU: its render node is where client buffers should live and
    /// where compositing for every output happens.
    pub primary_node: DrmNode,
    pub primary_render_node: DrmNode,
    pub devices: HashMap<DrmNode, Device>,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub dmabuf_feedback: Option<DmabufFeedback>,
}

impl KmsState {
    /// A renderer targeting the primary GPU, which is where all compositing
    /// happens (buffers on other GPUs get copied across by the multi-renderer).
    pub fn primary_renderer(&mut self) -> Result<KmsRenderer<'_>, KmsError> {
        self.gpu_manager
            .single_renderer(&self.primary_render_node)
            .map_err(|err| KmsError::GpuManager(err.to_string()))
    }

    pub fn can_import_dmabuf(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> bool {
        self.primary_renderer()
            .map(|mut renderer| renderer.import_dmabuf(dmabuf, None).is_ok())
            .unwrap_or(false)
    }

    /// Schedules a frame: flips each matching surface's state machine to
    /// "queued". The actual render runs after the current dispatch cycle
    /// drains, in [`surface::redraw_queued_outputs`] — so a burst of client
    /// commits coalesces into one frame.
    pub fn queue_redraw(&mut self, output: Option<&Output>) {
        for device in self.devices.values_mut() {
            for surface in device.surfaces.values_mut() {
                if output.is_none_or(|output| *output == surface.output) {
                    surface.redraw_state = std::mem::take(&mut surface.redraw_state).queue();
                }
            }
        }
    }
}

pub fn init(state: &mut State) -> anyhow::Result<()> {
    let (session, session_notifier) = LibSeatSession::new().map_err(KmsError::Session)?;
    let seat_name = session.seat();
    tracing::info!(seat = seat_name, "libseat session created");

    // Input: libinput drives every seat device, suspended/resumed with the VT.
    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| KmsError::SeatAssignment {
            seat: seat_name.clone(),
        })?;

    let udev_backend = UdevBackend::new(&seat_name).map_err(KmsError::Udev)?;

    // High-priority EGL contexts so a heavyweight client cannot starve the
    // compositor's own GPU work — the difference between a stutter in a game
    // and a stutter in the whole desktop.
    let gpu_manager =
        KmsGpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High))
            .map_err(|err| KmsError::GpuManager(err.to_string()))?;

    let api = GraphicsApi::detect();
    let vulkan = match api {
        GraphicsApi::Vulkan => match VulkanContext::try_new() {
            Ok(context) => Some(context),
            Err(err) => {
                tracing::warn!(%err, "vulkan requested but unavailable, allocations fall back to GBM");
                None
            }
        },
        GraphicsApi::EglGles3 => None,
    };

    // The boot GPU. `primary_gpu` prefers the boot_vga device; a headless
    // seat with zero GPUs is not something this backend can drive.
    let primary_path = udev::primary_gpu(&seat_name)
        .map_err(KmsError::Udev)?
        .or_else(|| {
            udev::all_gpus(&seat_name)
                .ok()
                .and_then(|mut gpus| (!gpus.is_empty()).then(|| gpus.remove(0)))
        })
        .ok_or_else(|| KmsError::NoGpu {
            seat: seat_name.clone(),
        })?;
    let primary_node = DrmNode::from_path(&primary_path)
        .with_context(|| format!("not a DRM node: {}", primary_path.display()))?;
    let primary_render_node = render_node_for(primary_node);
    tracing::info!(node = %primary_node, render = %primary_render_node, ?api, "primary GPU selected");

    state.backend = BackendState::Kms(Box::new(KmsState {
        session,
        libinput: libinput.clone(),
        gpu_manager,
        api,
        vulkan,
        primary_node,
        primary_render_node,
        devices: HashMap::new(),
        dmabuf_global: None,
        dmabuf_feedback: None,
    }));

    // Cold-plug: the GPUs that were present before we started listening.
    for (device_id, path) in udev_backend.device_list() {
        match DrmNode::from_dev_id(device_id) {
            Ok(node) => {
                if let Err(err) = device_added(state, node, path) {
                    tracing::error!(%err, %node, "skipping GPU");
                }
            }
            Err(err) => tracing::warn!(%err, device_id, "udev listed a non-DRM device"),
        }
    }
    init_dmabuf(state)?;

    let handle = state.common.event_loop_handle.clone();
    handle
        .insert_source(
            LibinputInputBackend::new(libinput),
            |mut event, _, state| {
                // Device defaults are applied here rather than inside
                // `process_input_event`: that is generic over the backend, and only
                // libinput has a device with settings on it to configure.
                if let InputEvent::DeviceAdded { device } = &mut event {
                    crate::input::libinput::apply_defaults(device);
                }
                state.process_input_event(event);
            },
        )
        .map_err(|err| anyhow::anyhow!("failed to insert the libinput source: {err}"))?;

    handle
        .insert_source(session_notifier, |event, _, state| {
            let Some(kms) = state.backend.kms() else {
                return;
            };
            match event {
                SessionEvent::PauseSession => {
                    tracing::info!("session paused (VT switch away)");
                    kms.libinput.suspend();
                    for device in kms.devices.values_mut() {
                        device.drm.pause();
                        // A leased client must not keep driving hardware that
                        // now belongs to another VT.
                        if let Some(lease_state) = device.lease_state.as_mut() {
                            lease_state.suspend();
                        }
                    }
                }
                SessionEvent::ActivateSession => {
                    tracing::info!("session activated (VT switch back)");
                    if let Err(err) = kms.libinput.resume() {
                        tracing::error!(?err, "failed to resume libinput");
                    }
                    for device in kms.devices.values_mut() {
                        if let Err(err) = device.drm.activate(false) {
                            tracing::error!(%err, node = %device.node, "failed to reactivate DRM");
                        }
                        for surface in device.surfaces.values_mut() {
                            // The other VT scribbled over the planes; buffer
                            // ages are meaningless now, so force full redraws.
                            surface.compositor.reset_buffers();
                            // A panel blanked before the switch stays blanked,
                            // but its CRTC was reset, so the state has to be
                            // re-asserted rather than assumed.
                            surface.powered = true;
                        }
                        if let Some(lease_state) = device.lease_state.as_mut() {
                            lease_state.resume::<State>();
                        }
                    }
                    // Everything on screen is stale after a VT switch.
                    state.backend.queue_redraw(None);
                }
            }
        })
        .map_err(|err| anyhow::anyhow!("failed to insert the session source: {err}"))?;

    handle
        .insert_source(udev_backend, |event, _, state| match event {
            UdevEvent::Added { device_id, path } => match DrmNode::from_dev_id(device_id) {
                Ok(node) => {
                    if let Err(err) = device_added(state, node, &path) {
                        tracing::error!(%err, %node, "failed to bring up hotplugged GPU");
                    }
                }
                Err(err) => tracing::warn!(%err, device_id, "hotplugged a non-DRM device"),
            },
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    device_changed(state, node);
                }
            }
            UdevEvent::Removed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    device_removed(state, node);
                }
            }
        })
        .map_err(|err| anyhow::anyhow!("failed to insert the udev source: {err}"))?;

    Ok(())
}

/// The render node for a card node, falling back to the card node itself
/// (some SoCs render and scan out through the same node).
fn render_node_for(node: DrmNode) -> DrmNode {
    node.node_with_type(NodeType::Render)
        .and_then(Result::ok)
        .unwrap_or(node)
}

fn device_added(state: &mut State, node: DrmNode, path: &Path) -> Result<(), KmsError> {
    let handle = state.common.event_loop_handle.clone();
    let display_handle = state.common.display_handle.clone();
    let Some(kms) = state.backend.kms() else {
        return Ok(());
    };
    if kms.devices.contains_key(&node) {
        return Ok(());
    }

    let fd = kms
        .session
        .open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|source| KmsError::OpenDevice {
            path: path.display().to_string(),
            source,
        })?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    // `disable_connectors: true`: we own the full output configuration and
    // want a clean slate rather than whatever the boot splash left mapped.
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true).map_err(|source| KmsError::Drm {
        path: path.display().to_string(),
        source,
    })?;
    let gbm = GbmDevice::new(fd).map_err(|source| KmsError::Gbm {
        path: path.display().to_string(),
        source,
    })?;

    let render_node = render_node_for(node);
    kms.gpu_manager
        .as_mut()
        .add_node(render_node, gbm.clone())
        .map_err(|err| KmsError::AddNode(err, render_node))?;

    // Effects compile once per GPU; failure degrades (square corners), never
    // blocks the GPU from lighting up.
    match kms.gpu_manager.single_renderer(&render_node) {
        Ok(mut renderer) => {
            if let Err(err) = renderer.compile_shaders() {
                tracing::warn!(%err, %render_node, "failed to compile effect shaders");
            }
        }
        Err(err) => {
            tracing::warn!(%err, %render_node, "no renderer to compile shaders with");
        }
    }

    let drm_token = handle
        .insert_source(drm_notifier, move |event, metadata, state| match event {
            DrmEvent::VBlank(crtc) => {
                surface::on_vblank(state, node, crtc, metadata.take());
            }
            DrmEvent::Error(err) => {
                tracing::error!(%err, "DRM device error");
            }
        })
        .map_err(|err| KmsError::GpuManager(format!("failed to insert the DRM source: {err}")))?;

    tracing::info!(%node, %render_node, "GPU added");
    kms.devices.insert(
        node,
        Device {
            node,
            render_node,
            drm,
            gbm,
            scanner: smithay_drm_extras::drm_scanner::DrmScanner::new(),
            heads: HashMap::new(),
            surfaces: HashMap::new(),
            drm_token,
            // A GPU with no lease global is still perfectly usable; only VR
            // headsets and the like go without.
            lease_state: match DrmLeaseState::new::<State>(&display_handle, &node) {
                Ok(state) => Some(state),
                Err(err) => {
                    tracing::debug!(%err, %node, "no DRM lease global for this GPU");
                    None
                }
            },
            active_leases: HashMap::new(),
            leased_crtcs: HashSet::new(),
        },
    );

    // Pick up whatever monitors are already connected.
    device_changed(state, node);
    Ok(())
}

/// Reconciles the device's head registry with what is physically plugged in,
/// then lights up whatever the config says should be on.
///
/// The scanner is still used to *detect* plug and unplug, but its CRTC
/// assignment is deliberately ignored: it reserves one for every connected
/// connector, including ones the user has switched off, so it cannot answer
/// "is there a CRTC free for this monitor?" — which is the question `test`
/// exists to answer. `crtc::assign` does that instead.
fn device_changed(state: &mut State, node: DrmNode) {
    use smithay_drm_extras::drm_scanner::DrmScanEvent;

    // Scan first, act second: the `head_*` functions need the whole `State`,
    // so the device borrow must not outlive the scan.
    let events: Vec<DrmScanEvent> = {
        let Some(device) = state
            .backend
            .kms()
            .and_then(|kms| kms.devices.get_mut(&node))
        else {
            return;
        };
        match device.scanner.scan_connectors(&device.drm) {
            Ok(scan) => scan.into_iter().collect(),
            Err(err) => {
                tracing::error!(%err, %node, "failed to scan connectors");
                return;
            }
        }
    };

    let mut appeared = Vec::new();
    for event in events {
        match event {
            DrmScanEvent::Connected { connector, .. } => {
                let handle = connector.handle();
                let probed = state
                    .backend
                    .kms()
                    .and_then(|kms| kms.devices.get(&node))
                    .map(|device| &device.drm)
                    .and_then(|drm| head::Head::probe(drm, handle));

                let Some(probed) = probed else {
                    tracing::warn!(
                        connector = connector.interface_id(),
                        "connector has no usable modes, ignoring"
                    );
                    continue;
                };

                // A VR headset and friends: the kernel says this is not part
                // of the desktop, so it must never be lit as an output. It is
                // offered for leasing instead, which is how its runtime gets
                // to drive it directly.
                let non_desktop = state
                    .backend
                    .kms()
                    .and_then(|kms| kms.devices.get(&node))
                    .is_some_and(|device| props::non_desktop(&device.drm, handle));

                if non_desktop {
                    offer_for_lease(state, node, handle, &probed);
                    continue;
                }

                if let Some(device) = state
                    .backend
                    .kms()
                    .and_then(|kms| kms.devices.get_mut(&node))
                {
                    device.heads.insert(handle, probed);
                }
                appeared.push(handle);
            }
            DrmScanEvent::Disconnected { connector, .. } => {
                let handle = connector.handle();
                if let Some(device) = state
                    .backend
                    .kms()
                    .and_then(|kms| kms.devices.get_mut(&node))
                    && let Some(lease_state) = device.lease_state.as_mut()
                {
                    lease_state.withdraw_connector(handle);
                }
                surface::connector_disconnected(state, node, handle);
            }
        }
    }

    for connector in appeared {
        bring_up_head(state, node, connector);
    }
}

/// Hands a non-desktop connector to `wp_drm_lease_v1` rather than lighting it.
fn offer_for_lease(
    state: &mut State,
    node: DrmNode,
    connector: connector::Handle,
    head: &head::Head,
) {
    let Some(device) = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get_mut(&node))
    else {
        return;
    };
    let Some(lease_state) = device.lease_state.as_mut() else {
        return;
    };

    tracing::info!(output = head.name, "connector is non-desktop, offering it for lease");
    lease_state.add_connector::<State>(
        connector,
        head.name.clone(),
        head.identity.as_str().to_owned(),
    );
}

/// Lights a head that is currently off, using the config's mode.
///
/// The public door for "turn this monitor on"; hotplug reaches the same code
/// through [`bring_up_head`], so a monitor appearing and a monitor being
/// switched on take the same path and cannot drift apart.
pub fn enable_configured_head(
    state: &mut State,
    node: DrmNode,
    connector: connector::Handle,
) -> anyhow::Result<()> {
    bring_up_head(state, node, connector);

    let enabled = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get(&node))
        .and_then(|device| device.heads.get(&connector))
        .is_some_and(|head| head.state.is_enabled());

    anyhow::ensure!(enabled, "the monitor could not be brought up");
    Ok(())
}

/// Lights a newly appeared head up, unless the config says it is off.
fn bring_up_head(state: &mut State, node: DrmNode, connector: connector::Handle) {
    // Cloned out so the device borrow ends before `assign_crtc` needs its own.
    let head = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get(&node))
        .and_then(|device| device.heads.get(&connector))
        .cloned();
    let Some(head) = head else {
        return;
    };

    let setting = state
        .config
        .current
        .output_setting(&head.name, head.identity.as_str())
        .cloned();

    if setting.as_ref().and_then(|setting| setting.enabled) == Some(false) {
        tracing::info!(output = head.name, "monitor stays off; the config disables it");
        state.refresh_output_heads();
        return;
    }

    if let Some(bpc) = setting.as_ref().and_then(|setting| setting.max_bpc)
        && !head.accepts_max_bpc(bpc)
    {
        tracing::warn!(
            output = head.name,
            bpc,
            supported = ?head.max_bpc,
            "the configured colour depth is outside what this connector accepts"
        );
    }

    let Some(drm_mode) = head.mode_for(setting.as_ref().and_then(|s| s.mode.as_deref())) else {
        return;
    };

    // Ask for a CRTC against the heads that are meant to be on, rather than
    // taking whatever the scanner reserved.
    let Some(crtc) = assign_crtc(state, node, connector) else {
        tracing::warn!(
            output = head.name,
            "no CRTC is free for this monitor; it stays dark"
        );
        state.refresh_output_heads();
        return;
    };

    if let Err(err) = surface::enable_head(state, node, connector, crtc, drm_mode) {
        tracing::error!(%err, "failed to bring up connector");
    }
}

/// A CRTC for `connector`, keeping every already-lit head on the one it has.
fn assign_crtc(
    state: &mut State,
    node: DrmNode,
    connector: connector::Handle,
) -> Option<crtc::Handle> {
    let device = state.backend.kms()?.devices.get(&node)?;

    let mut wanted: Vec<connector::Handle> = device
        .heads
        .values()
        .filter(|head| head.state.is_enabled())
        .map(|head| head.connector)
        .collect();
    let pinned: crtc_pool::Assignment = device
        .heads
        .values()
        .filter_map(|head| head.state.crtc().map(|crtc| (head.connector, crtc)))
        .collect();
    wanted.push(connector);

    match crtc_pool::assign(&device.drm, &wanted, &pinned, &device.leased_crtcs) {
        Ok(assignment) => assignment.get(&connector).copied(),
        Err(err) => {
            tracing::debug!(%err, "CRTC assignment failed");
            None
        }
    }
}

fn device_removed(state: &mut State, node: DrmNode) {
    let handle = state.common.event_loop_handle.clone();

    // Tear down every monitor on this GPU first — that path also detaches
    // the outputs from the shell.
    let connectors: Vec<connector::Handle> = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get(&node))
        .map(|device| device.heads.keys().copied().collect())
        .unwrap_or_default();
    for connector in connectors {
        surface::connector_disconnected(state, node, connector);
    }

    let Some(kms) = state.backend.kms() else {
        return;
    };
    let Some(device) = kms.devices.remove(&node) else {
        return;
    };

    kms.gpu_manager.as_mut().remove_node(&device.render_node);
    handle.remove(device.drm_token);
    tracing::info!(%node, "GPU removed");
}

/// Advertises the primary GPU's formats to clients, preferring v4 feedback.
///
/// Same fallback ladder as the winit backend, but the target device comes
/// from the primary render node rather than an EGL query — on KMS we already
/// know exactly which GPU compositing happens on.
fn init_dmabuf(state: &mut State) -> anyhow::Result<()> {
    let display = state.common.display_handle.clone();
    let State {
        backend, wayland, ..
    } = state;
    let Some(kms) = backend.kms() else {
        return Ok(());
    };

    let formats = kms
        .primary_renderer()
        .map_err(|err| anyhow::anyhow!("no renderer for the primary GPU: {err}"))?
        .dmabuf_formats();

    let feedback = DmabufFeedbackBuilder::new(kms.primary_render_node.dev_id(), formats.clone())
        .build()
        .ok();

    match feedback {
        Some(feedback) => {
            let global = wayland
                .dmabuf_state
                .create_global_with_default_feedback::<State>(&display, &feedback);
            kms.dmabuf_global = Some(global);
            kms.dmabuf_feedback = Some(feedback);
        }
        None => {
            tracing::warn!("failed to build dmabuf feedback, falling back to dmabuf v3");
            let global = wayland
                .dmabuf_state
                .create_global::<State>(&display, formats);
            kms.dmabuf_global = Some(global);
        }
    }
    Ok(())
}
