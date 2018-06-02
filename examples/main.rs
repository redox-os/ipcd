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

    println!("Testing events...");

    let thread = thread::spawn(move || -> io::Result<()> {
        println!("--> Thread: Sleeping for 1 second...");
        thread::sleep(Duration::from_secs(1));
        println!("--> Thread: Writing...");
        client.write(b"hello")?;
        client.flush()?;
        println!("--> Thread: Sleeping for 1 second...");
        thread::sleep(Duration::from_secs(1));
        println!("--> Thread: Dropping...");
        drop(client);
        println!("--> Thread: Sleeping for 3 seconds...");
        thread::sleep(Duration::from_secs(3));
        println!("--> Thread: Opening new...");
        let mut client = File::open("chan:hello_world")?;
        let mut buf = [0; 5];
        assert_eq!(client.read(&mut buf)?, 0);
        println!("--> Thread: Closing...");
        Ok(())
    });

    let mut event_file = File::open("event:")?;
    let mut time_file = File::open(format!("time:{}", syscall::CLOCK_MONOTONIC))?;

    let mut time = syscall::TimeSpec::default();
    time_file.read(&mut time)?;
    time.tv_sec += 3;
    time_file.write(&time)?;

    event_file.write(&syscall::Event {
        id: dup.as_raw_fd(),
        flags: syscall::EVENT_READ | syscall::EVENT_WRITE,
        data: 0
    })?;
    event_file.write(&syscall::Event {
        id: server.as_raw_fd(),
        flags: syscall::EVENT_WRITE | syscall::EVENT_READ,
        data: 1
    })?;
    event_file.write(&syscall::Event {
        id: time_file.as_raw_fd(),
        flags: syscall::EVENT_READ,
        data: 2
    })?;

    let mut event = syscall::Event::default();

    event_file.read(&mut event)?;
    assert_eq!(event.id, dup.as_raw_fd());
    assert_eq!(event.flags, syscall::EVENT_WRITE);
    assert_eq!(event.data, 0);
    println!("-> Writable event");

    for _ in 0..2 {
        event_file.read(&mut event)?;
        assert_eq!(event.id, dup.as_raw_fd());
        assert_eq!(event.flags, syscall::EVENT_READ);
        assert_eq!(event.data, 0);
        println!("-> Readable event");

        dup.read(&mut buf)?;
    }

    event_file.read(&mut event)?;
    assert_eq!(event.id, time_file.as_raw_fd());
    assert_eq!(event.flags, syscall::EVENT_READ);
    assert_eq!(event.data, 2);
    println!("-> Timed out");

    event_file.read(&mut event)?;
    assert_eq!(event.id, server.as_raw_fd());
    assert_eq!(event.flags, syscall::EVENT_WRITE);
    assert_eq!(event.data, 1);
    println!("-> Read accept event");

    let dup = syscall::dup(server.as_raw_fd(), b"listen").map_err(from_syscall_error)?;
    let dup = unsafe { File::from_raw_fd(dup) };

    drop(dup);

    thread.join().unwrap()?;

    println!("Everything tested!");

    Ok(())
}
