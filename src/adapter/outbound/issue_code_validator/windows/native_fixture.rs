//! Standalone synthetic PE fixture, compiled by tests with rustc (no external crates).
#![windows_subsystem = "windows"]
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    if input.starts_with('{') {
        if !input.contains(r#""schemaVersion":1"#)
            || !input.contains(r#""target":"publish""#)
            || !input.contains("[masked-1]")
        {
            std::process::exit(25);
        }
        input = ["pass", "reject", "crash", "timeout"]
            .into_iter()
            .find(|mode| input.contains(&format!("\"title\":\"{mode}\"")))
            .unwrap_or("invalid")
            .to_owned();
    }
    let mut lines = input.lines();
    match lines.next().unwrap_or("pass") {
        "junction-control" => {
            let link = std::path::PathBuf::from(lines.next().unwrap());
            let target = std::path::PathBuf::from(lines.next().unwrap());
            std::fs::create_dir(&link).unwrap();
            match create_junction(&link, &target) {
                Ok(()) => println!("junction control created"),
                Err((stage, error)) => println!(
                    "junction control denied at {stage}: {:?}",
                    error.raw_os_error()
                ),
            }
            std::process::exit(1);
        }
        "profile" => profile_payload(lines.next().unwrap(), lines.next().unwrap()),
        "pass" => println!("native fixture passed"),
        "reject" => {
            println!("synthetic validation failed");
            std::process::exit(1);
        }
        "crash" => std::process::exit(23),
        "invalid-utf8-reject" => {
            std::io::stdout().write_all(&[0xff; 4096]).unwrap();
            std::process::exit(1);
        }
        "long-reject" => {
            for _ in 0..16 {
                std::io::stdout().write_all(&[b'x'; 4096]).unwrap();
            }
            std::process::exit(1);
        }
        "timeout" => loop {
            std::thread::sleep(Duration::from_millis(50));
        },
        "flood" => {
            let block = [b'x'; 4096];
            loop {
                std::io::stdout().write_all(&block).unwrap();
                std::io::stderr().write_all(&block).unwrap();
            }
        }
        "files" => {
            for path in lines {
                if std::fs::read(path).is_ok()
                    || std::fs::OpenOptions::new().write(true).open(path).is_ok()
                {
                    println!("outside file access was allowed");
                    std::process::exit(1);
                }
            }
        }
        "network" => {
            let address: std::net::SocketAddr = lines.next().unwrap().parse().unwrap();
            // Rust std asserts WSAStartup success. Query its direct return first so an
            // LPAC startup denial is observed, rather than hidden by panic=abort.
            #[link(name = "ws2_32")]
            unsafe extern "system" {
                fn WSAStartup(version: u16, data: *mut u8) -> i32;
            }
            let mut data = [0usize; 64];
            let startup = unsafe { WSAStartup(0x0202, data.as_mut_ptr().cast()) };
            if startup != 0 {
                println!("network denied at WSAStartup: {startup}");
                std::process::exit(1);
            }
            match std::net::TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
                Ok(_) => println!("network access was allowed"),
                Err(error) => {
                    println!("network denied at connect: {:?}", error.raw_os_error());
                    std::process::exit(1);
                }
            }
        }
        "environment" => {
            let variables: Vec<_> = std::env::vars().collect();
            if variables.iter().any(|(key, _)| {
                !["LOCALAPPDATA", "SYSTEMROOT", "TEMP", "TMP"]
                    .contains(&key.to_ascii_uppercase().as_str())
            }) {
                println!("unexpected inherited environment variable");
                std::process::exit(1);
            }
        }
        mode @ ("child" | "detached-child") => {
            use std::os::windows::process::CommandExt;
            let image = std::env::current_exe().unwrap();
            let mut command = std::process::Command::new(image);
            if mode == "detached-child" {
                command.creation_flags(0x0100_0008);
            }
            match command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    println!("child process creation was allowed");
                    std::process::exit(1);
                }
                Err(_) => println!("child process creation denied"),
            }
        }
        _ => std::process::exit(24),
    }
}

fn profile_payload(target: &str, expected_profile: &str) {
    let local = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
    if !local.starts_with(expected_profile) {
        println!("profile path was not isolated");
        std::process::exit(2);
    }
    let payload = local.join("validator-owned-payload");
    std::fs::create_dir(&payload).unwrap();
    std::fs::write(payload.join("content.txt"), b"synthetic profile payload").unwrap();
    let junction = payload.join("external-junction");
    std::fs::create_dir(&junction).unwrap();
    match create_junction(&junction, std::path::Path::new(target)) {
        Ok(()) => println!("profile payload created; junction created"),
        Err((stage, error)) => println!(
            "profile payload created; junction denied at {stage}: {:?}",
            error.raw_os_error()
        ),
    }
    // Return the synthetic observation through the runner's bounded message channel.
    std::process::exit(1);
}

fn create_junction(
    link: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), (&'static str, std::io::Error)> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        fn DeviceIoControl(
            handle: *mut c_void,
            code: u32,
            input: *const c_void,
            input_len: u32,
            output: *mut c_void,
            output_len: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let substitute: Vec<u16> = format!(r"\??\{}", target.display())
        .encode_utf16()
        .collect();
    let print: Vec<u16> = target.as_os_str().encode_wide().collect();
    let path_bytes = (substitute.len() + 1 + print.len() + 1) * 2;
    if path_bytes + 16 > 16_384 {
        return Err(("buffer", std::io::Error::other("junction target too long")));
    }
    let mut buffer = Vec::with_capacity(16 + path_bytes);
    buffer.extend(0xa000_0003u32.to_le_bytes());
    buffer.extend(((8 + path_bytes) as u16).to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    for value in [
        0,
        (substitute.len() * 2) as u16,
        ((substitute.len() + 1) * 2) as u16,
        (print.len() * 2) as u16,
    ] {
        buffer.extend(value.to_le_bytes());
    }
    for value in substitute.into_iter().chain([0]).chain(print).chain([0]) {
        buffer.extend(value.to_le_bytes());
    }
    let name: Vec<u16> = link.as_os_str().encode_wide().chain([0]).collect();
    // Own newly created empty directory only; do not follow a pre-existing reparse point.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            0x0000_0100, // FILE_WRITE_ATTRIBUTES: sufficient for mount-point FSCTL.
            0,
            std::ptr::null(),
            3,
            0x0220_0000,
            std::ptr::null_mut(),
        )
    };
    if handle == (-1isize) as *mut c_void {
        return Err(("open", std::io::Error::last_os_error()));
    }
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            0x0009_00a4,
            buffer.as_ptr().cast(),
            buffer.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    let error = (ok == 0).then(std::io::Error::last_os_error);
    unsafe {
        CloseHandle(handle);
    }
    match error {
        Some(error) => Err(("FSCTL_SET_REPARSE_POINT", error)),
        None => Ok(()),
    }
}
