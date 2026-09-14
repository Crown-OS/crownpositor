//! PnP manufacturer IDs, for turning `"DEL"` into `"Dell Inc."`.
//!
//! The full UEFI registry is ~2000 entries and almost all of them are chipsets
//! that will never appear on a `connector`. This is the subset that actually
//! ships in monitors and laptop panels; anything else falls back to the raw
//! three-letter code, which is still a usable identity.

/// Sorted by code so the lookup can binary-search.
const VENDORS: &[(&[u8; 3], &str)] = &[
    (b"ACI", "Asus"),
    (b"ACR", "Acer"),
    (b"ADI", "ADI"),
    (b"AGO", "Agora"),
    (b"AOC", "AOC"),
    (b"APP", "Apple"),
    (b"AUO", "AU Optronics"),
    (b"AUS", "Asus"),
    (b"BBK", "BBK"),
    (b"BNQ", "BenQ"),
    (b"BOE", "BOE"),
    (b"CMN", "Chimei Innolux"),
    (b"CMO", "Chi Mei Optoelectronics"),
    (b"CPT", "Chunghwa Picture Tubes"),
    (b"CRO", "Crown"),
    (b"DEL", "Dell Inc."),
    (b"DON", "Denon"),
    (b"EIZ", "Eizo"),
    (b"ELE", "Element"),
    (b"ENC", "Eizo"),
    (b"EPI", "Envision"),
    (b"FUS", "Fujitsu Siemens"),
    (b"GBT", "Gigabyte"),
    (b"GSM", "LG Electronics"),
    (b"GWD", "Greenwood"),
    (b"HEC", "Hisense"),
    (b"HPN", "HP"),
    (b"HSD", "HannStar"),
    (b"HSP", "HannStar"),
    (b"HTC", "Hitachi"),
    (b"HWP", "HP"),
    (b"IFS", "InFocus"),
    (b"INL", "InnoLux"),
    (b"IVM", "Iiyama"),
    (b"KTC", "KTC"),
    (b"LEN", "Lenovo"),
    (b"LGD", "LG Display"),
    (b"LPL", "LG Philips"),
    (b"MED", "Medion"),
    (b"MEI", "Panasonic"),
    (b"MEL", "Mitsubishi"),
    (b"MSI", "MSI"),
    (b"MST", "MStar"),
    (b"NEC", "NEC"),
    (b"NVD", "Nvidia"),
    (b"ONK", "Onkyo"),
    (b"PHL", "Philips"),
    (b"PIO", "Pioneer"),
    (b"PNR", "Planar"),
    (b"QDS", "Quanta Display"),
    (b"RAT", "Rateo"),
    (b"SAM", "Samsung"),
    (b"SDC", "Samsung Display"),
    (b"SEC", "Seiko Epson"),
    (b"SHP", "Sharp"),
    (b"SNY", "Sony"),
    (b"STN", "Samtron"),
    (b"SYN", "Synaptics"),
    (b"TCL", "TCL"),
    (b"TSB", "Toshiba"),
    (b"VES", "Vestel"),
    (b"VIZ", "Vizio"),
    (b"VSC", "ViewSonic"),
    (b"YMH", "Yamaha"),
];

/// The registered name for a PnP code, if it is one worth naming.
pub fn name_for(code: &[u8; 3]) -> Option<&'static str> {
    VENDORS
        .binary_search_by(|(known, _)| known.as_slice().cmp(code.as_slice()))
        .ok()
        .map(|index| VENDORS[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_so_the_search_is_valid() {
        assert!(
            VENDORS.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "VENDORS must stay sorted by code"
        );
    }

    #[test]
    fn known_codes_resolve_and_unknown_ones_do_not() {
        assert_eq!(name_for(b"DEL"), Some("Dell Inc."));
        assert_eq!(name_for(b"GSM"), Some("LG Electronics"));
        assert_eq!(name_for(b"ZZZ"), None);
    }
}
