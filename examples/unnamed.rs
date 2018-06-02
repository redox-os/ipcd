extern crate syscall;

use std::{
    fs::File,
    io::{self, prelude::*},
    os::unix::io::{AsRawFd, FromRawFd}
};

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}

fn main() -> io::Result<()> {
    let mut buf = [0; 5];
    let server = File::create("chan:")?;

    let client = syscall::dup(server.as_raw_fd(), b"connect").map_err(from_syscall_error)?;
    let mut client = unsafe { File::from_raw_fd(client) };

    let dup = syscall::dup(server.as_raw_fd(), b"listen").map_err(from_syscall_error)?;
    let mut dup = unsafe { File::from_raw_fd(dup) };

    println!("Testing basic I/O...");

    dup.write(b"abc")?;
    dup.flush()?;
    println!("-> Wrote message");

    assert_eq!(client.read(&mut buf)?, 3);
    assert_eq!(&buf[..3], b"abc");
    println!("-> Read message");

    println!("Testing connecting to unnamed socket by name (makes no sense)...");
    assert_eq!(File::open("chan:").unwrap_err().kind(), io::ErrorKind::NotFound);

    println!("Everything tested!");
    Ok(())
}
