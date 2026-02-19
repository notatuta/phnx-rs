#[inline]
pub fn speck_round(x: &mut u64, y: &mut u64, k: u64) {
    *x = x.rotate_right(8);
    *x = x.wrapping_add(*y);
    *x ^= k;
    *y = y.rotate_left(3);
    *y ^= *x;
}

pub fn speck_schedule(key: &[u64; 4]) -> [u64; 34] {
    let mut schedule = [0u64; 34];
    let mut a = key[0];
    let mut bcd = [key[1], key[2], key[3]];
    for i in 0u64..33 {
        schedule[i as usize] = a;
        speck_round(&mut bcd[(i % 3) as usize], &mut a, i);
    }
    schedule[33] = a;
    schedule
}

pub fn speck_encrypt(plaintext: &[u64; 2], schedule: &[u64; 34]) -> [u64; 2] {
    let mut x = plaintext[1];
    let mut y = plaintext[0];
    for i in 0..34 {
        speck_round(&mut x, &mut y, schedule[i]);
    }
    [y, x]
}

#[cfg(target_feature = "avx2")]
pub fn speck_encrypt4(plaintext: &[u64; 8], schedule: &[u64; 34]) -> [u64; 8] {
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;

    unsafe {
        let mut x = _mm256_set_epi64x(
            plaintext[7] as i64,
            plaintext[6] as i64,
            plaintext[5] as i64,
            plaintext[4] as i64,
        );
        let mut y = _mm256_set_epi64x(
            plaintext[3] as i64,
            plaintext[2] as i64,
            plaintext[1] as i64,
            plaintext[0] as i64,
        );
        for i in 0..34 {
            let si = schedule[i] as i64;
            // rotate x right by 8
            x = _mm256_or_si256(
                _mm256_srli_epi64(x, 8),
                _mm256_slli_epi64(x, 56),
            );
            x = _mm256_add_epi64(x, y);
            x = _mm256_xor_si256(x, _mm256_set_epi64x(si, si, si, si));
            // rotate y left by 3
            y = _mm256_or_si256(
                _mm256_slli_epi64(y, 3),
                _mm256_srli_epi64(y, 61),
            );
            y = _mm256_xor_si256(y, x);
        }
        let mut ct = [0u64; 8];
        _mm256_storeu_si256(ct[4..].as_mut_ptr() as *mut __m256i, x);
        _mm256_storeu_si256(ct[0..].as_mut_ptr() as *mut __m256i, y);
        ct
    }
}

#[cfg(not(target_feature = "avx2"))]
pub fn speck_encrypt4(plaintext: &[u64; 8], schedule: &[u64; 34]) -> [u64; 8] {
    let mut ct = *plaintext;
    for i in 0..34 {
        let si = schedule[i];
        speck_round(&mut ct[4], &mut ct[0], si);
        speck_round(&mut ct[5], &mut ct[1], si);
        speck_round(&mut ct[6], &mut ct[2], si);
        speck_round(&mut ct[7], &mut ct[3], si);
    }
    ct
}

pub fn bytes_to_uint64(bytes: &[u8]) -> u64 {
    let mut w = 0u64;
    for (i, &b) in bytes.iter().enumerate() {
        w |= (b as u64) << (i * 8);
    }
    w
}

pub fn self_test() -> bool {
    let key: [u64; 4] = [
        0x0706050403020100u64,
        0x0f0e0d0c0b0a0908u64,
        0x1716151413121110u64,
        0x1f1e1d1c1b1a1918u64,
    ];
    let plaintext: [u64; 2] = [0x202e72656e6f6f70u64, 0x65736f6874206e49u64];
    let expected: [u64; 2] = [0x4eeeb48d9c188f43u64, 0x4109010405c0f53eu64];

    let schedule = speck_schedule(&key);
    let observed = speck_encrypt(&plaintext, &schedule);

    if expected[0] != observed[0] || expected[1] != observed[1] {
        eprintln!("speck_encrypt() self-test failed");
        eprintln!(
            "Expected 0x{:x}, 0x{:x}",
            expected[0], expected[1]
        );
        eprintln!(
            "Observed 0x{:x}, 0x{:x}",
            observed[0], observed[1]
        );
        return false;
    }

    let converted = [
        bytes_to_uint64(b"pooner. "),
        bytes_to_uint64(b"In those"),
    ];
    if plaintext[0] != converted[0] || plaintext[1] != converted[1] {
        eprintln!("bytes_to_uint64() self-test failed");
        eprintln!(
            "Expected 0x{:x}, 0x{:x}",
            plaintext[0], plaintext[1]
        );
        eprintln!(
            "Observed 0x{:x}, 0x{:x}",
            converted[0], converted[1]
        );
        return false;
    }

    true
}
