//! Reading an image description back.
//!
//! The protocol requires that every `wp_image_description_info_v1` created
//! from the same description "return the exact same data", so this is a pure
//! function of the record — nothing here may consult the compositor's current
//! state.
//!
//! The burst ends with `done`, which is a destructor: the object exists only
//! for the length of one description.

use std::os::fd::AsFd;

use wayland_protocols::wp::color_management::v1::server::wp_image_description_info_v1::WpImageDescriptionInfoV1;

use super::{
    creator::{primaries_wire_value, transfer_wire_value},
    record::{
        DescriptionKind, ImageDescription, ParametricDescription, PrimariesSpec, TransferFunction,
        chromaticity_to_wire,
    },
};

/// Sends everything known about `description`, then `done`.
pub fn send(object: &WpImageDescriptionInfoV1, description: &ImageDescription) {
    match description.kind() {
        DescriptionKind::Parametric(parametric) => send_parametric(object, parametric),
        DescriptionKind::Icc(icc) => send_icc(object, icc),
        // Neither is describable, and `dispatch` refuses `get_information` for
        // them before reaching here.
        DescriptionKind::WindowsScrgb | DescriptionKind::WindowsBt2100 => {}
    }
    object.done();
}

fn send_parametric(object: &WpImageDescriptionInfoV1, parametric: &ParametricDescription) {
    // The coordinates are always sent; the name only when the client's own
    // description was built from one, because that is the distinction the
    // protocol asks to be preserved.
    let primaries = parametric.primaries.resolve();
    let (red, green, blue, white) = (
        chromaticity_to_wire(primaries.red),
        chromaticity_to_wire(primaries.green),
        chromaticity_to_wire(primaries.blue),
        chromaticity_to_wire(primaries.white),
    );
    object.primaries(red.0, red.1, green.0, green.1, blue.0, blue.1, white.0, white.1);

    if let PrimariesSpec::Named(named) = parametric.primaries
        && let Ok(wire) = super::reexports::wp_color_manager_v1::Primaries::try_from(
            primaries_wire_value(named),
        )
    {
        object.primaries_named(wire);
    }

    match parametric.transfer {
        TransferFunction::Power(exponent) => object.tf_power(exponent),
        TransferFunction::Named(named) => {
            if let Ok(wire) = super::reexports::wp_color_manager_v1::TransferFunction::try_from(
                transfer_wire_value(named),
            ) {
                object.tf_named(wire);
            }
        }
    }

    let luminances = parametric.luminances;
    object.luminances(
        luminances.min_scaled,
        luminances.max,
        luminances.reference,
    );

    let target = parametric.target_primaries;
    let (red, green, blue, white) = (
        chromaticity_to_wire(target.red),
        chromaticity_to_wire(target.green),
        chromaticity_to_wire(target.blue),
        chromaticity_to_wire(target.white),
    );
    object.target_primaries(
        red.0, red.1, green.0, green.1, blue.0, blue.1, white.0, white.1,
    );
    object.target_luminance(parametric.target_luminance.0, parametric.target_luminance.1);

    if let Some(max_cll) = parametric.max_cll {
        object.target_max_cll(max_cll);
    }
    if let Some(max_fall) = parametric.max_fall {
        object.target_max_fall(max_fall);
    }
}

fn send_icc(object: &WpImageDescriptionInfoV1, icc: &super::record::IccData) {
    // A sealed, read-only memfd: the client must not be able to change the
    // profile out from under a compositor that is still reading it, and the
    // seal is what makes that guarantee rather than a convention.
    let Some(fd) = sealed_memfd(&icc.bytes) else {
        tracing::warn!("could not hand back an ICC profile; skipping the icc_file event");
        return;
    };
    // The event borrows the descriptor and the library dups it across the
    // wire, so ours is closed when `fd` drops at the end of this function.
    object.icc_file(fd.as_fd(), icc.bytes.len() as u32);
}

/// A read-only, size-sealed memfd holding `bytes`.
fn sealed_memfd(bytes: &[u8]) -> Option<std::os::fd::OwnedFd> {
    use smithay::reexports::rustix::{
        fs::{MemfdFlags, SealFlags, memfd_create, fcntl_add_seals},
        io::write,
    };

    let fd = memfd_create("crownpositor-icc", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)
        .ok()?;

    let mut written = 0;
    while written < bytes.len() {
        written += write(&fd, &bytes[written..]).ok()?;
    }

    fcntl_add_seals(
        &fd,
        SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
    )
    .ok()?;

    Some(fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_memfd_holds_exactly_the_bytes_given() {
        use std::os::unix::fs::FileExt;

        let bytes: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let fd = sealed_memfd(&bytes).expect("memfd_create is available");

        let file = std::fs::File::from(fd);
        assert_eq!(
            file.metadata().expect("stat").len(),
            bytes.len() as u64,
            "the profile must be exactly as long as the data"
        );

        let mut read_back = vec![0u8; bytes.len()];
        file.read_exact_at(&mut read_back, 0).expect("read");
        assert_eq!(read_back, bytes);
    }

    #[test]
    fn a_sealed_memfd_cannot_be_written_to() {
        use std::os::unix::fs::FileExt;

        let fd = sealed_memfd(&[1, 2, 3, 4]).expect("memfd_create is available");
        let file = std::fs::File::from(fd);
        assert!(
            file.write_at(&[9], 0).is_err(),
            "the seal must make the profile immutable"
        );
    }

    #[test]
    fn an_empty_profile_still_produces_a_descriptor() {
        assert!(sealed_memfd(&[]).is_some());
    }
}
