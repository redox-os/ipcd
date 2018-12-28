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
fn nonblock(file: &File) -> io::Result<()> {
    syscall::fcntl(file.as_raw_fd(), syscall::F_SETFL, syscall::O_NONBLOCK)
        .map(|_| ())
        .map_err(from_syscall_error)
}
fn dup(file: &File, buf: &str) -> io::Result<File> {
    let stream = syscall::dup(file.as_raw_fd(), buf.as_bytes()).map_err(from_syscall_error)?;
    Ok(unsafe { File::from_raw_fd(stream) })
}

fn main() -> io::Result<()> {
    let mut buf = [0; 5];
    let server = File::create("chan:hello_world")?;
    {
        let mut client = File::open("chan:hello_world")?;
        // First client not accepted yet
        assert_eq!(File::open("chan:hello_world").unwrap_err().kind(), io::ErrorKind::ConnectionRefused);

        let mut stream = dup(&server, "listen")?;

        println!("Testing basic I/O...");

        stream.write(b"abc")?;
        stream.flush()?;
        println!("-> Wrote message");

        assert_eq!(client.read(&mut buf)?, 3);
        assert_eq!(&buf[..3], b"abc");
        println!("-> Read message");

        println!("Testing close...");

        drop(client);
        assert_eq!(stream.write(b"a").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(stream.read(&mut buf)?, 0);
    }
    println!("Testing alternative connect method...");

    let mut client = dup(&server, "connect")?;
    let mut stream = dup(&server, "listen")?;

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

    assert_eq!(stream.read(&mut buf)?, 3);
    assert_eq!(&buf[..3], b"def");
    println!("-> Read message");

    thread.join().unwrap().unwrap();

    println!("Testing non-blocking I/O...");

    nonblock(&client)?;
    nonblock(&server)?;

    assert_eq!(client.read(&mut buf).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    println!("-> Read would block");

    assert_eq!(dup(&server, "listen").unwrap_err().kind(), io::ErrorKind::WouldBlock);
    println!("-> Accept would block");

    drop(client);
    {
        let mut client = File::open("chan:hello_world")?;
        nonblock(&client)?;

        assert_eq!(client.write(b"a").unwrap_err().kind(), io::ErrorKind::WouldBlock);
        println!("-> Write before accept would block");
    }

    assert_eq!(dup(&server, "listen").unwrap_err().kind(), io::ErrorKind::ConnectionReset);
    println!("-> Server can't accept dropped client");

    let mut client = dup(&server, "connect")?;
    nonblock(&client)?;

    assert_eq!(client.write(b"a").unwrap_err().kind(), io::ErrorKind::WouldBlock);
    println!("-> Write before accept would block (alternative connection method)");

    println!("Everything tested!");
    Ok(())
}
