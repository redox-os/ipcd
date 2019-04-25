use std::{
    fs::File,
    io::{self, prelude::*},
    os::unix::io::{AsRawFd, FromRawFd, RawFd}
};

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}
fn nonblock(file: &File) -> io::Result<()> {
    syscall::fcntl(file.as_raw_fd() as usize, syscall::F_SETFL, syscall::O_NONBLOCK)
        .map(|_| ())
        .map_err(from_syscall_error)
}
fn dup(file: &File, buf: &str) -> io::Result<File> {
    let stream = syscall::dup(file.as_raw_fd() as usize, buf.as_bytes()).map_err(from_syscall_error)?;
    Ok(unsafe { File::from_raw_fd(stream as RawFd) })
}

fn main() -> io::Result<()> {
    let server = File::create("chan:hello_world")?;

    nonblock(&server)?;

    let mut event_file = File::open("event:")?;
    let mut time_file = File::open(format!("time:{}", syscall::CLOCK_MONOTONIC))?;

    let mut time = syscall::TimeSpec::default();
    time_file.read(&mut time)?;
    time.tv_sec += 1;
    time_file.write(&time)?;
    time.tv_sec += 2;
    time_file.write(&time)?;
    time.tv_sec += 2;
    time_file.write(&time)?;

    const TOKEN_TIMER: usize = 0;
    const TOKEN_STREAM: usize = 1;
    const TOKEN_SERVER: usize = 2;
    const TOKEN_CLIENT: usize = 3;

    event_file.write(&syscall::Event {
        id: time_file.as_raw_fd() as usize,
        flags: syscall::EVENT_READ,
        data: TOKEN_TIMER
    })?;
    event_file.write(&syscall::Event {
        id: server.as_raw_fd() as usize,
        flags: syscall::EVENT_WRITE | syscall::EVENT_READ,
        data: TOKEN_SERVER
    })?;

    let mut event = syscall::Event::default();

    println!("Testing accept events...");

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_TIMER);
    assert_eq!(event.flags, syscall::EVENT_READ);
    println!("-> Timed out");

    let mut client = File::open("chan:hello_world")?;
    event_file.write(&syscall::Event {
        id: client.as_raw_fd() as usize,
        flags: syscall::EVENT_WRITE | syscall::EVENT_READ,
        data: TOKEN_CLIENT
    })?;

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_SERVER);
    assert_eq!(event.flags, syscall::EVENT_WRITE | syscall::EVENT_READ);
    println!("-> Accept event");

    println!("Testing write events...");

    let mut stream = dup(&server, "listen")?;

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_CLIENT);
    assert_eq!(event.flags, syscall::EVENT_WRITE);
    println!("-> Writable event");

    event_file.write(&syscall::Event {
        id: stream.as_raw_fd() as usize,
        flags: syscall::EVENT_READ | syscall::EVENT_WRITE,
        data: TOKEN_STREAM
    })?;

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_STREAM);
    assert_eq!(event.flags, syscall::EVENT_WRITE);
    println!("-> Writable event");

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_TIMER);
    assert_eq!(event.flags, syscall::EVENT_READ);
    println!("-> Timed out");

    println!("Testing read events...");

    client.write(b"a")?;

    let mut buf = [0; 5];

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_STREAM);
    assert_eq!(event.flags, syscall::EVENT_READ);
    println!("-> Readable event");

    assert_eq!(stream.read(&mut buf)?, 1);
    assert_eq!(buf[0], b'a');

    stream.write(b"b")?;

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_CLIENT);
    assert_eq!(event.flags, syscall::EVENT_READ);
    println!("-> Readable event");

    assert_eq!(client.read(&mut buf)?, 1);
    assert_eq!(buf[0], b'b');

    drop(client);

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_STREAM);
    println!("-> Readable event (EOF)");

    assert_eq!(stream.read(&mut buf)?, 0);

    event_file.read(&mut event)?;
    assert_eq!(event.data, TOKEN_TIMER);
    println!("-> Timed out");

    println!("Everything tested!");
    Ok(())
}
