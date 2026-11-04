//! Rockchip's RC4, used to obfuscate the IDBlock before it is written to flash.
//!
//! `P_RC4` at `0x426ee0` is textbook RC4 (identity state, `K[i] = key[i % 16]`,
//! standard KSA swap and PRGA). The only Rockchip-specific part is the key.

/// The 16-byte key `P_RC4` loads with a single `movdqa` from `0x61d4c0`.
///
/// Extracted from the binary's bytes directly. The final byte is `0x11`; it is
/// frequently published as `0x4d`, which does not match this tool and would
/// corrupt every byte of keystream after the key schedule.
pub const ROCKCHIP_RC4_KEY: [u8; 16] = [
    0x7c, 0x4e, 0x03, 0x04, 0x55, 0x05, 0x09, 0x07, 0x2d, 0x2c, 0x7b, 0x38, 0x17, 0x0d, 0x17, 0x11,
];

/// An RC4 keystream generator.
pub struct Rc4 {
    s: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    /// Run the key schedule for `key`, which is repeated to fill 256 bytes.
    pub fn new(key: &[u8]) -> Self {
        assert!(!key.is_empty(), "RC4 key must not be empty");
        let mut s = [0u8; 256];
        for (i, slot) in s.iter_mut().enumerate() {
            *slot = i as u8;
        }
        let mut j = 0u8;
        for i in 0..256 {
            j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
            s.swap(i, j as usize);
        }
        Self { s, i: 0, j: 0 }
    }

    /// Key schedule using [`ROCKCHIP_RC4_KEY`].
    pub fn rockchip() -> Self {
        Self::new(&ROCKCHIP_RC4_KEY)
    }

    /// Next keystream byte.
    pub fn next_byte(&mut self) -> u8 {
        self.i = self.i.wrapping_add(1);
        self.j = self.j.wrapping_add(self.s[self.i as usize]);
        self.s.swap(self.i as usize, self.j as usize);
        let idx = self.s[self.i as usize].wrapping_add(self.s[self.j as usize]);
        self.s[idx as usize]
    }

    /// XOR the keystream over `data` in place.
    pub fn apply(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            *byte ^= self.next_byte();
        }
    }
}

/// Encrypt or decrypt `data` in place with Rockchip's key, restarting the key
/// schedule for this buffer. RC4 is symmetric, so one function serves both.
pub fn rockchip_rc4(data: &mut [u8]) {
    Rc4::rockchip().apply(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6229 test vector: key "Key", plaintext "Plaintext".
    #[test]
    fn matches_published_rc4_vector() {
        let mut data = *b"Plaintext";
        Rc4::new(b"Key").apply(&mut data);
        assert_eq!(data, [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]);
    }

    #[test]
    fn is_symmetric() {
        let original = b"rockchip idblock sector".to_vec();
        let mut buf = original.clone();
        rockchip_rc4(&mut buf);
        assert_ne!(buf, original, "ciphertext must differ from plaintext");
        rockchip_rc4(&mut buf);
        assert_eq!(buf, original);
    }

    /// Guards against the 0x4d/0x11 key confusion: the first keystream byte is
    /// a fingerprint of the whole key schedule.
    #[test]
    fn key_last_byte_affects_keystream() {
        let mut wrong = ROCKCHIP_RC4_KEY;
        wrong[15] = 0x4d;
        let ours = Rc4::rockchip().next_byte();
        let theirs = Rc4::new(&wrong).next_byte();
        assert_ne!(
            ours, theirs,
            "0x11 vs 0x4d must produce different keystreams"
        );
    }
}
