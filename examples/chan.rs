extern crate syscall;

use std::{
    fs::File,
    io::{self, prelude::*},
    os::unix::io::{AsRawFd, FromRawFd},
    thread,
    time::Duration
};

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}

fn main() -> io::Result<()> {
    let server = File::create("chan:hello_world")?;
    let mut client = File::open("chan:hello_world")?;

    let dup = syscall::dup(server.as_raw_fd(), b"listen").map_err(from_syscall_error)?;
    let mut dup = unsafe { File::from_raw_fd(dup) };

    println!("Testing basic I/O...");

    dup.write(b"abc")?;
    dup.flush()?;

    let mut buf = [0; 5];
    assert_eq!(client.read(&mut buf)?, 3);
    assert_eq!(&buf[..3], b"abc");

    println!("Testing blocking I/O...");

    let mut client_clone = client.try_clone()?;

    let thread = thread::spawn(move || -> io::Result<()> {
        thread::sleep(Duration::from_secs(1));
        client_clone.write(b"def")?;
        client_clone.flush()?;
        Ok(())
    });

    assert_eq!(dup.read(&mut buf)?, 3);
    assert_eq!(&buf[..3], b"def");

    thread.join().unwrap().unwrap();

    println!("Testing non-blocking I/O...");

    syscall::fcntl(client.as_raw_fd(), syscall::F_SETFL, syscall::O_NONBLOCK)
        .map_err(from_syscall_error)?;
    assert_eq!(client.read(&mut buf).unwrap_err().kind(), io::ErrorKind::WouldBlock);

    println!("It works!");

    Ok(())
}
