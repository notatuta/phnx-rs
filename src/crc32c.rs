#[allow(dead_code)]
const CRC32C_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut j = i;
        let mut k = 0;
        while k < 8 {
            if j & 1 != 0 {
                j = (j >> 1) ^ 0x82f63b78;
            } else {
                j >>= 1;
            }
            k += 1;
        }
        table[i as usize] = j;
        i += 1;
    }
    table
};

pub struct Crc32c {
    value: u32,
}

impl Crc32c {
    pub fn new() -> Self {
        Crc32c { value: !0u32 }
    }

    #[inline]
    pub fn update(&mut self, byte: u8) {
        #[cfg(target_feature = "sse4.2")]
        {
            #[cfg(target_arch = "x86_64")]
            use std::arch::x86_64::_mm_crc32_u8;
            #[cfg(target_arch = "x86")]
            use std::arch::x86::_mm_crc32_u8;
            self.value = unsafe { _mm_crc32_u8(self.value, byte) };
            return;
        }

        #[cfg(not(target_feature = "sse4.2"))]
        {
            self.value = CRC32C_TABLE[((self.value ^ byte as u32) & 0xff) as usize]
                ^ (self.value >> 8);
        }
    }

    pub fn update_slice(&mut self, data: &[u8]) {
        for &b in data {
            self.update(b);
        }
    }

    pub fn finalize(&self) -> u32 {
        !self.value
    }
}
