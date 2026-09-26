//! What a capture session advertises: the renderer's own render formats,
//! narrowed to what an encoder takes, plus NV12 when the GPU can convert.

use smithay::{
    backend::allocator::{Fourcc, Modifier, format::FormatSet},
    utils::{Buffer as BufferCoords, Size},
};

use protocols::crownos_screencast::{BufferConstraints, FormatModifiers};

/// 8-bit RGB, opaque formats first: an encoder ignores alpha, and a driver
/// can skip writing it.
const RGB_FORMATS: [Fourcc; 4] = [
    Fourcc::Xrgb8888,
    Fourcc::Xbgr8888,
    Fourcc::Argb8888,
    Fourcc::Abgr8888,
];

/// The two single-plane formats an NV12 buffer is rendered through: its luma
/// plane as R8 and its interleaved chroma plane as GR88.
pub const NV12_LUMA_PLANE: Fourcc = Fourcc::R8;
pub const NV12_CHROMA_PLANE: Fourcc = Fourcc::Gr88;

/// NV12 is only offered linear. A tiled NV12 modifier describes both planes
/// of one image; nothing guarantees the same modifier means the same layout
/// for an R8 and a GR88 image on their own, and linear NV12 is what every
/// VA-API and V4L2 encoder imports.
const NV12_MODIFIER: Modifier = Modifier::Linear;

pub fn buffer_constraints(
    render_formats: &FormatSet,
    size: Size<i32, BufferCoords>,
    gpu_conversion: bool,
) -> BufferConstraints {
    let renders = |code: Fourcc, modifier: Modifier| {
        render_formats
            .iter()
            .any(|format| format.code == code && format.modifier == modifier)
    };

    let nv12 = (gpu_conversion
        && renders(NV12_LUMA_PLANE, NV12_MODIFIER)
        && renders(NV12_CHROMA_PLANE, NV12_MODIFIER))
    .then(|| FormatModifiers {
        format: Fourcc::Nv12,
        modifiers: vec![NV12_MODIFIER],
    });

    let rgb = RGB_FORMATS.iter().filter_map(|code| {
        let modifiers: Vec<Modifier> = render_formats
            .iter()
            .filter(|format| format.code == *code)
            .map(|format| format.modifier)
            .collect();
        (!modifiers.is_empty()).then_some(FormatModifiers {
            format: *code,
            modifiers,
        })
    });

    BufferConstraints {
        size,
        formats: nv12.into_iter().chain(rgb).collect(),
    }
}

#[cfg(test)]
mod tests {
    use smithay::backend::allocator::Format;

    use super::*;

    fn formats(list: &[(Fourcc, Modifier)]) -> FormatSet {
        list.iter()
            .map(|(code, modifier)| Format {
                code: *code,
                modifier: *modifier,
            })
            .collect()
    }

    const TILED: Modifier = Modifier::I915_x_tiled;

    #[test]
    fn nv12_comes_first_when_both_planes_are_renderable() {
        let set = formats(&[
            (Fourcc::R8, Modifier::Linear),
            (Fourcc::Gr88, Modifier::Linear),
            (Fourcc::Xrgb8888, Modifier::Linear),
            (Fourcc::Xrgb8888, TILED),
        ]);
        let constraints = buffer_constraints(&set, (1920, 1080).into(), true);

        assert_eq!(constraints.size, Size::from((1920, 1080)));
        assert_eq!(constraints.formats[0].format, Fourcc::Nv12);
        assert_eq!(constraints.formats[0].modifiers, vec![Modifier::Linear]);
        assert_eq!(constraints.formats[1].format, Fourcc::Xrgb8888);
        assert_eq!(constraints.formats[1].modifiers.len(), 2);
    }

    #[test]
    fn nv12_needs_the_shader_and_both_planes() {
        let both = formats(&[
            (Fourcc::R8, Modifier::Linear),
            (Fourcc::Gr88, Modifier::Linear),
        ]);
        assert!(
            buffer_constraints(&both, (8, 8).into(), false)
                .formats
                .is_empty()
        );

        let luma_only = formats(&[(Fourcc::R8, Modifier::Linear)]);
        assert!(
            buffer_constraints(&luma_only, (8, 8).into(), true)
                .formats
                .is_empty()
        );

        let tiled_planes = formats(&[(Fourcc::R8, TILED), (Fourcc::Gr88, TILED)]);
        assert!(
            buffer_constraints(&tiled_planes, (8, 8).into(), true)
                .formats
                .is_empty()
        );
    }

    #[test]
    fn formats_an_encoder_cannot_take_are_left_out() {
        let set = formats(&[
            (Fourcc::Argb2101010, Modifier::Linear),
            (Fourcc::Abgr8888, Modifier::Linear),
        ]);
        let constraints = buffer_constraints(&set, (8, 8).into(), false);
        let advertised: Vec<Fourcc> = constraints.formats.iter().map(|f| f.format).collect();
        assert_eq!(advertised, vec![Fourcc::Abgr8888]);
    }
}
