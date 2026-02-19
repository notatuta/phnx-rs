use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::SystemTime;

use crate::crc32c::Crc32c;
use crate::golay::GolayCode;
use crate::speck;

pub const PHNX_OK: i32 = 0;
pub const PHNX_IO_ERROR: i32 = 1;
pub const PHNX_WRONG_PASSWORD: i32 = 2;
pub const PHNX_UNCORRECTABLE_ERROR: i32 = 3;
pub const PHNX_FORMAT_ERROR: i32 = 4;

fn golay_read_and_decode(
    buffer: &mut [u8],
    bytes_to_read: usize,
    slices: &mut [Option<File>; 8],
    gc: &mut GolayCode,
) -> i32 {
    let mut block_offset = 0;
    while block_offset < bytes_to_read {
        // Read 3 bytes from each available slice into [u8; 24] laid out as 8x3
        let mut eighttriplets = [0u8; 24];
        for i in 0..8 {
            if let Some(ref mut f) = slices[i] {
                let base = i * 3;
                if f.read_exact(&mut eighttriplets[base..base + 3]).is_err() {
                    eprintln!("\nError reading from slice {}", (b'A' + i as u8) as char);
                    return PHNX_IO_ERROR;
                }
            }
        }

        // Convert to qwords for BMI2 path
        let qwords = [
            u64::from_le_bytes([
                eighttriplets[0],
                eighttriplets[1],
                eighttriplets[2],
                eighttriplets[3],
                eighttriplets[4],
                eighttriplets[5],
                eighttriplets[6],
                eighttriplets[7],
            ]),
            u64::from_le_bytes([
                eighttriplets[8],
                eighttriplets[9],
                eighttriplets[10],
                eighttriplets[11],
                eighttriplets[12],
                eighttriplets[13],
                eighttriplets[14],
                eighttriplets[15],
            ]),
            u64::from_le_bytes([
                eighttriplets[16],
                eighttriplets[17],
                eighttriplets[18],
                eighttriplets[19],
                eighttriplets[20],
                eighttriplets[21],
                eighttriplets[22],
                eighttriplets[23],
            ]),
        ];

        let mut twelvebytes = [0u8; 12];
        let mut twelvebytes_q = [0u64; 2];

        for i in 0..8 {
            let codeword;
            #[cfg(target_feature = "bmi2")]
            {
                #[cfg(target_arch = "x86_64")]
                use std::arch::x86_64::_pext_u64;
                #[cfg(target_arch = "x86")]
                use std::arch::x86::_pext_u64;

                let mask: u64 = 0x0101010101010101u64 << i;
                let extracted_lo = unsafe { _pext_u64(qwords[0], mask) };
                let extracted_mid = unsafe { _pext_u64(qwords[1], mask) };
                let extracted_hi = unsafe { _pext_u64(qwords[2], mask) };
                codeword = (extracted_lo | (extracted_mid << 8) | (extracted_hi << 16)) as u32;
            }
            #[cfg(not(target_feature = "bmi2"))]
            {
                let mut cw = 0u32;
                for k in 0..8 {
                    for t in 0..3 {
                        if eighttriplets[k * 3 + t] & (1 << i) != 0 {
                            cw |= 1 << (k * 3 + t);
                        }
                    }
                }
                codeword = cw;
            }

            let x = gc.decode(codeword);

            #[cfg(target_feature = "bmi2")]
            {
                #[cfg(target_arch = "x86_64")]
                use std::arch::x86_64::_pdep_u64;
                #[cfg(target_arch = "x86")]
                use std::arch::x86::_pdep_u64;

                let mask: u64 = 0x0101010101010101u64 << i;
                let halfmask: u64 = mask & 0xffffffff;
                twelvebytes_q[0] |= unsafe { _pdep_u64(x as u64, mask) };
                twelvebytes_q[1] |= unsafe { _pdep_u64((x >> 8) as u64, halfmask) };
            }
            #[cfg(not(target_feature = "bmi2"))]
            {
                for j in 0..12 {
                    if x & (1 << j) != 0 {
                        twelvebytes[j] |= 1 << i;
                    }
                }
            }
        }

        #[cfg(target_feature = "bmi2")]
        {
            twelvebytes[..8].copy_from_slice(&twelvebytes_q[0].to_le_bytes());
            twelvebytes[8..12].copy_from_slice(&twelvebytes_q[1].to_le_bytes()[..4]);
        }

        let end = std::cmp::min(block_offset + 12, buffer.len());
        let copy_len = end - block_offset;
        buffer[block_offset..block_offset + copy_len]
            .copy_from_slice(&twelvebytes[..copy_len]);
        block_offset += 12;
    }
    PHNX_OK
}

fn golay_encode_and_write(
    data: &[u8],
    data_size: usize,
    slices: &mut [Option<File>; 8],
    gc: &mut GolayCode,
) -> i32 {
    let mut block_offset = 0;
    while block_offset < data_size {
        // Pad with zeroes
        let mut twelvebytes = [0u8; 12];
        let copy_size = std::cmp::min(12, data_size - block_offset);
        twelvebytes[..copy_size].copy_from_slice(&data[block_offset..block_offset + copy_size]);

        let twelvebytes_q = [
            u64::from_le_bytes([
                twelvebytes[0],
                twelvebytes[1],
                twelvebytes[2],
                twelvebytes[3],
                twelvebytes[4],
                twelvebytes[5],
                twelvebytes[6],
                twelvebytes[7],
            ]),
            u64::from_le_bytes([
                twelvebytes[8],
                twelvebytes[9],
                twelvebytes[10],
                twelvebytes[11],
                0,
                0,
                0,
                0,
            ]),
        ];

        let mut eighttriplets = [0u8; 24];
        let mut eighttriplets_q = [0u64; 3];

        for i in 0..8 {
            let x;
            #[cfg(target_feature = "bmi2")]
            {
                #[cfg(target_arch = "x86_64")]
                use std::arch::x86_64::_pext_u64;
                #[cfg(target_arch = "x86")]
                use std::arch::x86::_pext_u64;

                let mask: u64 = 0x0101010101010101u64 << i;
                let halfmask: u64 = mask & 0xffffffff;
                let bits0to7 = unsafe { _pext_u64(twelvebytes_q[0], mask) };
                let bits8to11 = unsafe { _pext_u64(twelvebytes_q[1], halfmask) };
                x = (bits0to7 | (bits8to11 << 8)) as u32;
            }
            #[cfg(not(target_feature = "bmi2"))]
            {
                let mut val = 0u32;
                for j in 0..12 {
                    if twelvebytes[j] & (1 << i) != 0 {
                        val |= 1 << j;
                    }
                }
                x = val;
            }

            let codeword = gc.encode(x);

            #[cfg(target_feature = "bmi2")]
            {
                #[cfg(target_arch = "x86_64")]
                use std::arch::x86_64::_pdep_u64;
                #[cfg(target_arch = "x86")]
                use std::arch::x86::_pdep_u64;

                let mask: u64 = 0x0101010101010101u64 << i;
                eighttriplets_q[0] |=
                    unsafe { _pdep_u64((codeword & 0xff) as u64, mask) };
                eighttriplets_q[1] |=
                    unsafe { _pdep_u64(((codeword >> 8) & 0xff) as u64, mask) };
                eighttriplets_q[2] |=
                    unsafe { _pdep_u64(((codeword >> 16) & 0xff) as u64, mask) };
            }
            #[cfg(not(target_feature = "bmi2"))]
            {
                for k in 0..8 {
                    for t in 0..3 {
                        if codeword & (1 << (k * 3 + t)) != 0 {
                            eighttriplets[k * 3 + t] |= 1 << i;
                        }
                    }
                }
            }
        }

        #[cfg(target_feature = "bmi2")]
        {
            eighttriplets[0..8].copy_from_slice(&eighttriplets_q[0].to_le_bytes());
            eighttriplets[8..16].copy_from_slice(&eighttriplets_q[1].to_le_bytes());
            eighttriplets[16..24].copy_from_slice(&eighttriplets_q[2].to_le_bytes());
        }

        // Write each slice
        for i in 0..8 {
            let base = i * 3;
            if let Some(ref mut f) = slices[i] {
                if f.write_all(&eighttriplets[base..base + 3]).is_err() {
                    eprintln!("\nError writing slice {}", (b'A' + i as u8) as char);
                    return PHNX_IO_ERROR;
                }
            }
        }

        block_offset += 12;
    }
    PHNX_OK
}

#[allow(unused_assignments)]
pub fn process_one_file(
    filename: &str,
    schedule: &[u64; 34],
    compatibility_mode: bool,
) -> i32 {
    let mut check_checksum = false;
    let mut expected_checksum: u32 = 0;
    let mut check_crc32c = false;
    let mut expected_crc32c: u32 = 0;
    let mut append_suffix = true;
    let mut nonce: u64 = 0;
    let mut golay_encode = !compatibility_mode;
    let mut golay_decode = false;
    let mut slices: [Option<File>; 8] = [None, None, None, None, None, None, None, None];
    let mut length: i64 = 0;
    let mut remaining_length: i64 = 0;
    let mut gc = GolayCode::new();

    // p_offset: position of the last character in filename (like C++ p)
    let fname_bytes = filename.as_bytes();
    if fname_bytes.is_empty() {
        eprintln!("Empty filename");
        return PHNX_IO_ERROR;
    }

    let p = fname_bytes.len() - 1; // index of last char

    // Check for .phnx_[A-H]
    if p >= 6 {
        let suffix_start = p - 6;
        let suffix = &filename[suffix_start..];
        if suffix.len() == 7
            && suffix.starts_with(".phnx_")
            && (b'A'..=b'H').contains(&suffix.as_bytes()[6])
        {
            let mut missing_ct = 0;
            for i in 0..8 {
                let mut slice_filename = filename.to_string();
                let last = slice_filename.len() - 1;
                unsafe {
                    slice_filename.as_bytes_mut()[last] = b'A' + i as u8;
                }
                match File::open(&slice_filename) {
                    Ok(f) => slices[i] = Some(f),
                    Err(_) => {
                        eprintln!("Cannot open {}", slice_filename);
                        if missing_ct > 0 {
                            eprintln!(
                                "More than one slice is missing, not enough to recover"
                            );
                            return PHNX_UNCORRECTABLE_ERROR;
                        }
                        missing_ct += 1;
                    }
                }
            }
            golay_decode = true;
            golay_encode = false;
            check_crc32c = true;
            append_suffix = false;
        }
    }

    if p >= 6 {
        if golay_decode {
            let mut display_name = filename.to_string();
            let last = display_name.len() - 1;
            unsafe {
                display_name.as_bytes_mut()[last] = b'[';
            }
            println!("Processing {}A-H]", display_name);
        } else {
            println!("Processing {}", filename);
        }
    }

    // Check for .encrypted or .encrypted-XXXXXXXX
    // Store the position where we found the hex suffix start (for renaming)
    let mut hex_suffix_rename_end: Option<usize> = None;

    if !golay_decode && p >= 9 {
        if filename.ends_with(".encrypted") {
            check_crc32c = true;
            append_suffix = false;
            golay_encode = false;
        } else {
            // Check for .encrypted-XXXXXXXX
            // Find the dash after .encrypted
            if let Some(dot_pos) = filename.rfind(".encrypted-") {
                let hex_part = &filename[dot_pos + 11..];
                if !hex_part.is_empty() && hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
                    if let Ok(cs) = u32::from_str_radix(hex_part, 16) {
                        expected_checksum = cs;
                        check_checksum = true;
                        append_suffix = false;
                        golay_encode = false;
                        hex_suffix_rename_end = Some(dot_pos);
                    }
                }
            }
        }
    }

    // Open files
    let mut f: Option<File>;

    if golay_decode {
        // Read suffix (2 blocks = 48 bytes = 6 bytes per slice)
        for i in 0..8 {
            if let Some(ref mut s) = slices[i] {
                if s.seek(SeekFrom::End(-6)).is_err() {
                    eprintln!("\nError seeking in slice {}", (b'A' + i as u8) as char);
                    return PHNX_IO_ERROR;
                }
            }
        }
        let mut suffix_bytes = [0u8; 24];
        let ret = golay_read_and_decode(&mut suffix_bytes, 24, &mut slices, &mut gc);
        if ret != PHNX_OK {
            return ret;
        }
        for i in 0..8 {
            if let Some(ref mut s) = slices[i] {
                if s.seek(SeekFrom::Start(0)).is_err() {
                    return PHNX_IO_ERROR;
                }
            }
        }

        // Extract suffix
        let suffix_0 = u64::from_le_bytes(suffix_bytes[0..8].try_into().unwrap());
        let suffix_1 = u64::from_le_bytes(suffix_bytes[8..16].try_into().unwrap());
        let suffix_2 = u64::from_le_bytes(suffix_bytes[16..24].try_into().unwrap());

        // Decrypt suffix with nonce=-1, counter=-1, -2
        let nonce_ctr_m1 = [0xffffffffffffffffu64, 0xffffffffffffffffu64];
        let nonce_ctr_m2 = [0xffffffffffffffffu64, 0xfffffffffffffffeu64];
        let gamma1 = speck::speck_encrypt(&nonce_ctr_m1, schedule);
        let gamma2 = speck::speck_encrypt(&nonce_ctr_m2, schedule);

        let s0 = suffix_0 ^ gamma1[0];
        let s1 = suffix_1 ^ gamma1[1];
        let s2 = suffix_2 ^ gamma2[0];

        let crc32c0 = s0 as u32;
        let crc32c1 = (s0 >> 32) as u32;
        if crc32c0 != crc32c1 {
            eprintln!("CRC mismatch, wrong password?");
            return PHNX_WRONG_PASSWORD;
        }
        check_crc32c = true;
        expected_crc32c = crc32c0;
        nonce = s1;
        length = s2 as i64;
        remaining_length = length;

        // Create output file (trim .phnx_X)
        let base_filename = &filename[..filename.len() - 7];
        match File::create(base_filename) {
            Ok(file) => f = Some(file),
            Err(_) => {
                eprintln!("Cannot create {}", base_filename);
                return PHNX_IO_ERROR;
            }
        }
    } else {
        if golay_encode {
            match File::open(filename) {
                Ok(file) => f = Some(file),
                Err(_) => {
                    eprintln!("Cannot open {}", filename);
                    return PHNX_IO_ERROR;
                }
            }
        } else {
            match OpenOptions::new().read(true).write(true).open(filename) {
                Ok(file) => f = Some(file),
                Err(_) => {
                    eprintln!("Cannot open {}", filename);
                    return PHNX_IO_ERROR;
                }
            }
        }

        // Determine file length
        let file_ref = f.as_mut().unwrap();
        length = match file_ref.seek(SeekFrom::End(0)) {
            Ok(len) => len as i64,
            Err(_) => {
                eprintln!("Cannot determine file length");
                return PHNX_IO_ERROR;
            }
        };
        if file_ref.seek(SeekFrom::Start(0)).is_err() {
            return PHNX_IO_ERROR;
        }
        remaining_length = length;
        nonce = length as u64;

        if check_crc32c && !golay_decode {
            if length < 16 {
                eprintln!("\nNo suffix in {}", filename);
                return PHNX_FORMAT_ERROR;
            }
            // Read the suffix
            let file_ref = f.as_mut().unwrap();
            if file_ref.seek(SeekFrom::Start((length - 16) as u64)).is_err() {
                return PHNX_IO_ERROR;
            }
            let mut suffix_buf = [0u8; 16];
            if file_ref.read_exact(&mut suffix_buf).is_err() {
                eprintln!("\nError reading suffix from {}", filename);
                return PHNX_IO_ERROR;
            }
            if file_ref.seek(SeekFrom::Start(0)).is_err() {
                return PHNX_IO_ERROR;
            }

            let s0 = u64::from_le_bytes(suffix_buf[0..8].try_into().unwrap());
            let s1 = u64::from_le_bytes(suffix_buf[8..16].try_into().unwrap());

            // Decrypt suffix on nonce -1 and counter -1
            let all_ones = [0xffffffffffffffffu64, 0xffffffffffffffffu64];
            let gamma = speck::speck_encrypt(&all_ones, schedule);
            let s0 = s0 ^ gamma[0];
            let s1 = s1 ^ gamma[1];

            let crc32c0 = s0 as u32;
            let crc32c1 = (s0 >> 32) as u32;
            if crc32c0 != crc32c1 {
                eprintln!("CRC mismatch, maybe wrong password?");
                return PHNX_WRONG_PASSWORD;
            }
            expected_crc32c = crc32c0;
            nonce = s1;
            remaining_length = length - 16;
        }
    }

    if append_suffix || golay_encode {
        let mut random_number = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;

        #[cfg(target_feature = "rdrand")]
        unsafe {
            #[cfg(target_arch = "x86_64")]
            std::arch::x86_64::_rdrand64_step(&mut random_number);
            #[cfg(target_arch = "x86")]
            std::arch::x86::_rdrand64_step(&mut random_number);
        }

        nonce ^= random_number;
    }

    // Progress bar
    let mut total_notches: u32 = 10;
    {
        let mut x = remaining_length;
        while x > 0 {
            x >>= 1;
            total_notches += 1;
        }
    }
    eprint!(" ");
    for _ in 0..total_notches {
        eprint!(".");
    }
    eprint!(" \r ");
    let mut notches_shown: u32 = 0;

    let mut nonce_and_counter: [u64; 8] = [
        nonce, nonce, nonce, nonce, 0, 1, 2, 3,
    ];

    let mut crc32c_before = Crc32c::new();
    let mut crc32c_after = Crc32c::new();

    if golay_encode {
        for i in 0..8 {
            let mut slice_filename = filename.to_string();
            slice_filename.push_str(".phnx_");
            slice_filename.push((b'A' + i as u8) as char);
            match File::create(&slice_filename) {
                Ok(file) => slices[i] = Some(file),
                Err(_) => {
                    eprintln!("Cannot create {}", slice_filename);
                    return PHNX_IO_ERROR;
                }
            }
        }
    }

    while remaining_length > 0 {
        let mut buffer = [0u8; 16 * 4 * 12 * 100];
        let chunk_size = std::cmp::min(remaining_length as usize, buffer.len());

        if golay_decode {
            let ret =
                golay_read_and_decode(&mut buffer, chunk_size, &mut slices, &mut gc);
            if ret != PHNX_OK {
                return ret;
            }
        } else {
            let file_ref = f.as_mut().unwrap();
            let position = file_ref.stream_position().unwrap_or(0);
            if file_ref.read_exact(&mut buffer[..chunk_size]).is_err() {
                eprintln!("\nError reading {}", filename);
                return PHNX_IO_ERROR;
            }
            if !golay_encode {
                if file_ref.seek(SeekFrom::Start(position)).is_err() {
                    return PHNX_IO_ERROR;
                }
            }
        }

        // Update CRC32C before processing
        crc32c_before.update_slice(&buffer[..chunk_size]);

        // CTR mode encryption
        let mut offset = 0;
        while offset < chunk_size {
            let keystream = speck::speck_encrypt4(&nonce_and_counter, schedule);
            nonce_and_counter[4] += 4;
            nonce_and_counter[5] += 4;
            nonce_and_counter[6] += 4;
            nonce_and_counter[7] += 4;

            // XOR buffer with keystream in interleaved order [0,4,1,5,2,6,3,7]
            const KS_ORDER: [usize; 8] = [0, 4, 1, 5, 2, 6, 3, 7];
            for (block_idx, &ks_idx) in KS_ORDER.iter().enumerate() {
                for i in 0..8 {
                    let buf_pos = offset + block_idx * 8 + i;
                    if buf_pos < chunk_size {
                        buffer[buf_pos] ^= (keystream[ks_idx] >> (i * 8)) as u8;
                    }
                }
            }

            offset += 16 * 4;
        }

        // Update CRC32C after processing
        crc32c_after.update_slice(&buffer[..chunk_size]);

        if golay_encode {
            let ret =
                golay_encode_and_write(&buffer, chunk_size, &mut slices, &mut gc);
            if ret != PHNX_OK {
                return ret;
            }
        } else {
            let file_ref = f.as_mut().unwrap();
            if file_ref.write_all(&buffer[..chunk_size]).is_err() {
                eprintln!("\nError writing {}", filename);
                return PHNX_IO_ERROR;
            }
        }

        remaining_length -= chunk_size as i64;

        // Update progress bar
        let done = length - remaining_length;
        let notches_remaining =
            total_notches - (done as u32 * total_notches / length as u32).min(total_notches);
        if total_notches - notches_shown > notches_remaining {
            let notches_to_show = total_notches - notches_shown - notches_remaining;
            for _ in 0..notches_to_show {
                eprint!("o");
                notches_shown += 1;
            }
        }
    }

    // Clear progress bar
    eprint!("\r ");
    for _ in 0..total_notches {
        eprint!(" ");
    }
    eprint!(" \r");

    let crc32c_before_val = crc32c_before.finalize();
    let crc32c_after_val = crc32c_after.finalize();

    if golay_encode {
        let mut suffix = [0u64; 3];
        suffix[0] = ((crc32c_before_val as u64) << 32) | (crc32c_before_val as u64);
        suffix[1] = nonce;
        suffix[2] = length as u64;

        // Encrypt suffix with nonce=-1, counter=-1, -2
        let nonce_ctr_m1 = [0xffffffffffffffffu64, 0xffffffffffffffffu64];
        let nonce_ctr_m2 = [0xffffffffffffffffu64, 0xfffffffffffffffeu64];
        let gamma1 = speck::speck_encrypt(&nonce_ctr_m1, schedule);
        let gamma2 = speck::speck_encrypt(&nonce_ctr_m2, schedule);
        suffix[0] ^= gamma1[0];
        suffix[1] ^= gamma1[1];
        suffix[2] ^= gamma2[0];

        let mut suffix_bytes = [0u8; 24];
        suffix_bytes[0..8].copy_from_slice(&suffix[0].to_le_bytes());
        suffix_bytes[8..16].copy_from_slice(&suffix[1].to_le_bytes());
        suffix_bytes[16..24].copy_from_slice(&suffix[2].to_le_bytes());

        let ret = golay_encode_and_write(&suffix_bytes, 24, &mut slices, &mut gc);
        if ret != PHNX_OK {
            return ret;
        }

        // Close slices (drop them)
        for i in 0..8 {
            slices[i] = None;
        }

        return PHNX_OK;
    } else if append_suffix {
        let mut suffix = [0u64; 2];
        suffix[0] = ((crc32c_before_val as u64) << 32) | (crc32c_before_val as u64);
        suffix[1] = nonce;

        let all_ones = [0xffffffffffffffffu64, 0xffffffffffffffffu64];
        let gamma = speck::speck_encrypt(&all_ones, schedule);
        suffix[0] ^= gamma[0];
        suffix[1] ^= gamma[1];

        let file_ref = f.as_mut().unwrap();
        let mut suffix_bytes = [0u8; 16];
        suffix_bytes[0..8].copy_from_slice(&suffix[0].to_le_bytes());
        suffix_bytes[8..16].copy_from_slice(&suffix[1].to_le_bytes());
        if file_ref.write_all(&suffix_bytes).is_err() {
            eprintln!("\nError writing suffix");
            return PHNX_IO_ERROR;
        }
        drop(f);
        let new_filename = format!("{}.encrypted", filename);
        if fs::rename(filename, &new_filename).is_err() {
            eprintln!("Error renaming {} to {}", filename, new_filename);
            return PHNX_IO_ERROR;
        }
        return PHNX_OK;
    }

    // Close main file
    drop(f);

    if check_checksum {
        let checksum_in = [
            ((crc32c_before_val as u64) << 32) | (crc32c_after_val as u64),
            length as u64,
        ];
        let checksum_out = speck::speck_encrypt(&checksum_in, schedule);
        let checksum = checksum_out[0] as u32;

        if checksum != expected_checksum {
            eprintln!(
                "Checksum mismatch: expected 0x{:x}, got 0x{:x}",
                expected_checksum, checksum
            );
            return PHNX_FORMAT_ERROR;
        } else {
            let new_filename = &filename[..hex_suffix_rename_end.unwrap_or(0)];
            if fs::rename(filename, new_filename).is_err() {
                eprintln!("Error renaming {} to {}", filename, new_filename);
                return PHNX_IO_ERROR;
            }
            return PHNX_OK;
        }
    }

    if check_crc32c {
        if expected_crc32c != crc32c_after_val {
            eprintln!(
                "CRC32C mismatch: expected 0x{:x}, got 0x{:x}",
                expected_crc32c, crc32c_after_val
            );
            return PHNX_FORMAT_ERROR;
        } else if !golay_decode {
            // Remove .encrypted suffix from filename
            let new_filename = &filename[..filename.len() - 10]; // strip ".encrypted"
            if fs::rename(filename, new_filename).is_err() {
                eprintln!("Error renaming {} to {}", filename, new_filename);
                return PHNX_IO_ERROR;
            }
            // Truncate to remove the 16-byte suffix
            let trunc_file = OpenOptions::new().write(true).open(new_filename);
            match trunc_file {
                Ok(f) => {
                    if f.set_len((length - 16) as u64).is_err() {
                        eprintln!("Error truncating {}", new_filename);
                        return PHNX_IO_ERROR;
                    }
                }
                Err(_) => {
                    eprintln!("Error truncating {}", new_filename);
                    return PHNX_IO_ERROR;
                }
            }
        }
    }

    if golay_decode {
        if gc.corrected_codewords != 0 || gc.uncorrectable_codewords != 0 {
            eprintln!(
                "Processed {} Golay codewords, corrected {}, {} uncorrectable",
                gc.processed_codewords, gc.corrected_codewords, gc.uncorrectable_codewords
            );
        }
        if gc.uncorrectable_codewords != 0 {
            return PHNX_UNCORRECTABLE_ERROR;
        }
    }

    PHNX_OK
}
