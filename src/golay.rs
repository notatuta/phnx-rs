const GOLAY_MATRIX: [u32; 12] = [
    0x9f1, 0x4fa, 0x27d, 0x93e, 0xc9d, 0xe4e,
    0xf25, 0xf92, 0x7c9, 0x3e6, 0x557, 0xaab,
];

pub struct GolayCode {
    pub processed_codewords: i32,
    pub corrected_codewords: i32,
    pub uncorrectable_codewords: i32,
}

impl GolayCode {
    pub fn new() -> Self {
        GolayCode {
            processed_codewords: 0,
            corrected_codewords: 0,
            uncorrectable_codewords: 0,
        }
    }

    #[inline]
    fn checksum_bits(x: u32) -> u32 {
        let mut y = 0u32;
        for i in 0..12 {
            y = (y << 1) | ((x & GOLAY_MATRIX[i]).count_ones() & 1);
        }
        y
    }

    /// Takes 12 bits of data, appends 12 checksum bits, returns a 24 bit codeword
    #[inline]
    pub fn encode(&self, x: u32) -> u32 {
        ((x & 0xfff) << 12) | Self::checksum_bits(x)
    }

    /// Takes a 24 bit codeword, returns decoded 12 bits.
    /// On unrecoverable error, returns -1.
    pub fn decode(&mut self, x: u32) -> i32 {
        self.processed_codewords += 1;

        let received_data = (x >> 12) & 0xfff;
        let received_checksum = x & 0xfff;
        let expected_checksum = Self::checksum_bits(received_data);

        let syndrome = expected_checksum ^ received_checksum;
        let weight = syndrome.count_ones() as i32;

        if weight <= 3 {
            if weight != 0 {
                self.corrected_codewords += 1;
            }
            return received_data as i32;
        }

        for i in 0..12 {
            let error_mask = 1u32 << (11 - i);
            let coding_error = GOLAY_MATRIX[i];
            if (syndrome ^ coding_error).count_ones() <= 2 {
                self.corrected_codewords += 1;
                return (received_data ^ error_mask) as i32;
            }
        }

        let inverted_syndrome = Self::checksum_bits(syndrome);
        let w = inverted_syndrome.count_ones();
        if w <= 3 {
            self.corrected_codewords += 1;
            return (received_data ^ inverted_syndrome) as i32;
        }

        for i in 0..12 {
            let coding_error = GOLAY_MATRIX[i];
            if (inverted_syndrome ^ coding_error).count_ones() <= 2 {
                self.corrected_codewords += 1;
                return (received_data ^ inverted_syndrome ^ coding_error) as i32;
            }
        }

        self.uncorrectable_codewords += 1;
        -1
    }
}

pub fn self_test() -> bool {
    let mut gc = GolayCode::new();
    let mut not_decoded_ct = [0u32; 11];
    let mut decoded_ok_ct = [0u32; 11];
    let mut decoded_wrong_ct = [0u32; 11];

    // Simple LCG for deterministic testing (matching C's rand())
    let mut rng_state: u32 = 12345;
    let mut next_rand = || -> u32 {
        // Use a simple LCG
        rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (rng_state >> 16) & 0x7fff
    };

    for _i in 0..10000 {
        for j in 0..11usize {
            let x = next_rand() & 0xfff;
            let y = gc.encode(x);
            let mut errors = 0u32;
            let mut k = 0;
            while k < j {
                let bit = 1u32 << (next_rand() % 24);
                if (errors & bit) == 0 {
                    errors |= bit;
                    k += 1;
                }
            }
            let z = gc.decode(y ^ errors);
            if z < 0 {
                not_decoded_ct[j] += 1;
            } else if z as u32 == x {
                decoded_ok_ct[j] += 1;
            } else {
                decoded_wrong_ct[j] += 1;
            }
            if z as u32 != x && j < 4 {
                eprintln!("GolayCode self-test failed");
                eprintln!(
                    "Original:    0x{:03x}\nTransmitted: 0x{:06x}\nError bits:  0x{:06x}\nReceived:    0x{:06x}",
                    x, y, errors, y ^ errors
                );
                if z < 0 {
                    eprintln!("Nothing decoded");
                } else {
                    eprintln!("Decoded:     0x{:03x}", z);
                }
                return false;
            }
        }
    }

    // Suppress unused warnings
    let _ = (not_decoded_ct, decoded_ok_ct, decoded_wrong_ct);

    true
}
