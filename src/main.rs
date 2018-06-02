extern crate syscall;

use std::{
    collections::VecDeque,
    fs::File,
    io::{self, prelude::*}
};
use syscall::SchemeBlockMut;

mod scheme;

use scheme::ChanScheme;

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}

fn main() -> Result<(), Box<::std::error::Error>> {
    if unsafe { syscall::clone(0) }.map_err(from_syscall_error)? != 0 {
        return Ok(());
    }

    let mut scheme_file = File::create(":chan")?;
    let mut scheme = ChanScheme::default();

    let mut todo = VecDeque::with_capacity(16);

    syscall::setrens(0, 0).map_err(from_syscall_error)?;

    loop {
        let mut event = syscall::Packet::default();
        scheme_file.read(&mut event)?;

        // New event has to be handled first so any previous event
        // that is now updated gets processed after.
        todo.push_front(event);

        let mut error = None;

        todo.retain(|event| {
            if let Some(a) = scheme.handle(&event) {
                // Send event back with new ID
                let mut event = *event;
                event.a = a;
                if let Err(err) = scheme_file.write(&event) {
                    error = Some(err);
                }
                return false;
            }

            true
        });

        if let Some(err) = error {
            return Err(Box::new(err));
        }

        scheme.post_fevents(&mut scheme_file)?;
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
