use std::{
    collections::VecDeque,
    fs::File,
    io::{self, prelude::*},
    os::unix::io::AsRawFd
};
use syscall::{flag::*, Event, Packet, SchemeBlockMut, SchemeMut};

mod chan;
mod shm;

use self::chan::ChanScheme;
use self::shm::ShmScheme;

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}

const TOKEN_CHAN: usize = 0;
const TOKEN_SHM: usize = 1;

fn main() -> Result<(), Box<dyn ::std::error::Error>> {
    if unsafe { syscall::clone(0) }.map_err(from_syscall_error)? != 0 {
        return Ok(());
    }

    // Create event listener for both files
    let mut event_file = File::open("event:")?;

    let mut chan_file = File::create(":chan")?;
    event_file.write(&Event {
        id: chan_file.as_raw_fd() as usize,
        flags: EVENT_READ,
        data: TOKEN_CHAN
    })?;

    let mut shm_file = File::create(":shm")?;
    event_file.write(&Event {
        id: shm_file.as_raw_fd() as usize,
        flags: EVENT_READ,
        data: TOKEN_SHM
    })?;

    let mut chan = ChanScheme::default();
    let mut shm = ShmScheme::default();
    let mut todo = VecDeque::with_capacity(16);

    syscall::setrens(0, 0).map_err(from_syscall_error)?;

    loop {
        let mut event = Event::default();
        event_file.read(&mut event)?;

        match event.data {
            TOKEN_CHAN => {
                let mut packet = Packet::default();
                chan_file.read(&mut packet)?;

                // Put new packet first in the queue
                todo.push_front(packet);

                let mut error: Option<io::Error> = None;

                // Process queue, delete finished items
                todo.retain(|packet| {
                    if let Some(status) = chan.handle(&packet) {
                        // Send packet back with new ID
                        let mut packet = *packet;
                        packet.a = status;
                        if let Err(err) = chan_file.write(&packet) {
                            error = Some(err);
                        }
                        return false;
                    }

                    true
                });

                if let Some(err) = error {
                    return Err(Box::new(err));
                }

                // Handle fevents
                chan.post_fevents(&mut chan_file)?;
            },
            TOKEN_SHM => {
                let mut packet = Packet::default();
                shm_file.read(&mut packet)?;

                // Handle packet and update `a` to be status code
                shm.handle(&mut packet);

                shm_file.write(&packet)?;
            },
            _ => ()
        }
    }
}
fn post_fevent(file: &mut File, id: usize, flag: usize) -> io::Result<()> {
    file.write(&syscall::Packet {
        a: syscall::SYS_FEVENT,
        b: id,
        c: flag,
        d: 1,
        ..Default::default()
    })
    .map(|_| ())
}
