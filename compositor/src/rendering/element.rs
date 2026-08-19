use smithay::backend::renderer::{
    ImportAll, ImportMem,
    element::{memory::MemoryRenderBufferRenderElement, surface::WaylandSurfaceRenderElement, Wrap},
};

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
    /// A toplevel, however the backend's decorator rendered it.
    Tile = Wrap<E>,
    /// A themed cursor, rasterised into main memory rather than owned by a
    /// client. `ImportMem` above is what this variant costs.
    Cursor = MemoryRenderBufferRenderElement<R>,
}
