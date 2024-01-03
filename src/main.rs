#![feature(int_roundings, let_chains)]

use std::collections::VecDeque;
use event::{EventQueue, EventFlags};
use redox_scheme::{SignalBehavior, Response};
use syscall::{Error, Result, EAGAIN, EWOULDBLOCK, ENODEV};

mod chan;
mod shm;

use self::chan::ChanScheme;
use self::shm::ShmScheme;

fn main() {
    redox_daemon::Daemon::new(move |daemon| {
        // TODO: Better error handling
        match inner(daemon) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                println!("ipcd failed: {error}");
                std::process::exit(1);
            }
        }
    }).expect("ipcd: failed to daemonize");
}

fn inner(daemon: redox_daemon::Daemon) -> Result<()> {
    event::user_data! {
        enum EventSource {
            ChanSocket,
            ShmSocket,
        }
    }
    let chan = ChanScheme::new()?;
    let shm = ShmScheme::new()?;
    daemon.ready().unwrap();

    // Create event listener for both files
    let mut event_queue = EventQueue::<EventSource>::new()?;

    event_queue.subscribe(chan.socket.inner().raw(), EventSource::ChanSocket, EventFlags::READ)?;
    event_queue.subscribe(shm.socket.inner().raw(), EventSource::ShmSocket, EventFlags::READ)?;

    let mut todo = VecDeque::with_capacity(16);

    libredox::call::setrens(0, 0)?;

    let mut chan_opt = Some(chan);
    let mut shm_opt = Some(shm);
    while chan_opt.is_some() || shm_opt.is_some() {
        let Some(event_res) = event_queue.next() else {
            break;
        };
        let event = event_res?;

        match event.user_data {
            EventSource::ChanSocket => {
                let mut error: Option<Error> = None;

                let unmount = if let Some(ref mut chan) = chan_opt {
                    let eof = loop {
                        match chan.socket.next_request(SignalBehavior::Restart) {
                            Ok(None) => break true,
                            Ok(Some(request)) => todo.push_front(Some(request)),
                            Err(Error { errno: EAGAIN | EWOULDBLOCK }) => break false,
                            Err(error) => return Err(error),
                        }
                    };

                    // Process queue, delete finished items
                    todo.retain_mut(|request_opt| {
                        match request_opt.take().unwrap().handle_scheme_block_mut(chan) {
                            Ok(res) => {
                                if let Err(err) = chan.socket.write_response(res, SignalBehavior::Restart) {
                                    error = Some(err);
                                }
                                return false;
                            }
                            Err(req) => {
                                *request_opt = Some(req);
                                true
                            }
                        }
                    });

                    eof
                } else {
                    false
                };

                if unmount && let Some(chan) = chan_opt.take() {
                    for req in todo.drain(..) {
                        if let Err(err) = chan.socket.write_response(Response::new(&req.unwrap(), Err(Error::new(ENODEV))), SignalBehavior::Restart) {
                            error = Some(err);
                        }
                    }
                }

                if let Some(err) = error {
                    return Err(err);
                }
            },
            EventSource::ShmSocket => {
                let unmount = if let Some(ref mut shm) = shm_opt {
                    let eof = loop {
                        match shm.socket.next_request(SignalBehavior::Restart) {
                            Ok(None) => break true,
                            Ok(Some(request)) => {
                                let response = request.handle_scheme_mut(shm);
                                shm.socket.write_response(response, SignalBehavior::Restart)?;
                            },
                            Err(Error { errno: EAGAIN | EWOULDBLOCK }) => break false,
                            Err(err) => return Err(err),
                        };
                    };

                    eof
                } else {
                    false
                };

                if unmount {
                    shm_opt.take();
                }
            }
        }
    }

    Ok(())
}
