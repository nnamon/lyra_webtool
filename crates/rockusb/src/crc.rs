//! Checksums used by the rockusb protocol and its container formats.
//!
//! All three were recovered from `upgrade_tool` v2.17 rather than assumed:
//!
//! * [`crc16_ccitt`] is a decode of `CRC_CCITT` at `0x427180`.
//! * [`crc32_rockchip`] uses the polynomial implied by `gTable_Crc32`
//!   (`.data` at `0x69e180`); regenerating a 256-entry MSB-first table with
//!   that polynomial reproduces the binary's table byte for byte.
//! * [`crc32_ieee`] matches `crc32table_le` at `0x69e620`, which is the
//!   ordinary zlib/IEEE table.

/// Polynomial used by `CRC_CCITT`, seen as `xorw $0x1021`.
pub const CCITT_POLY: u16 = 0x1021;

/// Rockchip's CRC32 polynomial.
///
/// Note the third nibble: `0x04C10DB7`, not the standard `0x04C11DB7`. The
/// difference is real and is what makes Rockchip IDBlock checksums
/// incompatible with an ordinary CRC-32.
pub const ROCKCHIP_CRC32_POLY: u32 = 0x04C1_0DB7;

/// CRC-16/CCITT-FALSE: init `0xFFFF`, poly `0x1021`, MSB-first, no reflection
/// and no final XOR.
///
/// `CRC_CCITT` conditionally XORs the polynomial twice per bit -- once on the
/// previous CRC's bit 15 and once on the input bit. Two XORs of the same value
/// cancel, so the pair is equivalent to the usual `msb ^ input_bit` test.
///
/// The device expects this appended **big endian** after each 471/472 chunk:
/// `RKU_DeviceRequest` writes `%ah` then `%al`.
pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &byte in data {
        for bit in (0..8).rev() {
            let input = (byte >> bit) & 1 == 1;
            let msb = crc & 0x8000 != 0;
            crc <<= 1;
            if msb ^ input {
                crc ^= CCITT_POLY;
            }
        }
    }
    crc
}

/// Append a big-endian CRC-16/CCITT to `data`, as the MaskROM bootstrap requires.
pub fn append_crc16_ccitt(data: &mut Vec<u8>) {
    let crc = crc16_ccitt(data);
    data.extend_from_slice(&crc.to_be_bytes());
}

const fn build_msb_table(poly: u32) -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ poly
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

const fn build_lsb_table(poly: u32) -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ poly
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// Rockchip's MSB-first CRC32 table, equivalent to the binary's `gTable_Crc32`.
pub static ROCKCHIP_CRC32_TABLE: [u32; 256] = build_msb_table(ROCKCHIP_CRC32_POLY);

static IEEE_CRC32_TABLE: [u32; 256] = build_lsb_table(0xEDB8_8320);

/// Rockchip's CRC32 over `data`, starting from `init` (use 0 for a fresh run).
///
/// MSB-first with polynomial [`ROCKCHIP_CRC32_POLY`] and no final inversion.
/// Used for the IDBlock sector 3 checksum.
pub fn crc32_rockchip_with(init: u32, data: &[u8]) -> u32 {
    let mut crc = init;
    for &byte in data {
        let idx = (((crc >> 24) ^ byte as u32) & 0xff) as usize;
        crc = (crc << 8) ^ ROCKCHIP_CRC32_TABLE[idx];
    }
    crc
}

/// Rockchip's CRC32 over `data`, starting from zero.
pub fn crc32_rockchip(data: &[u8]) -> u32 {
    crc32_rockchip_with(0, data)
}

/// Standard IEEE/zlib CRC-32, as used for GPT headers and UBI/sparse images.
pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xff) as usize;
        crc = (crc >> 8) ^ IEEE_CRC32_TABLE[idx];
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical CRC-16/CCITT-FALSE check value for "123456789".
    #[test]
    fn ccitt_matches_published_check_value() {
        assert_eq!(crc16_ccitt(b"123456789"), 0x29B1);
    }

    /// `CRC_CCITT` returns its init value untouched for an empty buffer
    /// (`testl %esi,%esi; je` returns `0xffffffff`, low 16 bits `0xffff`).
    #[test]
    fn ccitt_empty_is_init() {
        assert_eq!(crc16_ccitt(&[]), 0xffff);
    }

    #[test]
    fn ccitt_appends_big_endian() {
        let mut buf = b"123456789".to_vec();
        append_crc16_ccitt(&mut buf);
        assert_eq!(&buf[9..], &[0x29, 0xB1]);
    }

    /// Table entry 1 must be the raw polynomial for an MSB-first table; this is
    /// the value read out of the binary's `.data` at `0x69e180 + 4`.
    #[test]
    fn rockchip_table_matches_binary() {
        assert_eq!(ROCKCHIP_CRC32_TABLE[0], 0x0000_0000);
        assert_eq!(ROCKCHIP_CRC32_TABLE[1], 0x04C1_0DB7);
        assert_eq!(ROCKCHIP_CRC32_TABLE[2], 0x0982_1B6E);
        assert_eq!(ROCKCHIP_CRC32_TABLE[3], 0x0D43_16D9);
    }

    /// Rockchip's polynomial must not be confused with the standard one.
    #[test]
    fn rockchip_crc32_differs_from_ieee() {
        assert_ne!(crc32_rockchip(b"123456789"), crc32_ieee(b"123456789"));
    }

    /// Canonical IEEE CRC-32 check value.
    #[test]
    fn ieee_matches_published_check_value() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn rockchip_crc32_is_resumable() {
        let data = b"the quick brown fox";
        let whole = crc32_rockchip(data);
        let split = crc32_rockchip_with(crc32_rockchip(&data[..7]), &data[7..]);
        assert_eq!(whole, split);
    }
}
