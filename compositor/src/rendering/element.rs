use smithay::backend::renderer::{
    ImportAll, ImportMem,
    element::{
        Wrap, memory::MemoryRenderBufferRenderElement, solid::SolidColorRenderElement,
        surface::WaylandSurfaceRenderElement, utils::RescaleRenderElement,
    },
};

use crate::rendering::decorate::Cropped;

smithay::backend::renderer::element::render_elements! {
    /// Everything that can appear on an output.
    ///
    /// Generic over both the renderer and the decorated tile type, so a backend
    /// added later brings its own of each and this needs no changes.
    ///
    /// The tile variant is wrapped: the macro derives a `From` per variant, and
    /// a bare `E` could unify with the surface variant, which the compiler
    /// rejects as overlapping.
    pub CrownElement<R, E> where R: ImportAll + ImportMem;
    /// Layer-shell surfaces and popups, drawn whole — and a client's own cursor
    /// surface, which is a surface tree like any other.
    Surface = WaylandSurfaceRenderElement<R>,
    /// Anything the backend's decorator produced: a toplevel it drew, or a
    /// rectangle of blurred glass to go behind one.
    Tile = Wrap<E>,
    /// Anything the compositor rasterised on the CPU rather than a client
    /// giving it: a themed cursor, a window's title. `ImportMem` above is what
    /// this variant costs.
    Memory = MemoryRenderBufferRenderElement<R>,
    /// A surface drawn at a size other than its own, with nothing added: the
    /// wallpaper as the overview zooms it.
    Scaled = Cropped<R>,
    /// A flat fill: what a locked output shows behind, or instead of, its
    /// lock surface, and the overview's '+'.
    Solid = SolidColorRenderElement,
    /// A surface tree drawn whole with its `crownos_surface_animation_v1`
    /// transforms applied: an undecorated panel or a fullscreen window.
    Transformed = RescaleRenderElement<WaylandSurfaceRenderElement<R>>,
}
