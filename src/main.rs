use std::{
    collections::VecDeque,
    fs::File,
    io::{self, prelude::*},
    os::unix::io::AsRawFd
};
use syscall::{flag::*, error, Error, Event, Packet, SchemeBlockMut, SchemeMut};

mod chan;
mod shm;

use self::chan::ChanScheme;
use self::shm::ShmScheme;

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}

const TOKEN_CHAN: usize = 0;
const TOKEN_SHM: usize = 1;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if unsafe { syscall::clone(CloneFlags::empty()) }.map_err(from_syscall_error)? != 0 {
        return Ok(());
    }

    // Create event listener for both files
    let mut event_file = File::open("event:")?;

    let chan = ChanScheme::new()?;
    event_file.write(&Event {
        id: chan.socket.as_raw_fd() as usize,
        flags: EVENT_READ,
        data: TOKEN_CHAN
    })?;
    let shm = ShmScheme::new()?;
    event_file.write(&Event {
        id: shm.socket.as_raw_fd() as usize,
        flags: EVENT_READ,
        data: TOKEN_SHM
    })?;

    let mut todo = VecDeque::with_capacity(16);

    syscall::setrens(0, 0).map_err(from_syscall_error)?;

    let mut chan_opt = Some(chan);
    let mut shm_opt = Some(shm);
    while chan_opt.is_some() || shm_opt.is_some() {
        let mut event = Event::default();
        event_file.read(&mut event)?;

        match event.data {
            TOKEN_CHAN => {
                let mut error: Option<io::Error> = None;

                let unmount = if let Some(ref mut chan) = chan_opt {
                    let eof = loop {
                        let mut packet = Packet::default();
                        match chan.socket.read(&mut packet) {
                            Ok(0) => break true,
                            Ok(_) => todo.push_front(packet),
                            Err(err) => if err.kind() == io::ErrorKind::WouldBlock {
                                break false;
                            } else {
                                return Err(Box::new(err));
                            }
                        };
                    };

                    // Process queue, delete finished items
                    todo.retain(|packet| {
                        if let Some(status) = chan.handle(&packet) {
                            // Send packet back with new ID
                            let mut packet = *packet;
                            packet.a = status;
                            if let Err(err) = chan.socket.write(&packet) {
                                error = Some(err);
                            }
                            return false;
                        }

                        true
                    });

                    eof
                } else {
                    false
                };

                if unmount {
                    if let Some(mut chan) = chan_opt.take() {
                        for mut packet in todo.drain(..) {
                            packet.a = Error::mux(Err(Error::new(error::ENODEV)));
                            if let Err(err) = chan.socket.write(&packet) {
                                error = Some(err);
                            }
                        }
                    }
                }

                if let Some(err) = error {
                    return Err(Box::new(err));
                }
            },
            TOKEN_SHM => {
                let unmount = if let Some(ref mut shm) = shm_opt {
                    let eof = loop {
                        let mut packet = Packet::default();
                        match shm.socket.read(&mut packet) {
                            Ok(0) => break true,
                            Ok(_) => {
                                shm.handle(&mut packet);
                                shm.socket.write(&packet)?;
                            },
                            Err(err) => if err.kind() == io::ErrorKind::WouldBlock {
                                break false;
                            } else {
                                return Err(Box::new(err));
                            }
                        };
                    };

                    eof
                } else {
                    false
                };

                if unmount {
                    shm_opt.take();
                }
            },
            _ => ()
        }
    }

    Ok(())
}
fn post_fevent(file: &mut File, id: usize, flag: EventFlags) -> syscall::Result<()> {
    file.write(&syscall::Packet {
        a: syscall::SYS_FEVENT,
        b: id,
        c: flag.bits(),
        d: 1,
        ..Default::default()
    })
        .map(|_| ())
        .map_err(|_| Error::new(error::EIO))
}
