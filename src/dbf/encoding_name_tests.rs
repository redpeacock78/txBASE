use super::*;

#[test]
fn reports_names_for_supported_language_drivers() {
    for (drivers, name) in [
        (&[0x01][..], "CP437"),
        (&[0x02][..], "CP850"),
        (&[0x1f, 0x22, 0x23, 0x40, 0x64, 0x87][..], "CP852"),
        (&[0x26, 0x65][..], "CP866"),
        (&[0x03, 0x57][..], "Windows-1252"),
        (&[0xc8][..], "Windows-1250"),
        (&[0xc9][..], "Windows-1251"),
        (&[0xca][..], "Windows-1254"),
        (&[0xcb][..], "Windows-1253"),
        (&[0x7d][..], "Windows-1255"),
        (&[0x7e][..], "Windows-1256"),
        (&[0x78][..], "Big5/CP950"),
        (&[0x79][..], "EUC-KR/CP949"),
        (&[0x4d, 0x7a][..], "GBK/CP936"),
        (&[0x7b][..], "Windows-31J/CP932"),
    ] {
        for driver in drivers {
            assert_eq!(encoding_name(*driver), Some(name));
        }
    }
    assert_eq!(encoding_name(0), None);
}
