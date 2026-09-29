//! Delegation glue for `wp-tearing-control-v1`; the protocol lives in
//! [`protocols::tearing_control`].

use smithay::reexports::wayland_protocols::wp::tearing_control::v1::server::{
    wp_tearing_control_manager_v1::WpTearingControlManagerV1,
    wp_tearing_control_v1::WpTearingControlV1,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use protocols::tearing_control::{TearingControlData, TearingControlState};

use crate::state::State;

delegate_global_dispatch!(State: [WpTearingControlManagerV1: ()] => TearingControlState);
delegate_dispatch!(State: [WpTearingControlManagerV1: ()] => TearingControlState);
delegate_dispatch!(State: [WpTearingControlV1: TearingControlData] => TearingControlState);
