//! Launcher icons a liquid morph carries, copied off the launcher's surface
//! the first frame they are drawn, before the launcher hides its own.

use std::{collections::HashMap, mem};

use protocols::crownos_app_icons::{IconRef, PlacedIcon};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Offscreen,
            damage::OutputDamageTracker,
            element::{
                Kind,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::{GlesRenderer, GlesTexture},
        },
    },
    utils::{Buffer as BufferCoords, Physical, Point, Scale, Size, Transform},
};

const TRANSPARENT: [f32; 4] = [0.0; 4];

/// One output's carried icons, kept only while a morph draws them.
#[derive(Debug, Default)]
pub struct CarriedIcons(HashMap<IconRef, IconCopy>);

#[derive(Debug)]
struct IconCopy {
    texture: GlesTexture,
    used: bool,
}

impl CarriedIcons {
    /// Lets go of the icons no morph drew last frame.
    pub fn begin_frame(&mut self) {
        self.0.retain(|_, copy| mem::take(&mut copy.used));
    }

    /// The copy of `icon`, taken now if this is the first frame it is drawn.
    /// `None` when the launcher's surface showed nothing there.
    pub fn copy_of(
        &mut self,
        gles: &mut GlesRenderer,
        icon: &PlacedIcon,
        scale: Scale<f64>,
    ) -> Option<GlesTexture> {
        if !self.0.contains_key(&icon.icon) {
            let texture = capture(gles, icon, scale)?;
            self.0.insert(
                icon.icon.clone(),
                IconCopy {
                    texture,
                    used: false,
                },
            );
        }
        let copy = self.0.get_mut(&icon.icon)?;
        copy.used = true;
        Some(copy.texture.clone())
    }
}

/// Renders the part of the launcher's surface the icon covers.
fn capture(gles: &mut GlesRenderer, icon: &PlacedIcon, scale: Scale<f64>) -> Option<GlesTexture> {
    let rect = icon.placement.rect;
    let size: Size<i32, Physical> = rect.size.to_physical_precise_round(scale);
    if size.w <= 0 || size.h <= 0 {
        return None;
    }
    let origin = Point::default() - rect.loc.to_physical_precise_round(scale);
    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(
            gles,
            &icon.surface,
            origin,
            scale,
            1.0,
            Kind::Unspecified,
        );
    if elements.is_empty() {
        return None;
    }
    let mut texture: GlesTexture = gles
        .create_buffer(
            Fourcc::Abgr8888,
            Size::<i32, BufferCoords>::from((size.w, size.h)),
        )
        .ok()?;
    let mut target = gles.bind(&mut texture).ok()?;
    OutputDamageTracker::new(size, scale, Transform::Normal)
        .render_output(gles, &mut target, 0, &elements, TRANSPARENT)
        .ok()?;
    drop(target);
    Some(texture)
}
