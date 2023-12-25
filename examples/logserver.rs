use std::{
    collections::HashMap,
    fs::File,
    io::{self, prelude::*},
    os::unix::io::{AsRawFd, FromRawFd, RawFd},
    str
};

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}

fn main() -> io::Result<()> {
    let server = File::create("chan:log")?;
    let mut event_file = File::open("event:")?;

    syscall::fcntl(server.as_raw_fd() as usize, syscall::F_SETFL, syscall::O_NONBLOCK)
        .map(|_| ())
        .map_err(from_syscall_error)?;
    event_file.write(&syscall::Event {
        id: server.as_raw_fd() as usize,
        data: 0,
        flags: syscall::EVENT_READ | syscall::EVENT_WRITE,
    })?;

    let mut clients = HashMap::new();
    let mut next_id = 1;

    'outer: loop {
        let mut event = syscall::Event::default();
        event_file.read(&mut event)?;

        if event.data == 0 {
            println!("Listener recevied flags: {:?}", event.flags);
            if event.flags & syscall::EVENT_WRITE == syscall::EVENT_WRITE {
                loop {
                    let stream = match syscall::dup(server.as_raw_fd() as usize, b"listen").map_err(from_syscall_error) {
                        Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                        stream => stream?
                    };
                    let stream = unsafe { File::from_raw_fd(stream as RawFd) };

                    event_file.write(&syscall::Event {
                        id: stream.as_raw_fd() as usize,
                        data: next_id,
                        flags: syscall::EVENT_READ | syscall::EVENT_WRITE,
                    })?;

                    clients.insert(next_id, stream);
                    println!("-> Spawned client #{}", next_id);
                    next_id += 1;
                }
            }
        } else {
            println!("Client #{} received flags: {:?}", event.data, event.flags);
            let client = clients.get_mut(&event.data).unwrap();

            if event.flags & syscall::EVENT_READ == syscall::EVENT_READ {
                println!("-> Reading");
                let mut buf = [0; 128];
                loop {
                    let len = match client.read(&mut buf) {
                        Ok(0) => {
                            println!("--> EOF");
                            clients.remove(&event.data);
                            continue 'outer;
                        },
                        Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                        len => len?
                    };
                    println!("--> Read {}/128 bytes: {:?}", len, str::from_utf8(&buf[..len]));
                }
            }
            if event.flags & syscall::EVENT_WRITE == syscall::EVENT_WRITE {
                println!("-> Writing");
                const BUF: &str = "Hello from the log server\n";
                let mut written = 0;
                while written < BUF.len() {
                    let len = match client.write(BUF[written..].as_bytes()) {
                        Ok(0) => panic!("EOF should never happen here"),
                        Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                        len => len?
                    };
                    println!("--> Wrote {}/{} bytes: {:?}", len, BUF.len(), &BUF[written..]);
                    written += len;
                }
            }
        }
    }
}
