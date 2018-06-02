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
    let mut buf = [0; 5];
    let server = File::create("chan:hello_world")?;
    {
        let mut client = File::open("chan:hello_world")?;
        // First client not accepted yet
        assert_eq!(File::open("chan:hello_world").unwrap_err().kind(), io::ErrorKind::ConnectionRefused);

        let dup = syscall::dup(server.as_raw_fd(), b"listen").map_err(from_syscall_error)?;
        let mut dup = unsafe { File::from_raw_fd(dup) };

        println!("Testing basic I/O...");

        dup.write(b"abc")?;
        dup.flush()?;
        println!("-> Wrote message");

        assert_eq!(client.read(&mut buf)?, 3);
        assert_eq!(&buf[..3], b"abc");
        println!("-> Read message");

        println!("Testing close...");

        drop(client);
        assert_eq!(dup.write(b"a").unwrap_err().kind(), io::ErrorKind::NotConnected);
        assert_eq!(dup.read(&mut buf)?, 0);
    }
    println!("Testing alternative connect method...");
    let client = syscall::dup(server.as_raw_fd(), b"connect").map_err(from_syscall_error)?;
    let mut client = unsafe { File::from_raw_fd(client) };

    let dup = syscall::dup(server.as_raw_fd(), b"listen").map_err(from_syscall_error)?;
    let mut dup = unsafe { File::from_raw_fd(dup) };

    println!("Testing blocking I/O...");

    let mut client_clone = client.try_clone()?;

    let thread = thread::spawn(move || -> io::Result<()> {
        println!("--> Thread: Sleeping for 1 second...");
        thread::sleep(Duration::from_secs(1));
        println!("--> Thread: Writing...");
        client_clone.write(b"def")?;
        client_clone.flush()?;
        Ok(())
    });

    assert_eq!(dup.read(&mut buf)?, 3);
    assert_eq!(&buf[..3], b"def");
    println!("-> Read message");

    thread.join().unwrap().unwrap();

    println!("Testing non-blocking I/O...");

    syscall::fcntl(client.as_raw_fd(), syscall::F_SETFL, syscall::O_NONBLOCK)
        .map_err(from_syscall_error)?;
    syscall::fcntl(server.as_raw_fd(), syscall::F_SETFL, syscall::O_NONBLOCK)
        .map_err(from_syscall_error)?;
    assert_eq!(client.read(&mut buf).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    println!("-> Read would block");
    match syscall::dup(server.as_raw_fd(), b"listen") {
        Ok(dup) => {
            unsafe { File::from_raw_fd(dup); }
            panic!("this is supposed to fail");
        },
        Err(err) => {
            let err = from_syscall_error(err);
            assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        }
    }
    println!("-> Accept would block");

    println!("Everything tested!");
    Ok(())
}
