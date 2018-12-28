use std::{
    fs::File,
    io::{self, prelude::*},
    os::unix::io::{AsRawFd, FromRawFd}
};

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}

fn main() -> io::Result<()> {
    let server = File::create("chan:hello")?;

    loop {
        let stream = syscall::dup(server.as_raw_fd(), b"listen").map_err(from_syscall_error)?;
        let mut stream = unsafe { File::from_raw_fd(stream) };

        stream.write(b"Hello World!\n")?;
    }
}
