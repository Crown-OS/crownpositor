//! Building an image description, either from parameters or from a profile.
//!
//! Both creators are write-once: every setter may be sent at most once, and
//! `create` consumes the object. Validation that needs no renderer happens
//! here; anything that depends on what the compositor can actually draw is
//! deferred to [`ColorManagementHandler::accept_parametric`] and
//! [`accept_icc`](ColorManagementHandler::accept_icc).

use std::{os::fd::OwnedFd, sync::Mutex};

use wayland_protocols::wp::color_management::v1::server::{
    wp_image_description_creator_icc_v1, wp_image_description_creator_params_v1,
};

use super::{
    math::Primaries,
    record::{Luminances, NamedPrimaries, NamedTransferFunction, PrimariesSpec, TransferFunction},
};

/// The largest ICC profile accepted.
///
/// The protocol names 4 MiB as a lower bound a compositor must accept; 32 MiB
/// covers every real profile including LUT-heavy ones, and caps what a hostile
/// client can make the compositor allocate.
pub const MAX_ICC_SIZE: u32 = 32 * 1024 * 1024;

/// User data of a `wp_image_description_creator_params_v1`.
pub type ParamsData = Mutex<Params>;

#[derive(Debug, Default)]
pub struct Params {
    pub transfer: Option<TransferFunction>,
    pub primaries: Option<PrimariesSpec>,
    pub luminances: Option<Luminances>,
    pub mastering_primaries: Option<Primaries>,
    /// `(min_scaled, max)`.
    pub mastering_luminance: Option<(u32, u32)>,
    pub max_cll: Option<u32>,
    pub max_fall: Option<u32>,
    /// Set by `create`; the object is a destructor but the state outlives it
    /// briefly, so a second use is caught.
    pub used: bool,
}

impl Params {
    /// Whether enough has been set for a description to exist.
    ///
    /// The protocol requires both a transfer function and a set of primaries;
    /// everything else has a defined default.
    pub fn is_complete(&self) -> bool {
        self.transfer.is_some() && self.primaries.is_some()
    }

    /// The luminances to use, falling back to the curve's defaults.
    pub fn resolved_luminances(&self) -> Luminances {
        let default = match self.transfer {
            Some(TransferFunction::Named(named)) => named.default_luminances(),
            // A power curve is a relative encoding, so it takes the same
            // defaults the ordinary SDR curves do.
            _ => NamedTransferFunction::Gamma22.default_luminances(),
        };

        match self.luminances {
            Some(mut given) => {
                // The protocol is explicit: with PQ, `max_lum` is ignored and
                // forced to min + 10000.
                if self.transfer == Some(TransferFunction::Named(NamedTransferFunction::St2084Pq))
                {
                    given.max = (given.min().round() as u32).saturating_add(10_000);
                }
                given
            }
            None => default,
        }
    }
}

/// Rejections that map onto the creator's own error enum.
#[derive(Debug, PartialEq, Eq)]
pub enum ParamsError {
    Incomplete,
    AlreadySet,
    InvalidLuminance,
}

/// Validates everything the protocol layer can decide on its own.
pub fn validate(params: &Params) -> Result<(), ParamsError> {
    if !params.is_complete() {
        return Err(ParamsError::Incomplete);
    }

    let luminances = params.resolved_luminances();
    if !luminances.is_valid() {
        return Err(ParamsError::InvalidLuminance);
    }

    if let Some((min_scaled, max)) = params.mastering_luminance {
        let mastering = Luminances {
            min_scaled,
            max,
            // The mastering range has no reference of its own; borrowing the
            // description's keeps the ordering check meaningful.
            reference: luminances.reference.min(max).max(1),
        };
        if !mastering.is_valid() {
            return Err(ParamsError::InvalidLuminance);
        }
    }

    Ok(())
}

/// The primaries a finished description ends up with.
pub fn resolved_primaries(params: &Params) -> Option<PrimariesSpec> {
    params.primaries
}

/// The target volume, defaulting to the description's own primaries.
pub fn resolved_target_primaries(params: &Params) -> Option<Primaries> {
    params
        .mastering_primaries
        .or_else(|| params.primaries.map(PrimariesSpec::resolve))
}

/// The target luminance range, defaulting to the description's own.
pub fn resolved_target_luminance(params: &Params) -> (u32, u32) {
    let luminances = params.resolved_luminances();
    params
        .mastering_luminance
        .unwrap_or((luminances.min_scaled, luminances.max))
}

/// User data of a `wp_image_description_creator_icc_v1`.
pub type IccCreatorData = Mutex<IccCreator>;

#[derive(Debug, Default)]
pub struct IccCreator {
    pub file: Option<IccFile>,
    pub used: bool,
}

#[derive(Debug)]
pub struct IccFile {
    pub fd: OwnedFd,
    pub offset: u32,
    pub length: u32,
}

/// Rejections that map onto the ICC creator's error enum.
#[derive(Debug, PartialEq, Eq)]
pub enum IccError {
    Incomplete,
    AlreadySet,
    BadFd,
    BadSize,
    OutOfFile,
}

/// Reads the profile out of the client's file.
///
/// Read rather than mapped, and through a duplicated descriptor: the client
/// can `ftruncate` the file after handing it over, which would turn a mapping
/// into a `SIGBUS` inside the compositor, and reading through its own
/// descriptor would move an offset another process owns.
pub fn read_icc(file: &IccFile) -> Result<Vec<u8>, IccError> {
    use std::os::unix::fs::FileExt;

    if file.length == 0 || file.length > MAX_ICC_SIZE {
        return Err(IccError::BadSize);
    }

    let handle = std::fs::File::from(file.fd.try_clone().map_err(|_| IccError::BadFd)?);
    let metadata = handle.metadata().map_err(|_| IccError::BadFd)?;
    if !metadata.is_file() {
        return Err(IccError::BadFd);
    }

    let end = u64::from(file.offset) + u64::from(file.length);
    if end > metadata.len() {
        return Err(IccError::OutOfFile);
    }

    let mut bytes = vec![0u8; file.length as usize];
    handle
        .read_exact_at(&mut bytes, u64::from(file.offset))
        .map_err(|_| IccError::BadFd)?;
    Ok(bytes)
}

/// The header checks the protocol mandates before a profile is accepted.
///
/// Deliberately shallow: this is the "is this even an ICC profile" gate, and
/// the compositor's own parser decides whether it can be rendered.
pub fn validate_icc_header(bytes: &[u8]) -> Result<(), &'static str> {
    // 128-byte header plus the tag count.
    if bytes.len() < 132 {
        return Err("the profile is too short to contain a header");
    }
    if &bytes[36..40] != b"acsp" {
        return Err("the profile has no 'acsp' signature");
    }

    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if size > bytes.len() {
        return Err("the profile's declared size runs past the data given");
    }

    match bytes[8] {
        2 | 4 => {}
        version => {
            tracing::debug!(version, "rejecting an ICC profile that is neither v2 nor v4");
            return Err("only ICC v2 and v4 profiles are supported");
        }
    }

    if &bytes[16..20] != b"RGB " {
        return Err("only RGB profiles are supported");
    }

    match &bytes[12..16] {
        b"mntr" | b"scnr" | b"spac" => Ok(()),
        _ => Err("only display, input and colour-space profiles are supported"),
    }
}

/// Maps a params rejection onto the wire error.
pub fn params_error(error: ParamsError) -> wp_image_description_creator_params_v1::Error {
    use wp_image_description_creator_params_v1::Error;
    match error {
        ParamsError::Incomplete => Error::IncompleteSet,
        ParamsError::AlreadySet => Error::AlreadySet,
        ParamsError::InvalidLuminance => Error::InvalidLuminance,
    }
}

/// Maps an ICC rejection onto the wire error.
pub fn icc_error(error: IccError) -> wp_image_description_creator_icc_v1::Error {
    use wp_image_description_creator_icc_v1::Error;
    match error {
        IccError::Incomplete => Error::IncompleteSet,
        IccError::AlreadySet => Error::AlreadySet,
        IccError::BadFd => Error::BadFd,
        IccError::BadSize => Error::BadSize,
        IccError::OutOfFile => Error::OutOfFile,
    }
}

/// The protocol's named primaries, from the wire enum.
pub fn named_primaries(value: u32) -> Option<NamedPrimaries> {
    Some(match value {
        1 => NamedPrimaries::Srgb,
        2 => NamedPrimaries::PalM,
        3 => NamedPrimaries::Pal,
        4 => NamedPrimaries::Ntsc,
        5 => NamedPrimaries::GenericFilm,
        6 => NamedPrimaries::Bt2020,
        7 => NamedPrimaries::Cie1931Xyz,
        8 => NamedPrimaries::DciP3,
        9 => NamedPrimaries::DisplayP3,
        10 => NamedPrimaries::AdobeRgb,
        _ => return None,
    })
}

/// The protocol's named transfer functions, from the wire enum.
pub fn named_transfer(value: u32) -> Option<NamedTransferFunction> {
    Some(match value {
        1 => NamedTransferFunction::Bt1886,
        2 => NamedTransferFunction::Gamma22,
        3 => NamedTransferFunction::Gamma28,
        4 => NamedTransferFunction::St240,
        5 => NamedTransferFunction::ExtLinear,
        8 => NamedTransferFunction::Xvycc,
        9 => NamedTransferFunction::Srgb,
        10 => NamedTransferFunction::ExtSrgb,
        11 => NamedTransferFunction::St2084Pq,
        12 => NamedTransferFunction::St428,
        13 => NamedTransferFunction::Hlg,
        14 => NamedTransferFunction::CompoundPower2_4,
        // 6 and 7 are the logarithmic curves, deliberately not implemented.
        _ => return None,
    })
}

/// The wire value for a named transfer function.
pub fn transfer_wire_value(transfer: NamedTransferFunction) -> u32 {
    match transfer {
        NamedTransferFunction::Bt1886 => 1,
        NamedTransferFunction::Gamma22 => 2,
        NamedTransferFunction::Gamma28 => 3,
        NamedTransferFunction::St240 => 4,
        NamedTransferFunction::ExtLinear => 5,
        NamedTransferFunction::Xvycc => 8,
        NamedTransferFunction::Srgb => 9,
        NamedTransferFunction::ExtSrgb => 10,
        NamedTransferFunction::St2084Pq => 11,
        NamedTransferFunction::St428 => 12,
        NamedTransferFunction::Hlg => 13,
        NamedTransferFunction::CompoundPower2_4 => 14,
    }
}

/// The wire value for a named primary set.
pub fn primaries_wire_value(primaries: NamedPrimaries) -> u32 {
    match primaries {
        NamedPrimaries::Srgb => 1,
        NamedPrimaries::PalM => 2,
        NamedPrimaries::Pal => 3,
        NamedPrimaries::Ntsc => 4,
        NamedPrimaries::GenericFilm => 5,
        NamedPrimaries::Bt2020 => 6,
        NamedPrimaries::Cie1931Xyz => 7,
        NamedPrimaries::DciP3 => 8,
        NamedPrimaries::DisplayP3 => 9,
        NamedPrimaries::AdobeRgb => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete() -> Params {
        Params {
            transfer: Some(TransferFunction::Named(NamedTransferFunction::Gamma22)),
            primaries: Some(PrimariesSpec::Named(NamedPrimaries::Srgb)),
            ..Params::default()
        }
    }

    #[test]
    fn a_description_needs_both_a_curve_and_primaries() {
        assert_eq!(validate(&Params::default()), Err(ParamsError::Incomplete));

        let mut only_curve = Params::default();
        only_curve.transfer = Some(TransferFunction::Named(NamedTransferFunction::Gamma22));
        assert_eq!(validate(&only_curve), Err(ParamsError::Incomplete));

        assert_eq!(validate(&complete()), Ok(()));
    }

    #[test]
    fn each_curve_brings_its_own_luminance_defaults() {
        let mut params = complete();
        params.transfer = Some(TransferFunction::Named(NamedTransferFunction::St2084Pq));
        let luminances = params.resolved_luminances();
        assert_eq!((luminances.max, luminances.reference), (10_000, 203));
    }

    #[test]
    fn pq_ignores_the_maximum_the_client_asked_for() {
        // The protocol says so: with PQ, max is forced to min + 10000.
        let mut params = complete();
        params.transfer = Some(TransferFunction::Named(NamedTransferFunction::St2084Pq));
        params.luminances = Some(Luminances {
            min_scaled: 0,
            max: 600,
            reference: 203,
        });
        assert_eq!(params.resolved_luminances().max, 10_000);
    }

    #[test]
    fn a_luminance_range_the_wrong_way_round_is_refused() {
        let mut params = complete();
        params.luminances = Some(Luminances {
            min_scaled: 1_000_000,
            max: 80,
            reference: 80,
        });
        assert_eq!(validate(&params), Err(ParamsError::InvalidLuminance));
    }

    #[test]
    fn the_target_volume_defaults_to_the_descriptions_own() {
        let params = complete();
        assert_eq!(
            resolved_target_primaries(&params),
            Some(super::super::math::named::SRGB)
        );

        let luminances = params.resolved_luminances();
        assert_eq!(
            resolved_target_luminance(&params),
            (luminances.min_scaled, luminances.max)
        );
    }

    #[test]
    fn a_profile_that_is_not_icc_is_refused() {
        assert!(validate_icc_header(&[0u8; 200]).is_err());
        assert!(validate_icc_header(&[]).is_err());
    }

    #[test]
    fn a_minimal_valid_header_is_accepted() {
        let mut bytes = vec![0u8; 200];
        bytes[0..4].copy_from_slice(&180u32.to_be_bytes());
        bytes[8] = 4;
        bytes[12..16].copy_from_slice(b"mntr");
        bytes[16..20].copy_from_slice(b"RGB ");
        bytes[36..40].copy_from_slice(b"acsp");
        assert_eq!(validate_icc_header(&bytes), Ok(()));
    }

    #[test]
    fn a_cmyk_or_v5_profile_is_refused() {
        let mut bytes = vec![0u8; 200];
        bytes[0..4].copy_from_slice(&180u32.to_be_bytes());
        bytes[8] = 4;
        bytes[12..16].copy_from_slice(b"mntr");
        bytes[36..40].copy_from_slice(b"acsp");

        bytes[16..20].copy_from_slice(b"CMYK");
        assert!(validate_icc_header(&bytes).is_err());

        bytes[16..20].copy_from_slice(b"RGB ");
        bytes[8] = 5;
        assert!(validate_icc_header(&bytes).is_err());
    }

    #[test]
    fn a_profile_claiming_to_be_bigger_than_its_data_is_refused() {
        let mut bytes = vec![0u8; 200];
        bytes[0..4].copy_from_slice(&100_000u32.to_be_bytes());
        bytes[8] = 4;
        bytes[12..16].copy_from_slice(b"mntr");
        bytes[16..20].copy_from_slice(b"RGB ");
        bytes[36..40].copy_from_slice(b"acsp");
        assert!(validate_icc_header(&bytes).is_err());
    }

    #[test]
    fn every_named_value_round_trips_through_the_wire() {
        for named in NamedPrimaries::ALL {
            assert_eq!(named_primaries(primaries_wire_value(*named)), Some(*named));
        }
        for transfer in [
            NamedTransferFunction::Bt1886,
            NamedTransferFunction::Gamma22,
            NamedTransferFunction::St2084Pq,
            NamedTransferFunction::Hlg,
            NamedTransferFunction::CompoundPower2_4,
        ] {
            assert_eq!(named_transfer(transfer_wire_value(transfer)), Some(transfer));
        }
    }

    #[test]
    fn the_logarithmic_curves_are_not_accepted() {
        assert_eq!(named_transfer(6), None);
        assert_eq!(named_transfer(7), None);
        assert_eq!(named_transfer(0), None);
        assert_eq!(named_primaries(0), None);
        assert_eq!(named_primaries(99), None);
    }
}
