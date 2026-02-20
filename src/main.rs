mod crc32c;
mod golay;
mod process;
mod speck;

use std::env;
use std::io::{self, BufRead, Write};

const PHNX_VERSION: &str = "4.0.1";
const PHNX_SELF_TEST_FAILED: i32 = 5;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() <= 1 {
        // Run self-tests
        if !speck::self_test() {
            std::process::exit(PHNX_SELF_TEST_FAILED);
        }

        if !golay::self_test() {
            std::process::exit(PHNX_SELF_TEST_FAILED);
        }

        eprintln!(
            "phnx version {}\n\n\
             Usage:\n\n\t{} [-c] file1 [-g] [file2] [...]\n\n\
             Encrypt a given file or files, add error correction bits, split into eight slices.\n\
             When given a slice, read all eight slices, correct errors if possible, then decrypt the original file.\n\
             Option -c turns on compatibility mode (encryption only, no error correction) for the files that follow,\n\
             option -g turns it off. Password can be passed via environment variable PHNX_PASSWORD.",
            PHNX_VERSION, args[0]
        );

        #[cfg(all(target_feature = "sse4.2", target_feature = "avx2", target_feature = "bmi2"))]
        eprintln!("Will use SSE4.2, AVX2, and BMI instructions.");

        #[cfg(all(target_feature = "sse4.2", target_feature = "avx2", not(target_feature = "bmi2")))]
        eprintln!("Will use SSE4.2 and AVX2 instructions.");

        #[cfg(all(target_feature = "bmi2", not(target_feature = "avx2")))]
        eprintln!("Will use BMI2 instructions.");

        std::process::exit(process::PHNX_OK);
    }

    let mut first_attempt = String::new();
    let password: String;

    match env::var("PHNX_PASSWORD") {
        Ok(pw) => {
            eprintln!("Using password from environment variable");
            password = pw;
        }
        Err(_) => {
            let stdin = io::stdin();
            let mut reader = stdin.lock();

            eprint!("Enter encryption key (32 chars max): ");
            io::stderr().flush().ok();
            reader.read_line(&mut first_attempt).ok();
            // Strip trailing newline
            if first_attempt.ends_with('\n') {
                first_attempt.pop();
                if first_attempt.ends_with('\r') {
                    first_attempt.pop();
                }
            }

            eprint!("Enter encryption key again         : ");
            io::stderr().flush().ok();
            let mut second_attempt = String::new();
            reader.read_line(&mut second_attempt).ok();
            if second_attempt.ends_with('\n') {
                second_attempt.pop();
                if second_attempt.ends_with('\r') {
                    second_attempt.pop();
                }
            }

            if first_attempt != second_attempt {
                eprintln!("Keys don't match");
                std::process::exit(process::PHNX_WRONG_PASSWORD);
            }
            password = first_attempt;
        }
    }

    // Convert password to four little-endian 64-bit words
    let pw_bytes = password.as_bytes();
    let mut bytes_left = pw_bytes.len();
    if bytes_left < 16 {
        eprintln!("WARNING: password is less than 16 characters long");
    } else if bytes_left > 32 {
        eprintln!(
            "WARNING: password is longer than 32 characters, only using the first 32"
        );
    }

    let mut k = [0u64; 4];
    for i in 0..4 {
        let start = i * 8;
        let len = if bytes_left > 8 { 8 } else { bytes_left };
        k[i] = speck::bytes_to_uint64(&pw_bytes[start..start + len]);
        if bytes_left <= 8 {
            break;
        }
        bytes_left -= 8;
    }

    let schedule = speck::speck_schedule(&k);

    // Iterate over files
    let mut ok_ct: u32 = 0;
    let mut fail_ct: u32 = 0;
    let mut compatibility_mode = false;
    let mut last_error_code = process::PHNX_OK;

    for i in 1..args.len() {
        if args[i] == "-c" {
            compatibility_mode = true;
            continue;
        }
        if args[i] == "-g" {
            compatibility_mode = false;
            continue;
        }
        let result = process::process_one_file(&args[i], &schedule, compatibility_mode);
        if result != process::PHNX_OK {
            last_error_code = result;
            fail_ct += 1;
        } else {
            ok_ct += 1;
        }
    }

    if ok_ct + fail_ct > 1 {
        eprintln!("{} files, {} errors", ok_ct + fail_ct, fail_ct);
    }
    std::process::exit(last_error_code);
}
