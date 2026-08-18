// use smithay::wayland::background_effect::{
//     BackgroundEffectState, Capability, ExtBackgroundEffectHandler,
// };
// use smithay::wayland::compositor;
// use wayland_server::protocol::wl_surface::WlSurface;
//
// impl ExtBackgroundEffectHandler for State {
//     fn capabilities(&self) -> Capability {
//         Capability::Blur
//     }
//
//     fn set_blur_region(&mut self, wl_surface: WlSurface, region: compositor::RegionAttributes) {
//         // Called when blur becomes pending, and awaits surface commit.
//         // Blur region is stored in wl_surface [BackgroundEffectSurfaceCachedState]
//     }
// }
//
// smithay::delegate_dispatch2!(State);
